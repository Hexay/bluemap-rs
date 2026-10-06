//! What the renderers see of the world: `ExtendedBlock`'s masked reads over a [`ChunkArea`]. Positions outside the
//! area, in absent or failed chunks read as BlueMap's empty chunk (air, no light, sections 0..=255).

use bm_world::{BiomeId, ChunkArea, StateId};

use crate::settings::{RenderMask, RenderSettings};

pub(crate) struct View<'a> {
    pub area: &'a ChunkArea,
    mask: Option<&'a dyn RenderMask>,
    render_edges: bool,
    edge_sky: u8,
}

impl<'a> View<'a> {
    pub fn new(area: &'a ChunkArea, settings: &'a RenderSettings, has_skylight: bool) -> Self {
        Self {
            area,
            mask: settings.mask.as_deref(),
            render_edges: settings.render_edges,
            edge_sky: if has_skylight { settings.edge_light_strength as u8 } else { 0 },
        }
    }

    pub fn inside(&self, x: i32, y: i32, z: i32) -> bool {
        self.mask.is_none_or(|m| m.test(x, y, z))
    }

    pub fn inside_column(&self, x: i32, z: i32) -> bool {
        self.mask.is_none_or(|m| m.test_column(x, z))
    }

    fn edge(&self, x: i32, y: i32, z: i32) -> bool {
        self.render_edges && !self.inside(x, y, z)
    }

    pub fn state(&self, x: i32, y: i32, z: i32) -> StateId {
        if self.edge(x, y, z) {
            return StateId::AIR;
        }
        self.area.block(x, y, z)
    }

    /// `(sky, block)`.
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let (sky, block) = self.area.chunk_at_block(x, z).map_or((0, 0), |c| c.light(x, y, z));
        if self.edge(x, y, z) { (self.edge_sky, block) } else { (sky, block) }
    }

    pub fn biome(&self, x: i32, y: i32, z: i32) -> BiomeId {
        self.area.biome(x, y, z)
    }

    pub fn ocean_floor_y(&self, x: i32, z: i32) -> Option<i32> {
        self.area.chunk_at_block(x, z).and_then(|c| c.ocean_floor_y(x, z))
    }

    /// `chunk.getMinY/getMaxY` of the column's chunk.
    pub fn column_y_range(&self, x: i32, z: i32) -> (i32, i32) {
        self.area.chunk_at_block(x, z).map_or((0, 255), |c| (c.min_y(), c.max_y()))
    }
}
