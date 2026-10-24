//! Hires tile bundles of an optimized file storage: `<map>/hires/x<bx>z<bz>.bmb`, one append-only log per
//! 16×16 tiles (`bx = x >> 4`, a 512-block region at BlueMap's default 32-block tiles).
//!
//! # File
//! Header (16 B): `"BMB1"`, `u32 4` (log2 tiles per side), `u64 generation` (random per file version). Then
//! records: `u8 kind` (1 put, 2 delete), `u8 lx`, `u8 lz`, `u8 0`, `u32 len`, `u32 xxh32(first 8 header bytes ‖
//! data)`, `len` bytes of compact blob. The last valid record of a tile wins; a delete record removes it.
//!
//! # Why an append log
//! Rewriting a bundle per tile write would copy ~half a bundle (≈1 MB) per tile during a full render (write
//! amplification ~100×) and still create one temp file + rename per tile, the Windows pain point (docs/08).
//! Appending writes each tile once, in place, without creating files. Garbage from re-renders is reclaimed by
//! compaction: when dead bytes exceed live bytes (and 1 MiB), the live records are rewritten to a new file
//! version via the atomic temp + rename of [`crate::file`], amortising to < 2 extra copies per tile.
//!
//! The writing process keeps a write-only handle per recently written bundle (at most [`MAX_WRITERS`]): on
//! Windows, closing a file that was written costs a synchronous antivirus scan (measured 1.5–50 ms per close of
//! a growing bundle, versus ~20 µs per append through a held handle).
//!
//! # Concurrency and crash safety
//! - One writing process per storage (as upstream). Writers of one bundle are serialised by a per-bundle key
//!   lock; the in-memory index is then authoritative and appends never read the file.
//! - A crash mid-append leaves a torn tail: its checksum fails, readers ignore it and still see the tile's
//!   previous record; the next writer truncates it. A crash mid-compaction leaves the old file (rename is atomic).
//! - Readers validate their cached index on every read against the file's generation and length, rescanning
//!   only new records, so a separate webserver process sees the renderer's appends. A handle always reads one
//!   file version, so a concurrent compaction cannot hand a reader a wrong offset.
//!
//! # Sidecar index
//! `x<bx>z<bz>.bmi` (2076 B): `"BMI1"`, `u32 4`, `u64 generation`, `u64 covered` (end of the last indexed record),
//! 256 × (`u32 offset`, `u32 len`) by `lz * 16 + lx` (offset 0 = no tile), `u32 xxh32` of all before. Without it, a
//! reader's first touch of a bundle reads and scans the whole log (~1.9 MB at 300k²); with it, 2 KB plus the
//! records appended after `covered` (testbox HDD, cold: 2× the reads/s, p99 2.5× lower; ~190× fewer bytes).
//! - Written atomically when a write handle closes ([`MapStorage::flush`](crate::MapStorage::flush), drop, writer
//!   eviction, compaction). A writer
//!   deletes it before its first append and never trusts one (it rescans the log), so the record bytes under a
//!   sidecar were valid when it was written and are never rewritten while it exists.
//! - Readers use it only for the header's generation and a file at least `covered` long, and still check each
//!   record they read; a mismatch drops it for that file version and rescans. Older versions ignore the file, and
//!   their compactions change the generation; a record they append lands after `covered` and is scanned.
//! - Not caught: a power loss that shortens the log below `covered`, followed by an older version regrowing it
//!   past `covered` (a sidecar can then hide a tile). A newer writer deletes the sidecar first.
//!
//! # Determinism
//! The live tiles of a bundle are a function of the input; its bytes are not. Records follow write order, and a
//! render writes the tiles of a region from parallel workers in completion order. Where the order is ours to
//! pick, it is fixed: compaction writes live records sorted by local `(x, z)`, and `--convert-storage` writes
//! each bundle's tiles in `(x, z)` order from one thread, so two conversions of the same storage give the same
//! records. The header's generation is random on every new file version and stays so: a content-derived one
//! could repeat after delete + recreate or a compaction back to identical records, and a reader holding the old
//! version's index would then trust offsets into the new file. Compare bundles by their decoded tiles (or by the
//! bytes after the 16-byte header of compacted/converted bundles), never whole files.

mod cache;
mod log;
mod sidecar;
mod writer;

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bm_format::grid::Tile;

use self::cache::BundleCache;
use self::log::{Index, Local, SHIFT};
use super::HiresStore;
use crate::api::Version;
use crate::error::{Error, IoContext, Result};
use crate::file::fsops;
use crate::locks::KeyLocks;

/// Write handles kept open by one store.
pub(crate) const MAX_WRITERS: usize = 64;
const EXT: &str = ".bmb";
const SIDECAR_EXT: &str = ".bmi";

/// One bundle as this process knows it.
#[derive(Default)]
struct Bundle {
    ix: Index,
    /// Write-only handle of the current file version, held between appends.
    writer: Option<File>,
    /// Generation whose sidecar disagreed with the log; that version is indexed by scanning.
    distrust: u64,
}

pub(crate) struct BundleStore {
    dir: PathBuf,
    read_only: bool,
    writers: KeyLocks,
    cache: Mutex<BundleCache>,
    /// Bundles that may hold a writer, oldest first.
    open: Mutex<VecDeque<Tile>>,
}

fn split((x, z): Tile) -> (Tile, Local) {
    ((x >> SHIFT, z >> SHIFT), ((x & 15) as u8, (z & 15) as u8))
}

/// The bundle holding `tile`.
pub(crate) fn bundle_of(tile: Tile) -> Tile {
    split(tile).0
}

fn join((bx, bz): Tile, (lx, lz): Local) -> Tile {
    ((bx << SHIFT) | i32::from(lx), (bz << SHIFT) | i32::from(lz))
}

/// A record never moves within a file version and every put appends a new one, so (generation, offset) names
/// exactly one blob; compaction changes the generation.
fn record_version(ix: &Index, offset: u64, len: u32) -> Version {
    Version([ix.generation, offset, u64::from(len)])
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl BundleStore {
    pub fn new(dir: PathBuf, read_only: bool) -> Self {
        Self { dir, read_only, writers: KeyLocks::default(), cache: Mutex::default(), open: Mutex::default() }
    }

    fn path(&self, (bx, bz): Tile) -> PathBuf {
        self.dir.join(format!("x{bx}z{bz}{EXT}"))
    }

    fn sidecar_path(&self, (bx, bz): Tile) -> PathBuf {
        self.dir.join(format!("x{bx}z{bz}{SIDECAR_EXT}"))
    }

    /// Best effort: without a sidecar, readers scan the log.
    fn write_sidecar(&self, bundle: Tile, ix: &Index) {
        if let Some(bytes) = sidecar::encode(ix) {
            let _ = fsops::write_atomic(&self.sidecar_path(bundle), &bytes);
        }
    }

    fn close_writer(&self, bundle: Tile, b: &mut Bundle) {
        if b.writer.take().is_some() {
            self.write_sidecar(bundle, &b.ix);
        }
    }

    /// Closes every held write handle, leaving each bundle with a sidecar.
    fn close_writers(&self) {
        let open: Vec<Tile> = lock(&self.open).drain(..).collect();
        for bundle in open {
            let state = lock(&self.cache).get(bundle);
            if let Some(state) = state {
                self.close_writer(bundle, &mut lock(&state));
            }
        }
    }

    fn bundle(&self, bundle: Tile) -> Arc<Mutex<Bundle>> {
        lock(&self.cache).get_or_insert(bundle)
    }

    fn open_read(path: &Path) -> io::Result<Option<File>> {
        match fsops::retry(path, || File::open(path)) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            other => other.map(Some),
        }
    }

    /// Brings `ix` up to date with the file behind `f` (another version, or records appended elsewhere). The new
    /// records are read in one sequential read: indexing by record heads alone (positional reads of 12 bytes)
    /// defeats readahead and was 2× slower disk-cold on an HDD (docs/22). A stale index starts from `sidecar` when
    /// given and it describes this file version.
    fn refresh(f: &mut File, ix: &mut Index, sidecar: Option<(&Path, u64)>) -> io::Result<u64> {
        let len = f.metadata()?.len();
        let Some(generation) = log::read_generation(f, len)? else {
            *ix = Index::default();
            return Ok(len);
        };
        if generation != ix.generation || len < ix.end {
            *ix = sidecar
                .filter(|&(_, distrust)| distrust != generation)
                .and_then(|(path, _)| sidecar::load(path, generation, len))
                .unwrap_or_else(|| Index::fresh(generation));
        }
        if len > ix.end {
            log::scan(&log::read_range(f, ix.end, len)?, ix);
        }
        Ok(len)
    }

    /// Runs `op` on the refreshed index of an existing bundle file and returns its result with the read handle the
    /// index was refreshed from. The bundle's lock is held only for the refresh and `op`, so callers read records
    /// in parallel; the handle keeps reading the indexed file version even if it is replaced meanwhile.
    fn with_index<T>(&self, tile: Tile, op: impl FnOnce(&Index, Local) -> T) -> Result<Option<(File, T)>> {
        let (bundle, local) = split(tile);
        let path = self.path(bundle);
        let Some(mut f) = Self::open_read(&path).ctx("read", &path)? else { return Ok(None) };
        let side = self.sidecar_path(bundle);
        let state = self.bundle(bundle);
        let mut b = lock(&state);
        let distrust = b.distrust;
        Self::refresh(&mut f, &mut b.ix, Some((&side, distrust))).ctx("read", &path)?;
        let out = op(&b.ix, local);
        drop(b);
        Ok(Some((f, out)))
    }

    /// The blob of `tile` and its version, reading the record outside the bundle's lock. A record that doesn't
    /// match an index from a sidecar drops the sidecar for that file version and reads again from a scan.
    fn read_record(&self, tile: Tile) -> Result<Option<(Vec<u8>, Version)>> {
        let path = self.path(split(tile).0);
        for _ in 0..2 {
            let found = self.with_index(tile, |ix, local| {
                let entry = ix.entries.get(&local).copied();
                entry.map(|(offset, len)| (local, offset, len, record_version(ix, offset, len), ix.sidecar))
            })?;
            let Some((f, Some((local, offset, len, version, unverified)))) = found else { return Ok(None) };
            if let Some(data) = log::read_record(&f, offset, local, len).ctx("read", &path)? {
                return Ok(Some((data, version)));
            }
            if !unverified {
                break;
            }
            self.distrust_sidecar(split(tile).0, version.0[0]);
        }
        Err(Error::CorruptBundle { path, reason: "record does not match its index entry" })
    }

    fn distrust_sidecar(&self, bundle: Tile, generation: u64) {
        let state = self.bundle(bundle);
        let mut b = lock(&state);
        b.distrust = generation;
        if b.ix.sidecar && b.ix.generation == generation {
            b.ix = Index::default();
        }
    }

    fn bundles(&self) -> Result<Vec<Tile>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).ctx("list", &self.dir),
        };
        let mut out = Vec::new();
        for entry in entries {
            let name = entry.ctx("list", &self.dir)?.file_name();
            let parsed = name.to_str().and_then(|n| n.strip_prefix('x')?.strip_suffix(EXT)?.split_once('z'));
            if let Some((x, z)) = parsed
                && let (Ok(x), Ok(z)) = (x.parse(), z.parse())
            {
                out.push((x, z));
            }
        }
        Ok(out)
    }
}

impl HiresStore for BundleStore {
    fn read(&self, tile: Tile) -> Result<Option<Vec<u8>>> {
        Ok(self.read_versioned(tile)?.map(|(data, _)| data))
    }

    fn read_versioned(&self, tile: Tile) -> Result<Option<(Vec<u8>, Option<Version>)>> {
        Ok(self.read_record(tile)?.map(|(data, version)| (data, Some(version))))
    }

    fn write(&self, tile: Tile, blob: &[u8]) -> Result<()> {
        self.append(tile, Some(blob))
    }

    fn delete(&self, tile: Tile) -> Result<()> {
        self.append(tile, None)
    }

    fn exists(&self, tile: Tile) -> Result<bool> {
        Ok(self.with_index(tile, |ix, local| ix.entries.contains_key(&local))?.is_some_and(|(_, hit)| hit))
    }

    fn version(&self, tile: Tile) -> Result<Option<Version>> {
        let found = self.with_index(tile, |ix, local| {
            ix.entries.get(&local).map(|&(offset, len)| record_version(ix, offset, len))
        })?;
        Ok(found.and_then(|(_, v)| v))
    }

    fn list(&self) -> Result<Vec<Tile>> {
        let mut tiles = Vec::new();
        for bundle in self.bundles()? {
            let locals = self.with_index(join(bundle, (0, 0)), |ix, _| ix.entries.keys().copied().collect());
            tiles.extend(locals?.map_or_else(Vec::new, |(_, l)| l).into_iter().map(|l: Local| join(bundle, l)));
        }
        Ok(tiles)
    }

    fn clear(&self) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        self.release();
        fsops::remove_tree(&self.dir)
    }

    fn flush(&self) {
        self.close_writers();
    }

    fn release(&self) {
        let states = lock(&self.cache).drain();
        lock(&self.open).clear();
        for state in states {
            lock(&state).writer = None;
        }
    }
}

impl Drop for BundleStore {
    fn drop(&mut self) {
        self.close_writers();
    }
}

#[cfg(test)]
mod tests;
