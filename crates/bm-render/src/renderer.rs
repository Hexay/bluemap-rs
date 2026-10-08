//! `HiresModelManager.render`: one tile through the render passes, then sorted by material.

use bm_format::grid::{Grid, Tile};
use bm_format::prbm::TileModel;
use bm_resources::datapack::BiomeTable;
use bm_resources::resource_pack::ResourcePack;
use bm_world::{BlockStates, ChunkArea, DimensionType};

use crate::block_pass::Stop;
use crate::context::Ctx;
use crate::settings::RenderSettings;
use crate::states::StateCache;
use crate::view::{Masking, View, Volume};
use crate::{ColumnMeta, Error, block_pass, mesh};

/// Shareable across threads; each thread brings its own [`TileBuffers`].
pub struct HiresRenderer<'r, 'a> {
    pub pack: &'a ResourcePack,
    pub states: &'r StateCache<'a>,
    pub settings: &'r RenderSettings,
    pub biomes: &'r BiomeTable,
    pub dimension: &'r DimensionType,
}

/// Per-thread output and scratch, reused across tiles.
#[derive(Default)]
pub struct TileBuffers {
    /// The tile, sorted by material: ready for [`TileModel::write_prbm`].
    pub model: TileModel,
    /// One per block column, x-major, as `TileMetaConsumer.set` receives them.
    pub columns: Vec<ColumnMeta>,
    /// True when the tile hit [`crate::MAX_FACES`] and was cut short, as BlueMap does.
    pub truncated: bool,
    unsorted: TileModel,
    volume: Volume,
}

impl HiresRenderer<'_, '_> {
    /// Renders `tile` of `grid` from `area`, which must hold the tile's chunks plus a two-block border (biome
    /// blending reaches that far; blocks outside the area read as air). `registry` is the area's block-state registry, to check the cache is current.
    pub fn render_tile(
        &self,
        area: &ChunkArea,
        registry: &BlockStates,
        grid: &Grid,
        tile: Tile,
        out: &mut TileBuffers,
    ) -> Result<(), Error> {
        self.block_pass(area, registry, grid, tile, out, true)?;
        mesh::sort_by_material(&out.unsorted, &mut out.model);
        Ok(())
    }

    /// [`Self::render_tile`] for the columns and `truncated` only, for maps without hires tiles: the same block
    /// pass, minus all geometry. `out.model` is left alone.
    pub fn render_lowres(
        &self,
        area: &ChunkArea,
        registry: &BlockStates,
        grid: &Grid,
        tile: Tile,
        out: &mut TileBuffers,
    ) -> Result<(), Error> {
        self.block_pass(area, registry, grid, tile, out, false)?;
        if cfg!(debug_assertions) {
            let mut full = TileBuffers::default();
            self.render_tile(area, registry, grid, tile, &mut full)?;
            assert_eq!((&out.columns, out.truncated), (&full.columns, full.truncated), "lowres-only {tile:?}");
        }
        Ok(())
    }

    /// Fills the volume and runs the block pass into `out.unsorted` (only counting faces without `geometry`),
    /// `out.columns` and `out.truncated`.
    fn block_pass(
        &self,
        area: &ChunkArea,
        registry: &BlockStates,
        grid: &Grid,
        tile: Tile,
        out: &mut TileBuffers,
        geometry: bool,
    ) -> Result<(), Error> {
        if registry.len() > self.states.len() {
            return Err(Error::StaleStateCache { registry: registry.len(), cache: self.states.len() });
        }
        let (min_x, min_z) = grid.tile_min(tile);
        let max = [min_x + grid.size[0] - 1, min_z + grid.size[1] - 1];
        let min = [min_x, min_z];
        let masking = Masking::new(self.settings, self.dimension.has_skylight, min, max);
        // without geometry, top-only walks stop near the surface, so the volume can start there; a walk that goes
        // deeper refills. Masked-out blocks (a y range, say) don't stop a walk, so masked tiles always fill fully.
        let floored = !geometry && self.settings.render_top_only && masking.unmasked();
        let mut floor = floored.then(|| surface_floor(area, min, max)).flatten();
        loop {
            out.volume.fill(area, &masking, min, max, floor, |s| self.states.flags(s).is_air());
            let ctx = Ctx {
                pack: self.pack,
                states: self.states,
                settings: self.settings,
                biomes: self.biomes,
                view: View::new(area, masking, &out.volume),
                geometry,
            };
            out.unsorted.clear();
            out.columns.clear();
            out.columns.reserve(grid.size[0] as usize * grid.size[1] as usize);
            // TODO: entity pass (`EntityRenderPass`); core ships no entity models, so BlueMap's output has none either
            out.truncated = match block_pass::render(&ctx, min, max, &mut out.unsorted, &mut out.columns) {
                Ok(()) => false,
                Err(Stop::Full) => true,
                Err(Stop::Shallow) => {
                    floor = None;
                    continue;
                }
            };
            return Ok(());
        }
    }
}

/// Blocks a top-only column walk may go below the lowest `OCEAN_FLOOR` of the tile before it needs a deeper fill.
const FLOOR_SLACK: i32 = 4;

/// Where to start a top-only volume: `None` when a column has no heightmap.
fn surface_floor(area: &ChunkArea, min: [i32; 2], max: [i32; 2]) -> Option<i32> {
    let mut floor = i32::MAX;
    for x in min[0]..=max[0] {
        for z in min[1]..=max[1] {
            floor = floor.min(area.chunk_at_block(x, z)?.ocean_floor_y(x, z)?);
        }
    }
    Some(floor.saturating_sub(FLOOR_SLACK))
}
