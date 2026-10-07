//! Hires tile bundles of an optimized file storage: `<map>/hires/x<bx>z<bz>.bmb`, one append-only log per
//! 16×16 tiles (`bx = x >> 4`, a 512-block region at BlueMap's default 32-block tiles).
//!
//! # File
//! Header (16 B): `"BMB1"`, `u32 4` (log2 tiles per side), `u64 generation` (random per file version). Then
//! records: `u8 kind` (1 put, 2 delete), `u8 lx`, `u8 lz`, `u8 0`, `u32 len`, `u32 xxh32(first 8 header bytes ‖
//! data)`, `len` bytes of BMQ2 blob. The last valid record of a tile wins; a delete record removes it.
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

mod log;
mod writer;

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bm_format::grid::Tile;

use self::log::{Index, Local, SHIFT};
use super::HiresStore;
use crate::api::Version;
use crate::error::{Error, IoContext, Result};
use crate::file::fsops;
use crate::locks::KeyLocks;

/// Cached bundle states before the cache is dropped (each index holds ≤ 256 entries).
const CACHE_CAP: usize = 4096;
/// Write handles kept open by one store.
pub(crate) const MAX_WRITERS: usize = 64;
const EXT: &str = ".bmb";

/// One bundle as this process knows it.
#[derive(Default)]
struct Bundle {
    ix: Index,
    /// Write-only handle of the current file version, held between appends.
    writer: Option<File>,
}

pub(crate) struct BundleStore {
    dir: PathBuf,
    read_only: bool,
    writers: KeyLocks,
    cache: Mutex<HashMap<Tile, Arc<Mutex<Bundle>>>>,
    /// Bundles that may hold a writer, oldest first.
    open: Mutex<VecDeque<Tile>>,
}

fn split((x, z): Tile) -> (Tile, Local) {
    ((x >> SHIFT, z >> SHIFT), ((x & 15) as u8, (z & 15) as u8))
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

    fn bundle(&self, bundle: Tile) -> Arc<Mutex<Bundle>> {
        let mut cache = lock(&self.cache);
        if cache.len() >= CACHE_CAP && !cache.contains_key(&bundle) {
            cache.clear();
        }
        cache.entry(bundle).or_default().clone()
    }

    fn open_read(path: &Path) -> io::Result<Option<File>> {
        match fsops::retry(path, || File::open(path)) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            other => other.map(Some),
        }
    }

    /// Brings `ix` up to date with the file behind `f` (another version, or records appended elsewhere).
    fn refresh(f: &mut File, ix: &mut Index) -> io::Result<u64> {
        let len = f.metadata()?.len();
        let Some(generation) = log::read_generation(f, len)? else {
            *ix = Index::default();
            return Ok(len);
        };
        if generation != ix.generation || len < ix.end {
            *ix = Index::fresh(generation);
        }
        if len > ix.end {
            log::scan(&log::read_range(f, ix.end, len)?, ix);
        }
        Ok(len)
    }

    /// Runs `op` on a read handle and the refreshed index of an existing bundle file.
    fn with_index<T>(&self, tile: Tile, op: impl FnOnce(&mut File, &Index, Local) -> io::Result<T>) -> Result<Option<T>> {
        let (bundle, local) = split(tile);
        let path = self.path(bundle);
        let Some(mut f) = Self::open_read(&path).ctx("read", &path)? else { return Ok(None) };
        let state = self.bundle(bundle);
        let mut b = lock(&state);
        Self::refresh(&mut f, &mut b.ix).ctx("read", &path)?;
        op(&mut f, &b.ix, local).map(Some).ctx("read", &path)
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
        let found = self.with_index(tile, |f, ix, local| match ix.entries.get(&local) {
            Some(&(offset, len)) => {
                let version = record_version(ix, offset, len);
                log::read_record(f, offset, local, len).map(|r| Some(r.map(|d| (d, Some(version))).ok_or(())))
            }
            None => Ok(None),
        })?;
        match found.flatten() {
            None => Ok(None),
            Some(Ok(read)) => Ok(Some(read)),
            Some(Err(())) => Err(Error::CorruptBundle {
                path: self.path(split(tile).0),
                reason: "record does not match its index entry",
            }),
        }
    }

    fn write(&self, tile: Tile, blob: &[u8]) -> Result<()> {
        self.append(tile, Some(blob))
    }

    fn delete(&self, tile: Tile) -> Result<()> {
        self.append(tile, None)
    }

    fn exists(&self, tile: Tile) -> Result<bool> {
        Ok(self.with_index(tile, |_, ix, local| Ok(ix.entries.contains_key(&local)))?.unwrap_or(false))
    }

    fn version(&self, tile: Tile) -> Result<Option<Version>> {
        let found = self.with_index(tile, |_, ix, local| {
            Ok(ix.entries.get(&local).map(|&(offset, len)| record_version(ix, offset, len)))
        })?;
        Ok(found.flatten())
    }

    fn list(&self) -> Result<Vec<Tile>> {
        let mut tiles = Vec::new();
        for bundle in self.bundles()? {
            let locals = self.with_index(join(bundle, (0, 0)), |_, ix, _| Ok(ix.entries.keys().copied().collect()))?;
            tiles.extend(locals.unwrap_or_else(Vec::new).into_iter().map(|l: Local| join(bundle, l)));
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

    fn release(&self) {
        let states: Vec<_> = lock(&self.cache).drain().map(|(_, s)| s).collect();
        lock(&self.open).clear();
        for state in states {
            lock(&state).writer = None;
        }
    }
}

#[cfg(test)]
mod tests;
