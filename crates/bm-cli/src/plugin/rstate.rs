//! Read-only views of a map's render state for commands (`getMapTileState`, `getMapChunkState`). They read the
//! stored cells, so they lag the render thread by at most one save.

use std::io;
use std::sync::Arc;

use bm_format::grid::Tile;
use bm_map::renderstate::{CellIo, CellKind, MapChunkState, MapTileState, TileInfo};
use bm_storage::{GridKey, MapStorage};

struct ReadOnly(Arc<dyn MapStorage>);

fn grid(kind: CellKind) -> GridKey {
    match kind {
        CellKind::Tiles => GridKey::TileState,
        CellKind::Chunks => GridKey::ChunkState,
        CellKind::Regions => GridKey::RegionState,
    }
}

impl CellIo for ReadOnly {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.read_grid(grid(kind), cell).map_err(io::Error::other)?.map(|s| s.data))
    }

    fn write_cell(&self, _: CellKind, _: Tile, _: &[u8]) -> io::Result<()> {
        Ok(())
    }

    // a corrupt cell is the render thread's to heal
    fn delete_cell(&self, _: CellKind, _: Tile) -> io::Result<()> {
        Ok(())
    }

    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>> {
        self.0.list_grid(grid(kind)).map_err(io::Error::other)
    }
}

/// The stored state of hires tile `tile`.
pub fn tile_info(storage: &Arc<dyn MapStorage>, (x, z): Tile) -> TileInfo {
    MapTileState::new(ReadOnly(storage.clone())).get(x, z)
}

/// The chunk timestamp ("hash") the chunk was last rendered from.
pub fn chunk_hash(storage: &Arc<dyn MapStorage>, (x, z): Tile) -> i32 {
    MapChunkState::new(ReadOnly(storage.clone())).get(x, z)
}
