use std::convert::Infallible;

use bm_format::grid::Tile;
use bm_format::lowres::LowresTile;
use rustc_hash::FxHashMap;

/// Where lowres tiles come from and go to. `lod` is 1-based, as in `tiles/<lod>/`.
pub trait LowresStore {
    type Error: std::error::Error + 'static;

    /// The stored tile, or `None` if there is none yet (it then starts transparent).
    fn load(&mut self, lod: u32, tile: Tile) -> Result<Option<LowresTile>, Self::Error>;

    /// Persists a finished tile. Called once per dirty tile per flush, never concurrently for one tile.
    fn save(&mut self, lod: u32, tile: Tile, data: &LowresTile) -> Result<(), Self::Error>;

    /// [`Self::save`] for several distinct tiles of one LOD, results in input order; stores may save them in
    /// parallel.
    fn save_all(&mut self, lod: u32, tiles: &[(Tile, &LowresTile)]) -> Vec<Result<(), Self::Error>> {
        tiles.iter().map(|&(tile, data)| self.save(lod, tile, data)).collect()
    }

    /// Called instead of [`Self::save`] when a flushed tile's pixels equal the ones loaded from the store.
    fn unchanged(&mut self, _lod: u32, _tile: Tile) {}
}

/// In-memory store, for tests and for callers that encode tiles elsewhere.
#[derive(Default)]
pub struct MemoryStore {
    pub tiles: FxHashMap<(u32, Tile), LowresTile>,
    /// Number of `save` calls so far.
    pub saves: usize,
}

impl LowresStore for MemoryStore {
    type Error = Infallible;

    fn load(&mut self, lod: u32, tile: Tile) -> Result<Option<LowresTile>, Infallible> {
        Ok(self.tiles.get(&(lod, tile)).cloned())
    }

    fn save(&mut self, lod: u32, tile: Tile, data: &LowresTile) -> Result<(), Infallible> {
        self.saves += 1;
        self.tiles.insert((lod, tile), data.clone());
        Ok(())
    }
}
