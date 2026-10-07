//! Lowres layers (`LowresTileManager`, `LowresLayer`): per-column map metadata at LOD 1 and its downsampled LODs.
//!
//! One owner (`&mut self`) holds every dirty tile, so each tile has exactly one writer and its saves are sequential
//! (no #821-style concurrent PNG writes). Java's `MAX_PENDING`/timed saves become explicit flushes: a tile's next-LOD
//! pixels depend only on its final content, so one flush yields the pixels Java converges to, with one encode per
//! tile instead of Java's repeated re-saves (docs/10-perf-audit-render.md §2).
//!
//! A loaded tile whose pixels end up as they were is not encoded and written again ([`LowresStore::unchanged`]
//! instead of `save`); it still cascades, so the next LOD converges as Java's would.

mod downsample;
mod store;
#[cfg(test)]
mod tests;

use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};

use bm_format::grid::{Grid, Tile};
use bm_format::lowres::LowresTile;
use bm_math::Color;
use rustc_hash::FxHashMap;
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
    store: S,
}

struct Dirty {
    data: LowresTile,
    /// Hash of the pixels as loaded; `None` for a tile the store didn't have.
    loaded: Option<u64>,
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
            store,
        }
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

    /// Dirty (unsaved) tiles of `lod`, in no particular order.
    pub fn dirty_tiles(&self, lod: u32) -> impl Iterator<Item = Tile> + '_ {
        self.layers[lod as usize - 1].keys().copied()
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
    /// A tile whose save or cascade fails stays dirty, so a later flush retries it.
    pub fn flush_lod(&mut self, lod: u32, mut select: impl FnMut(Tile) -> bool) -> Result<(), S> {
        let mut tiles: Vec<Tile> = self.dirty_tiles(lod).filter(|&t| select(t)).collect();
        tiles.sort_unstable();
        for tile in tiles {
            let dirty = self.layers[lod as usize - 1].remove(&tile).expect("listed as dirty");
            if let Err(e) = self.save_and_cascade(lod, tile, &dirty) {
                self.layers[lod as usize - 1].insert(tile, dirty);
                return Err(e);
            }
        }
        Ok(())
    }

    /// Drops every unsaved change (`LowresTileManager.discard`, used by map purges).
    pub fn discard(&mut self) {
        self.layers.iter_mut().for_each(FxHashMap::clear);
    }

    fn save_and_cascade(&mut self, lod: u32, tile: Tile, Dirty { data, loaded }: &Dirty) -> Result<(), S> {
        if *loaded == Some(pixel_hash(data)) {
            self.store.unchanged(lod, tile);
        } else {
            self.store.save(lod, tile, data).map_err(|source| LowresLayerError::Save { lod, tile, source })?;
        }
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
        let dirty = match self.layers[lod as usize - 1].entry(tile) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let stored = self.store.load(lod, tile).map_err(|source| LowresLayerError::Load { lod, tile, source })?;
                let [sx, sz] = self.grid.size;
                let loaded = stored.as_ref().map(pixel_hash);
                e.insert(Dirty { data: stored.unwrap_or_else(|| LowresTile::new([sx as usize, sz as usize])), loaded })
            }
        };
        dirty.data.set(px as usize, pz as usize, argb, h, l);
        Ok(())
    }
}
