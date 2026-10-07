//! Shared render inputs and the block being rendered (`BlockNeighborhood`).

use bm_math::Color;
use bm_resources::datapack::BiomeTable;
use bm_resources::resource_pack::ResourcePack;
use bm_world::StateId;

use crate::settings::RenderSettings;
use crate::states::{StateCache, StateInfo};
use crate::view::View;

pub(crate) struct Ctx<'r, 'a> {
    pub pack: &'a ResourcePack,
    pub states: &'r StateCache<'a>,
    pub settings: &'r RenderSettings,
    pub biomes: &'r BiomeTable,
    pub view: View<'r>,
}

impl<'r, 'a> Ctx<'r, 'a> {
    /// `BlockColorCalculator.getBlockColor` for `state` at the position.
    pub fn tint(&self, state: &StateInfo, x: i32, y: i32, z: i32) -> Color {
        self.pack.block_colors.color(&state.state, (x, y, z), self.biomes, |x, y, z| self.view.biome(x, y, z))
    }
}

/// The block being rendered, which is always inside the render mask.
pub(crate) struct Block<'r, 'a> {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: StateId,
    pub info: &'r StateInfo<'a>,
    pub sky: u8,
    pub block_light: u8,
    /// `ExtendedBlock.isRemoveIfCave`.
    pub remove_if_cave: bool,
    /// [`View::interior_index`] of the block.
    index: Option<usize>,
}

impl<'r, 'a> Block<'r, 'a> {
    /// `index` must be `ctx.view.interior_index(x, y, z)`; callers walking a column derive it without the bounds math.
    pub fn new(ctx: &Ctx<'r, 'a>, x: i32, y: i32, z: i32, index: Option<usize>) -> Self {
        debug_assert_eq!(index, ctx.view.interior_index(x, y, z));
        let (id, (sky, block_light)) = match index {
            Some(i) => (ctx.view.state_at(i), ctx.view.light_at(i)),
            None => (ctx.view.state(x, y, z), ctx.view.light(x, y, z)),
        };
        let info = ctx.states.get(id);
        let s = ctx.settings;
        // air renders nothing, so it never needs the heightmap read
        let remove_if_cave = !info.is_air()
            && y < s.remove_caves_below_y
            && ctx.view.ocean_floor_y(x, z).is_none_or(|floor| y < floor.wrapping_add(s.cave_detection_ocean_floor));
        Self { x, y, z, id, info, sky, block_light, remove_if_cave, index }
    }

    /// The neighbour's volume index when the direct read is in bounds.
    fn neighbor_index(&self, ctx: &Ctx, dx: i32, dy: i32, dz: i32) -> Option<usize> {
        let unit = |d: i32| d.wrapping_add(1) as u32 <= 2;
        let i = self.index.filter(|_| unit(dx) && unit(dy) && unit(dz))?;
        Some(ctx.view.step(i, dx, dy, dz))
    }

    pub fn neighbor(&self, ctx: &Ctx<'r, 'a>, dx: i32, dy: i32, dz: i32) -> (StateId, &'r StateInfo<'a>) {
        let id = match self.neighbor_index(ctx, dx, dy, dz) {
            Some(i) => ctx.view.state_at(i),
            None => ctx.view.state(self.x + dx, self.y + dy, self.z + dz),
        };
        (id, ctx.states.get(id))
    }

    pub fn neighbor_light(&self, ctx: &Ctx<'r, 'a>, dx: i32, dy: i32, dz: i32) -> (u8, u8) {
        match self.neighbor_index(ctx, dx, dy, dz) {
            Some(i) => ctx.view.light_at(i),
            None => ctx.view.light(self.x + dx, self.y + dy, self.z + dz),
        }
    }

    /// The cave filter on a face's light.
    pub fn culled_as_cave(&self, ctx: &Ctx, sky: u8, block_light: u8) -> bool {
        let light = if ctx.settings.cave_detection_uses_block_light { sky.max(block_light) } else { sky };
        self.remove_if_cave && light == 0
    }
}
