//! BlueMap's hires mesher (`core/map/hires`, docs/03-rendering.md §2): renders a hires tile from a loaded
//! [`bm_world::ChunkArea`] into a [`bm_format::prbm::TileModel`], and reports the per-column colour, height and light
//! BlueMap feeds its lowres layer. Arithmetic follows Java's float/double/cast order so tiles match BlueMap's.

mod block_pass;
mod context;
mod liquid;
mod mesh;
mod renderer;
mod resource;
mod settings;
mod states;
mod view;

pub use mesh::{CapacityReached, MAX_FACES};
pub use renderer::{HiresRenderer, TileBuffers};
pub use settings::{RenderMask, RenderSettings};
pub use states::{Props, StateCache, StateInfo};

use bm_math::Color;

/// What `BlockRenderPass` hands its `TileMetaConsumer` for one block column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnMeta {
    pub x: i32,
    pub z: i32,
    /// Premultiplied; fully transparent when nothing visible was found.
    pub color: Color,
    /// Highest block with a visible colour, 0 when none.
    pub height: i32,
    pub block_light: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the block state registry has {registry} states but the state cache only {cache}; call StateCache::update")]
    StaleStateCache { registry: usize, cache: usize },
}
