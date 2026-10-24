//! Lowres layers (`LowresTileManager`, `LowresLayer`): per-column map metadata at LOD 1 and its downsampled LODs.
//!
//! One owner (`&mut self`) holds every dirty tile, so each tile has exactly one writer and its saves are sequential
//! (no #821-style concurrent PNG writes). Java's `MAX_PENDING`/timed saves become explicit flushes: a tile's next-LOD
//! pixels depend only on its final content, so one flush yields the pixels Java converges to, with one encode per
//! tile instead of Java's repeated re-saves (docs/10-perf-audit-render.md §2).
//!
//! A loaded tile whose pixels end up as they were is not encoded and written again ([`LowresStore::unchanged`]
//! instead of `save`); it still cascades, so the next LOD converges as Java's would.
//!
//! Memory is bounded by [`LowresTileManager::with_budget`]: past it, the least recently written tile is spilled
//! (saved without cascading) and reloaded by its next write or flush. The PNG round trip is lossless and cascades
//! only ever run on a final tile, so spilling changes no output; it keeps an update's open tiles — a front that
//! grows with map extent — from growing memory (docs/22).

mod downsample;
mod spill;
mod store;
#[cfg(test)]
mod tests;

use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};

use bm_format::grid::{Grid, Tile};
use bm_format::lowres::LowresTile;
use bm_math::Color;
use rustc_hash::{FxHashMap, FxHashSet};
use twox_hash::XxHash3_64;

use downsample::downsample;
pub use store::{LowresStore, MemoryStore};

#[derive(Debug, thiserror::Error)]
pub enum LowresLayerError<E: std::error::Error + 'static> {
    #[error("failed to load lowres tile {tile:?} (lod {lod})")]
    Load { lod: u32, tile: Tile, source: E },
    #[error("failed to save lowres tile {tile:?} (lod {lod})")]
    Save { lod: u32, tile: Tile, source: E },
}

type Result<T, S> = std::result::Result<T, LowresLayerError<<S as LowresStore>::Error>>;

pub struct LowresTileManager<S: LowresStore> {
    grid: Grid,
    lod_factor: i32,
    /// Dirty tiles per LOD, index `lod - 1`.
    layers: Vec<FxHashMap<Tile, Dirty>>,
    /// Dirty tiles whose pixels were spilled to the store, per LOD; disjoint from `layers`.
    spilled: Vec<FxHashSet<Tile>>,
    /// Most dirty tiles held in memory.
    budget: usize,
    /// Write counter, for least-recently-written eviction.
    clock: u64,
    store: S,
}

struct Dirty {
    data: LowresTile,
    /// Hash of the pixels as loaded; `None` for a tile the store didn't have.
    loaded: Option<u64>,
    last_write: u64,
}

impl Dirty {
    fn unchanged(&self) -> bool {
        self.loaded == Some(pixel_hash(&self.data))
    }
}

fn pixel_hash(tile: &LowresTile) -> u64 {
    let mut h = XxHash3_64::default();
    tile.hash(&mut h);
    h.finish()
}

impl<S: LowresStore> LowresTileManager<S> {
    /// Settings as in `settings.json` `lowres`: `tileSize`, `lodCount`, `lodFactor`.
    ///
    /// # Panics
    /// If any setting is not positive.
    pub fn new(store: S, tile_size: [i32; 2], lod_count: u32, lod_factor: i32) -> Self {
        assert!(tile_size.iter().all(|&s| s > 0) && lod_count > 0 && lod_factor > 0, "invalid lowres settings");
        Self {
            grid: Grid { size: tile_size, offset: [0, 0] },
            lod_factor,
            layers: (0..lod_count).map(|_| FxHashMap::default()).collect(),
            spilled: (0..lod_count).map(|_| FxHashSet::default()).collect(),
            budget: usize::MAX,
            clock: 0,
            store,
        }
    }

    /// Holds at most `tiles` dirty tiles (at least 1) in memory, spilling the rest to the store.
    #[must_use]
    pub fn with_budget(mut self, tiles: usize) -> Self {
        self.budget = tiles.max(1);
        self
    }

    pub fn grid(&self) -> Grid {
        self.grid
    }

    pub fn lod_count(&self) -> u32 {
        self.layers.len() as u32
    }

    pub fn lod_factor(&self) -> i32 {
        self.lod_factor
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn into_store(self) -> S {
        self.store
    }

    /// Dirty tiles of `lod` held in memory, in no particular order.
    pub fn dirty_tiles(&self, lod: u32) -> impl Iterator<Item = Tile> + '_ {
        self.layers[lod as usize - 1].keys().copied()
    }

    /// Dirty tiles of `lod` spilled to the store, in no particular order.
    pub fn spilled_tiles(&self, lod: u32) -> impl Iterator<Item = Tile> + '_ {
        self.spilled[lod as usize - 1].iter().copied()
    }

    /// `TileMetaConsumer.set`: one block column in world coordinates. `color` may be premultiplied; it is stored
    /// straight and truncated. `height` keeps its low 16 bits, `block_light` its low 8.
    pub fn set(&mut self, x: i32, z: i32, mut color: Color, height: i32, block_light: i32) -> Result<(), S> {
        self.set_argb(x, z, color.straight().get_int() as u32, height, block_light)
    }

    /// [`Self::set`] with the colour already as stored: straight ARGB.
    pub fn set_argb(&mut self, x: i32, z: i32, argb: u32, height: i32, block_light: i32) -> Result<(), S> {
        let tile = self.grid.tile_of(x, z);
        let (min_x, min_z) = self.grid.tile_min(tile);
        self.set_pixel(1, tile, [x - min_x, z - min_z], argb, height, block_light as u8)
    }

    /// Saves every dirty tile, LOD 1 first, cascading each into the next LOD, until nothing is dirty.
    pub fn flush(&mut self) -> Result<(), S> {
        for lod in 1..=self.lod_count() {
            self.flush_lod(lod, |_| true)?;
        }
        Ok(())
    }

    /// Saves the dirty tiles of `lod` that `select` picks, in tile order, and writes each one's downsampled core into
    /// `lod + 1`, where it stays dirty. Saved tiles leave memory; a later write reloads them from the store.
    /// A tile whose save or cascade fails stays dirty, so a later flush retries it; the first failure is returned.
    pub fn flush_lod(&mut self, lod: u32, mut select: impl FnMut(Tile) -> bool) -> Result<(), S> {
        let i = lod as usize - 1;
        let mut tiles: Vec<Tile> =
            self.layers[i].keys().chain(&self.spilled[i]).copied().filter(|&t| select(t)).collect();
        tiles.sort_unstable();
        // spilled tiles are reloaded to flush, so a batch's worth at a time keeps within budget
        let batch = if self.budget == usize::MAX { tiles.len().max(1) } else { self.budget.div_ceil(2) };
        let mut first_err = None;
        for chunk in tiles.chunks(batch) {
            if let Err(e) = self.flush_batch(lod, chunk) {
                first_err.get_or_insert(e);
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    fn flush_batch(&mut self, lod: u32, tiles: &[Tile]) -> Result<(), S> {
        let mut first_err = None;
        let mut dirty: Vec<(Tile, Dirty, bool)> = Vec::with_capacity(tiles.len());
        for &t in tiles {
            let d = match self.layers[lod as usize - 1].remove(&t) {
                Some(d) => d,
                None => match self.unspill(lod, t) {
                    Ok(d) => d,
                    Err(e) => {
                        first_err.get_or_insert(e);
                        continue;
                    }
                },
            };
            let unchanged = d.unchanged();
            dirty.push((t, d, unchanged));
        }
        let changed: Vec<(Tile, &LowresTile)> =
            dirty.iter().filter(|(.., unchanged)| !unchanged).map(|(t, d, _)| (*t, &d.data)).collect();
        let mut saved = self.store.save_all(lod, &changed).into_iter();
        for (tile, d, unchanged) in dirty {
            let result = if unchanged {
                self.store.unchanged(lod, tile);
                Ok(())
            } else {
                saved.next().expect("one result per changed tile").map_err(|source| LowresLayerError::Save {
                    lod,
                    tile,
                    source,
                })
            };
            if let Err(e) = result.and_then(|()| self.cascade(lod, tile, &d.data)) {
                self.layers[lod as usize - 1].insert(tile, d);
                first_err.get_or_insert(e);
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Drops every unsaved change (`LowresTileManager.discard`, used by map purges). Spilled pixels already in the
    /// store stay there.
    pub fn discard(&mut self) {
        self.layers.iter_mut().for_each(FxHashMap::clear);
        self.spilled.iter_mut().for_each(FxHashSet::clear);
    }

    /// Writes a saved tile's downsampled core into `lod + 1`.
    fn cascade(&mut self, lod: u32, tile: Tile, data: &LowresTile) -> Result<(), S> {
        if lod == self.lod_count() {
            return Ok(());
        }
        let (f, size) = (self.lod_factor, self.grid.size);
        let next = (tile.0.div_euclid(f), tile.1.div_euclid(f));
        let base = [tile.0.rem_euclid(f) * size[0].div_euclid(f), tile.1.rem_euclid(f) * size[1].div_euclid(f)];
        downsample(data, size, f, |gx, gz, argb, height, light| {
            self.set_pixel(lod + 1, next, [base[0] + gx, base[1] + gz], argb, height, light as u8)
        })
    }

    /// `LowresLayer.set`: a pixel on row/column 0 is copied into the extra row/column of the neighbour tiles.
    fn set_pixel(&mut self, lod: u32, (tx, tz): Tile, [px, pz]: [i32; 2], argb: u32, h: i32, l: u8) -> Result<(), S> {
        let [sx, sz] = self.grid.size;
        self.write(lod, (tx, tz), px, pz, argb, h, l)?;
        if px == 0 {
            self.write(lod, (tx - 1, tz), sx, pz, argb, h, l)?;
        }
        if pz == 0 {
            self.write(lod, (tx, tz - 1), px, sz, argb, h, l)?;
        }
        if px == 0 && pz == 0 {
            self.write(lod, (tx - 1, tz - 1), sx, sz, argb, h, l)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write(&mut self, lod: u32, tile: Tile, px: i32, pz: i32, argb: u32, h: i32, l: u8) -> Result<(), S> {
        self.clock += 1;
        let i = lod as usize - 1;
        let added = match self.layers[i].entry(tile) {
            Entry::Occupied(e) => {
                let dirty = e.into_mut();
                dirty.last_write = self.clock;
                dirty.data.set(px as usize, pz as usize, argb, h, l);
                false
            }
            Entry::Vacant(_) => true,
        };
        if added {
            let mut dirty = self.unspill(lod, tile)?;
            dirty.last_write = self.clock;
            dirty.data.set(px as usize, pz as usize, argb, h, l);
            self.layers[i].insert(tile, dirty);
            self.enforce_budget((lod, tile))?;
        }
        Ok(())
    }
}
