//! The map settings the hires mesher reads (`RenderSettings.java`, defaults from `MapConfig.java`).

use std::sync::Arc;

/// The map's render mask (`Mask`): which blocks are inside the rendered area.
pub trait RenderMask: Send + Sync {
    fn test(&self, x: i32, y: i32, z: i32) -> bool;

    /// `isInsideRenderBoundaries(x, z)`: false only when no block of the column can be inside
    /// (`test(column box).getOr(true)`).
    fn test_column(&self, _x: i32, _z: i32) -> bool {
        true
    }

    /// `test(box)` over every y of the x/z rectangle: `Some(true)` wholly inside, `Some(false)` wholly outside,
    /// `None` mixed or unknown. Lets a tile skip per-block mask tests.
    fn test_area(&self, _min: [i32; 2], _max: [i32; 2]) -> Option<bool> {
        None
    }
}

#[derive(Clone)]
pub struct RenderSettings {
    pub remove_caves_below_y: i32,
    /// Relative to the `OCEAN_FLOOR` heightmap.
    pub cave_detection_ocean_floor: i32,
    pub cave_detection_uses_block_light: bool,
    /// 0..1, the map config's `ambient-light`.
    pub ambient_light: f32,
    /// Blocks outside the mask read as air (with `edge_light_strength` sky light), so cut faces get drawn.
    pub render_edges: bool,
    pub edge_light_strength: i32,
    /// Read by the tile pre-checks, not the mesher.
    pub ignore_missing_light_data: bool,
    /// `!enableHires || (!perspective && !freeFlight)`: only faces looking up, columns stop at the first opaque block.
    pub render_top_only: bool,
    /// `None` renders everything.
    pub mask: Option<Arc<dyn RenderMask>>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            remove_caves_below_y: 55,
            cave_detection_ocean_floor: 10000,
            cave_detection_uses_block_light: false,
            ambient_light: 0.0,
            render_edges: true,
            edge_light_strength: 15,
            ignore_missing_light_data: false,
            render_top_only: false,
            mask: None,
        }
    }
}
