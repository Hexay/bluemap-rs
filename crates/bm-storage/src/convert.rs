//! In-place conversion of a whole storage between `compat` and `optimized`. Only hires tiles differ between the
//! formats, so only they are rewritten; lowres, render state and items stay byte-identical.
//!
//! Crash safety: target tiles are written (and read back and compared) first, then the marker flips, then the
//! source tiles go. Until the flip the storage is still the old format with garbage beside it; after it, the new
//! one. Re-running finishes or redoes the job. Nothing may use the storage meanwhile (stop BlueMap first).

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bm_compress::Compression;
use bm_format::grid::Tile;
use tokio::runtime::Handle;

use crate::api::{MapStorage, Storage};
use crate::error::{Error, Result};
use crate::file::{self, FileStorage, fsops, layout};
use crate::format::Format;
use crate::key::GridKey;
use crate::optimized::OptimizedMapStorage;
use crate::sql::{SqlConfig, SqlStorage};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConvertStats {
    pub maps: usize,
    pub tiles: usize,
    /// The storage was already in the target format (leftovers of an interrupted run were removed).
    pub already: bool,
}

/// Called with (map id, tiles done, tiles total) as tiles convert, from worker threads.
pub type Progress<'a> = &'a (dyn Fn(&str, usize, usize) + Sync);

/// Both views of one physical storage.
trait Physical {
    fn detect(&self) -> Result<Option<Format>>;
    fn set_marker(&self, optimized: bool) -> Result<()>;
    fn map_ids(&self) -> Result<Vec<String>>;
    fn compat(&self, id: &str) -> Result<Arc<dyn MapStorage>>;
    fn optimized(&self, id: &str) -> Result<Arc<OptimizedMapStorage>>;
    fn clear_compat_hires(&self, id: &str) -> Result<()>;
}

impl Physical for FileStorage {
    fn detect(&self) -> Result<Option<Format>> {
        file::detect_format(self.root())
    }
    fn set_marker(&self, optimized: bool) -> Result<()> {
        file::set_marker(self.root(), optimized)
    }
    fn map_ids(&self) -> Result<Vec<String>> {
        Storage::map_ids(self)
    }
    fn compat(&self, id: &str) -> Result<Arc<dyn MapStorage>> {
        Ok(self.file_map(id)?)
    }
    fn optimized(&self, id: &str) -> Result<Arc<OptimizedMapStorage>> {
        self.optimized_map(id)
    }
    fn clear_compat_hires(&self, id: &str) -> Result<()> {
        fsops::remove_tree(&self.file_map(id)?.root().join(layout::TILES).join("0"))
    }
}

impl Physical for SqlStorage {
    fn detect(&self) -> Result<Option<Format>> {
        self.detect_format()
    }
    fn set_marker(&self, optimized: bool) -> Result<()> {
        SqlStorage::set_marker(self, optimized)
    }
    fn map_ids(&self) -> Result<Vec<String>> {
        Storage::map_ids(self)
    }
    fn compat(&self, id: &str) -> Result<Arc<dyn MapStorage>> {
        Ok(self.sql_map(id))
    }
    fn optimized(&self, id: &str) -> Result<Arc<OptimizedMapStorage>> {
        Ok(self.optimized_map(id))
    }
    fn clear_compat_hires(&self, id: &str) -> Result<()> {
        self.sql_map(id).delete_cells(&GridKey::Hires.sql_key())
    }
}

/// Converts the file storage at `root` (hires compat tiles use `compression`) to `to`.
pub fn convert_file_storage(root: &Path, compression: Compression, to: Format, progress: Progress) -> Result<ConvertStats> {
    convert(&FileStorage::new(root, compression), to, progress)
}

/// Converts the SQL storage of `config` (its `format` is ignored) to `to`.
pub fn convert_sql_storage(config: &SqlConfig, runtime: Handle, to: Format, progress: Progress) -> Result<ConvertStats> {
    let storage = SqlStorage::connect_unchecked(config, runtime)?;
    let result = convert(&storage, to, progress);
    storage.close();
    result
}

fn convert(p: &dyn Physical, to: Format, progress: Progress) -> Result<ConvertStats> {
    let optimized = to == Format::Optimized;
    let ids = p.map_ids()?;
    let mut stats = ConvertStats { maps: ids.len(), ..ConvertStats::default() };
    match p.detect()? {
        None => return p.set_marker(optimized).map(|()| ConvertStats { already: true, ..stats }),
        Some(found) if found == to => {
            remove_source(p, &ids, optimized)?;
            return Ok(ConvertStats { already: true, ..stats });
        }
        Some(_) => {}
    }
    for id in &ids {
        let (compat, opt) = (p.compat(id)?, p.optimized(id)?);
        let (src, dst): (&dyn MapStorage, &dyn MapStorage) = if optimized {
            opt.hires().clear()?;
            (compat.as_ref(), opt.as_ref())
        } else {
            p.clear_compat_hires(id)?;
            (opt.as_ref(), compat.as_ref())
        };
        stats.tiles += copy_hires(id, src, dst, progress)?;
    }
    p.set_marker(optimized)?;
    remove_source(p, &ids, optimized)?;
    Ok(stats)
}

/// Removes the hires tiles of the format not being converted to.
fn remove_source(p: &dyn Physical, ids: &[String], optimized: bool) -> Result<()> {
    for id in ids {
        if optimized { p.clear_compat_hires(id)? } else { p.optimized(id)?.hires().clear()? }
    }
    Ok(())
}

/// Copies every hires tile in parallel, reading each back to compare the raw PRBM.
fn copy_hires(id: &str, src: &dyn MapStorage, dst: &dyn MapStorage, progress: Progress) -> Result<usize> {
    let tiles = src.list_grid(GridKey::Hires)?;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let error = Mutex::new(None);
    let work = |tile: Tile| -> Result<()> {
        let Some(stored) = src.read_grid(GridKey::Hires, tile)? else { return Ok(()) };
        let raw = stored.decompress()?;
        dst.write_grid(GridKey::Hires, tile, &raw)?;
        let back = dst.read_grid(GridKey::Hires, tile)?.ok_or(Error::Protocol("converted tile vanished"))?;
        if back.decompress()? != raw {
            return Err(Error::Protocol("converted tile does not read back identically"));
        }
        Ok(())
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(tiles.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while !failed.load(Ordering::Relaxed) {
                    let Some(&tile) = tiles.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
                    if let Err(e) = work(tile) {
                        failed.store(true, Ordering::Relaxed);
                        error.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(e);
                        break;
                    }
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if n.is_multiple_of(256) || n == tiles.len() {
                        progress(id, n, tiles.len());
                    }
                }
            });
        }
    });
    match error.into_inner().unwrap_or_else(PoisonError::into_inner) {
        Some(e) => Err(e),
        None => Ok(tiles.len()),
    }
}
