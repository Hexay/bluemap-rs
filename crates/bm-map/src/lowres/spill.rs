//! Spilling dirty tiles past the memory budget, and loading them (or stored ones) back.

use bm_format::grid::Tile;
use bm_format::lowres::LowresTile;

use super::{Dirty, LowresLayerError, LowresStore, LowresTileManager, Result, pixel_hash};

impl<S: LowresStore> LowresTileManager<S> {
    /// A tile that is not in memory, as the store has it: spilled pixels, a previous render's, or a new tile.
    pub(super) fn unspill(&mut self, lod: u32, tile: Tile) -> Result<Dirty, S> {
        let stored = self.store.load(lod, tile).map_err(|source| LowresLayerError::Load { lod, tile, source })?;
        self.spilled[lod as usize - 1].remove(&tile);
        let [sx, sz] = self.grid.size;
        let loaded = stored.as_ref().map(pixel_hash);
        let data = stored.unwrap_or_else(|| LowresTile::new([sx as usize, sz as usize]));
        Ok(Dirty { data, loaded, last_write: 0 })
    }

    /// Spills least recently written tiles other than `keep` until the budget holds.
    pub(super) fn enforce_budget(&mut self, keep: (u32, Tile)) -> Result<(), S> {
        while self.layers.iter().map(|l| l.len()).sum::<usize>() > self.budget {
            let oldest = self
                .layers
                .iter()
                .zip(1u32..)
                .flat_map(|(layer, lod)| layer.iter().map(move |(&t, d)| (d.last_write, lod, t)))
                .filter(|&(_, lod, t)| (lod, t) != keep)
                .min();
            let Some((_, lod, tile)) = oldest else { return Ok(()) };
            self.spill(lod, tile)?;
        }
        Ok(())
    }

    fn spill(&mut self, lod: u32, tile: Tile) -> Result<(), S> {
        let i = lod as usize - 1;
        let dirty = self.layers[i].remove(&tile).expect("picked from the layer");
        // an unchanged tile is already in the store as it is
        if !dirty.unchanged()
            && let Err(source) = self.store.spill(lod, tile, &dirty.data)
        {
            self.layers[i].insert(tile, dirty);
            return Err(LowresLayerError::Save { lod, tile, source });
        }
        self.spilled[i].insert(tile);
        Ok(())
    }
}
