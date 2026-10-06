//! Render state (`core/map/renderstate/**`): what every hires tile was last rendered as, the region-header chunk
//! timestamps it was rendered from ("chunk hashes"), and when each region was last updated. Persisted as gzip'd
//! BlueNBT in `rstate/` cells; layout and change detection: docs/03-rendering.md §5, paths: docs/04-storage-web.md.
//!
//! Each cell file covers `2^shift × 2^shift` entries, indexed `(z & mask) << shift | (x & mask)`.

mod cells;
mod paletted;
mod store;
mod tile_state;

#[cfg(test)]
mod tests;

pub use cells::{
    Cell, ChunkHashes, ChunkInfoRegion, IntCell, IntField, RegionInfoRegion, RegionUpdateTimes, TileInfo,
    TileInfoRegion,
};
pub use store::{CellIo, CellStore, MapChunkState, MapRegionState, MapTileState};
pub use tile_state::{Action, ActionAndNextState, BoundsSituation, TileState, TileUpdateStrategy};

use bm_compress::Compression;
use bm_format::grid::{Tile, tile_path};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Compress(#[from] bm_compress::Error),
    #[error("nbt: {0}")]
    Nbt(#[from] bm_nbt::Error),
    #[error("`{0}` has an unexpected tag type")]
    WrongType(&'static str),
    #[error("missing or empty palette")]
    EmptyPalette,
    #[error("palette (size {size}) has no entry {index}")]
    PaletteIndex { size: usize, index: u8 },
    #[error("{op} {kind:?} cell {cell:?}: {source}")]
    Io { op: &'static str, kind: CellKind, cell: Tile, source: std::io::Error },
    #[error("corrupt {kind:?} cell {cell:?}: {source}")]
    Corrupt { kind: CellKind, cell: Tile, source: Box<Error> },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Decompressed cells are a few KiB; the cap only guards against gzip bombs.
const MAX_CELL_BYTES: usize = 64 << 20;

/// The three render-state grids of a map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellKind {
    /// `MapTileState`: hires tiles, 32×32 per cell.
    Tiles,
    /// `MapChunkState`: chunks, 128×128 per cell.
    Chunks,
    /// `MapRegionState`: region files, 64×64 per cell.
    Regions,
}

impl CellKind {
    pub const ALL: [Self; 3] = [Self::Tiles, Self::Chunks, Self::Regions];

    pub const fn shift(self) -> u32 {
        match self {
            Self::Tiles => 5,
            Self::Chunks => 7,
            Self::Regions => 6,
        }
    }

    /// Entries per cell.
    pub const fn entries(self) -> usize {
        1 << (2 * self.shift())
    }

    /// Index of entry (x, z) — any world coordinate of this grid — inside its cell.
    pub const fn index(self, x: i32, z: i32) -> usize {
        let mask = (1 << self.shift()) - 1;
        (((z & mask) << self.shift()) | (x & mask)) as usize
    }

    /// The cell holding entry (x, z).
    pub const fn cell_of(self, x: i32, z: i32) -> Tile {
        (x >> self.shift(), z >> self.shift())
    }

    /// Directory relative to the map root (`FileMapStorage`).
    pub const fn dir(self) -> &'static str {
        match self {
            Self::Tiles | Self::Chunks => "rstate",
            Self::Regions => "rstate/regions",
        }
    }

    /// File suffix; the files are always gzip despite the missing `.gz`.
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Tiles => ".tiles.dat",
            Self::Chunks => ".chunks.dat",
            Self::Regions => ".regions.dat",
        }
    }

    /// Grid key in SQL storages (`KeyedMapStorage`).
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::Tiles => "bluemap:tile-state",
            Self::Chunks => "bluemap:chunk-state",
            Self::Regions => "bluemap:region-state",
        }
    }

    /// File path relative to the map root, e.g. `rstate/regions/x-1/z0.regions.dat`.
    pub fn relative_path(self, cell: Tile) -> String {
        format!("{}/{}{}", self.dir(), tile_path(cell), self.suffix())
    }
}

/// Storage compression of every render-state cell, regardless of the storage's configured compression.
pub const COMPRESSION: Compression = Compression::Gzip;

pub(crate) fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(COMPRESSION.decompress(bytes, MAX_CELL_BYTES)?)
}

pub(crate) fn compress(nbt: &[u8]) -> Result<Vec<u8>> {
    Ok(COMPRESSION.compress(nbt)?)
}
