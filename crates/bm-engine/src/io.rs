//! bm-map's storage-agnostic traits over a [`MapStorage`]: render-state cells and lowres tiles.

use std::io;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use bm_format::grid::Tile;
use bm_format::lowres::{LowresError, LowresTile};
use bm_map::lowres::LowresStore;
use bm_map::renderstate::{CellIo, CellKind};
use bm_storage::{GridKey, MapStorage};

use crate::persist::Msg;

pub(crate) fn grid_key(kind: CellKind) -> GridKey {
    match kind {
        CellKind::Tiles => GridKey::TileState,
        CellKind::Chunks => GridKey::ChunkState,
        CellKind::Regions => GridKey::RegionState,
    }
}

fn io_err(e: bm_storage::Error) -> io::Error {
    io::Error::other(e)
}

/// Reads go straight to storage; writes are queued to the persistence thread so saving never blocks the caller.
pub(crate) struct QueuedCells {
    pub storage: Arc<dyn MapStorage>,
    pub queue: SyncSender<Msg>,
}

impl CellIo for QueuedCells {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>> {
        Ok(self.storage.read_grid(grid_key(kind), cell).map_err(io_err)?.map(|s| s.data))
    }

    fn write_cell(&self, kind: CellKind, cell: Tile, bytes: &[u8]) -> io::Result<()> {
        let msg = Msg::WriteCell { grid: grid_key(kind), cell, bytes: bytes.to_vec() };
        self.queue.send(msg).map_err(|_| io::Error::other("the persistence thread has stopped"))
    }

    fn delete_cell(&self, kind: CellKind, cell: Tile) -> io::Result<()> {
        self.storage.delete_grid(grid_key(kind), cell).map_err(io_err)
    }

    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>> {
        self.storage.list_grid(grid_key(kind)).map_err(io_err)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LowresStoreError {
    #[error(transparent)]
    Storage(#[from] bm_storage::Error),
    #[error(transparent)]
    Png(#[from] LowresError),
}

/// Lowres PNGs in a map storage, never compressed (`GridKey::Lowres`).
pub(crate) struct StorageLowres {
    pub storage: Arc<dyn MapStorage>,
    pub tile_size: [usize; 2],
    pub png: Vec<u8>,
    pub saves: usize,
}

impl LowresStore for StorageLowres {
    type Error = LowresStoreError;

    fn load(&mut self, lod: u32, tile: Tile) -> Result<Option<LowresTile>, LowresStoreError> {
        let Some(stored) = self.storage.read_grid(GridKey::Lowres(lod), tile)? else { return Ok(None) };
        // like `LowresLayer`, an unreadable tile starts over empty instead of failing the render
        Ok(LowresTile::decode_png(&stored.decompress()?, self.tile_size).ok())
    }

    fn save(&mut self, lod: u32, tile: Tile, data: &LowresTile) -> Result<(), LowresStoreError> {
        data.encode_png(&mut self.png)?;
        self.storage.write_grid(GridKey::Lowres(lod), tile, &self.png)?;
        self.saves += 1;
        Ok(())
    }
}
