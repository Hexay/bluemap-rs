//! The `optimized` format ([`crate::format`]): a compat map storage whose hires grid is replaced by compact blobs
//! ([`bm_format::compact`]) in a [`HiresStore`] (bundles for files, rows for SQL). Hires cells go in and out as
//! raw PRBM (`grid_compression(Hires)` is `None`), so the web layer, `copy_map` and the renderer need no special
//! case: reads decode a blob back to the exact PRBM bytes, writes encode them.

pub(crate) mod bundle;

use std::cell::RefCell;
use std::sync::{Arc, PoisonError, RwLock};

use bm_compress::Compression;
use bm_format::compact::CompactCodec;
use bm_format::grid::{Grid, Tile};

use crate::Result;
use crate::api::{MapStorage, Stored, Version};
use crate::key::{GridKey, ItemKey};
use crate::locks::KeyLocks;

/// Where an optimized map keeps its hires blobs.
pub(crate) trait HiresStore: Send + Sync {
    fn read(&self, tile: Tile) -> Result<Option<Vec<u8>>>;
    fn write(&self, tile: Tile, blob: &[u8]) -> Result<()>;
    fn delete(&self, tile: Tile) -> Result<()>;
    fn exists(&self, tile: Tile) -> Result<bool>;
    /// [`MapStorage::grid_version`] of a hires tile.
    fn version(&self, tile: Tile) -> Result<Option<Version>> {
        let _ = tile;
        Ok(None)
    }
    /// The blob with a version no newer than it (see [`MapStorage::read_grid_versioned`]).
    fn read_versioned(&self, tile: Tile) -> Result<Option<(Vec<u8>, Option<Version>)>> {
        let version = self.version(tile)?;
        Ok(self.read(tile)?.map(|blob| (blob, version)))
    }
    fn list(&self) -> Result<Vec<Tile>>;
    /// Removes every hires tile of the map.
    fn clear(&self) -> Result<()>;
    /// Closes held file handles (before something else deletes the files).
    fn release(&self) {}
}

thread_local! {
    static CODEC: RefCell<(CompactCodec, Vec<u8>)> = RefCell::new((CompactCodec::default(), Vec::new()));
}

/// Keeps the per-thread blob buffer from pinning a huge tile's allocation.
const BLOB_KEEP: usize = 4 << 20;

pub struct OptimizedMapStorage {
    inner: Arc<dyn MapStorage>,
    hires: Box<dyn HiresStore>,
    /// See [`MapStorage::set_hires_grid`].
    grid: RwLock<Option<Grid>>,
}

impl OptimizedMapStorage {
    pub(crate) fn new(inner: Arc<dyn MapStorage>, hires: Box<dyn HiresStore>) -> Self {
        Self { inner, hires, grid: RwLock::default() }
    }

    pub(crate) fn hires(&self) -> &dyn HiresStore {
        self.hires.as_ref()
    }

    /// The blob of a hires tile, as stored.
    pub fn read_hires_blob(&self, tile: Tile) -> Result<Option<Vec<u8>>> {
        self.hires.read(tile)
    }

    /// World block (x, z) of the tile's minimum corner, once the grid is known.
    fn origin(&self, tile: Tile) -> Option<[i32; 2]> {
        let grid = *self.grid.read().unwrap_or_else(PoisonError::into_inner);
        grid.map(|g| g.tile_min(tile).into())
    }
}

impl MapStorage for OptimizedMapStorage {
    fn map_id(&self) -> &str {
        self.inner.map_id()
    }

    fn compression(&self) -> Compression {
        self.inner.compression()
    }

    fn grid_compression(&self, grid: GridKey) -> Compression {
        if grid == GridKey::Hires { Compression::None } else { self.inner.grid_compression(grid) }
    }

    fn read_grid(&self, grid: GridKey, tile: Tile) -> Result<Option<Stored>> {
        if grid != GridKey::Hires {
            return self.inner.read_grid(grid, tile);
        }
        self.hires.read(tile)?.map(|blob| decode(&blob)).transpose()
    }

    fn write_grid_encoded(&self, grid: GridKey, tile: Tile, encoded: &[u8]) -> Result<()> {
        if grid != GridKey::Hires {
            return self.inner.write_grid_encoded(grid, tile, encoded);
        }
        let origin = self.origin(tile);
        CODEC.with(|c| {
            let (codec, blob) = &mut *c.borrow_mut();
            codec.encode_into(encoded, origin, blob)?;
            let written = self.hires.write(tile, blob);
            if blob.capacity() > BLOB_KEEP {
                *blob = Vec::new();
            }
            written
        })
    }

    fn delete_grid(&self, grid: GridKey, tile: Tile) -> Result<()> {
        if grid == GridKey::Hires { self.hires.delete(tile) } else { self.inner.delete_grid(grid, tile) }
    }

    fn grid_exists(&self, grid: GridKey, tile: Tile) -> Result<bool> {
        if grid == GridKey::Hires { self.hires.exists(tile) } else { self.inner.grid_exists(grid, tile) }
    }

    fn list_grid(&self, grid: GridKey) -> Result<Vec<Tile>> {
        if grid == GridKey::Hires { self.hires.list() } else { self.inner.list_grid(grid) }
    }

    fn grids(&self) -> Result<Vec<GridKey>> {
        let mut grids: Vec<GridKey> = self.inner.grids()?.into_iter().filter(|&g| g != GridKey::Hires).collect();
        grids.push(GridKey::Hires);
        grids.sort();
        Ok(grids)
    }

    fn read_item(&self, item: &ItemKey) -> Result<Option<Stored>> {
        self.inner.read_item(item)
    }

    fn write_item_encoded(&self, item: &ItemKey, encoded: &[u8]) -> Result<()> {
        self.inner.write_item_encoded(item, encoded)
    }

    fn delete_item(&self, item: &ItemKey) -> Result<()> {
        self.inner.delete_item(item)
    }

    fn item_exists(&self, item: &ItemKey) -> Result<bool> {
        self.inner.item_exists(item)
    }

    fn list_assets(&self) -> Result<Vec<String>> {
        self.inner.list_assets()
    }

    /// The inner delete removes the hires data too (file: the map directory; SQL: all of the map's grid rows).
    fn delete(&self, on_progress: &mut dyn FnMut(f64) -> bool) -> Result<()> {
        self.hires.release();
        self.inner.delete(on_progress)
    }

    fn exists(&self) -> Result<bool> {
        self.inner.exists()
    }

    fn key_locks(&self) -> &KeyLocks {
        self.inner.key_locks()
    }

    fn set_hires_grid(&self, grid: Grid) {
        *self.grid.write().unwrap_or_else(PoisonError::into_inner) = Some(grid);
    }

    fn grid_version(&self, grid: GridKey, tile: Tile) -> Result<Option<Version>> {
        if grid == GridKey::Hires { self.hires.version(tile) } else { self.inner.grid_version(grid, tile) }
    }

    fn item_version(&self, item: &ItemKey) -> Result<Option<Version>> {
        self.inner.item_version(item)
    }

    fn read_grid_versioned(&self, grid: GridKey, tile: Tile) -> Result<Option<(Stored, Option<Version>)>> {
        if grid != GridKey::Hires {
            return self.inner.read_grid_versioned(grid, tile);
        }
        let Some((blob, version)) = self.hires.read_versioned(tile)? else { return Ok(None) };
        Ok(Some((decode(&blob)?, version)))
    }

    fn read_item_versioned(&self, item: &ItemKey) -> Result<Option<(Stored, Option<Version>)>> {
        self.inner.read_item_versioned(item)
    }
}

#[cfg(test)]
mod tests;

/// A blob back to its PRBM bytes.
fn decode(blob: &[u8]) -> Result<Stored> {
    let mut prbm = Vec::new();
    CODEC.with(|c| c.borrow_mut().0.decode_into(blob, &mut prbm))?;
    Ok(Stored { data: prbm, compression: Compression::None })
}
