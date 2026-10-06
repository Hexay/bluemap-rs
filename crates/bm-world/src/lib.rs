//! Minecraft world reading for the renderer. See docs/01-world-reading.md for the formats and BlueMap's behaviour.

pub mod chunk;
pub mod dimension;
mod packed;
pub mod region;
pub mod registry;
mod world;

pub use chunk::{BlockEntity, Chunk, ChunkContext};
pub use dimension::DimensionType;
pub use registry::{BiomeId, Biomes, BlockState, BlockStates, StateId};
pub use world::{ChunkArea, ChunkSlot, World};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Compression(#[from] bm_compress::Error),
    #[error(transparent)]
    Nbt(#[from] bm_nbt::Error),
    #[error("unsupported chunk compression type {0}")]
    UnsupportedCompression(u8),
    #[error("unsupported chunk data version {0}")]
    UnsupportedVersion(i32),
    #[error("corrupt world data: {0}")]
    Corrupt(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
