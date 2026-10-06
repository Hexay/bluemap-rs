//! `HiresModelManager.render`: one tile through the render passes, then sorted by material.

use bm_format::grid::{Grid, Tile};
use bm_format::prbm::TileModel;
use bm_resources::datapack::BiomeTable;
use bm_resources::resource_pack::ResourcePack;
use bm_world::{BlockStates, ChunkArea, DimensionType};

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
        if registry.len() > self.states.len() {
            return Err(Error::StaleStateCache { registry: registry.len(), cache: self.states.len() });
        }
        let (min_x, min_z) = grid.tile_min(tile);
        let max = [min_x + grid.size[0] - 1, min_z + grid.size[1] - 1];
        let min = [min_x, min_z];
        let masking = Masking::new(self.settings, self.dimension.has_skylight, min, max);
        out.volume.fill(area, &masking, min, max);
        let ctx = Ctx {
            pack: self.pack,
            states: self.states,
            settings: self.settings,
            biomes: self.biomes,
            view: View::new(area, masking, &out.volume),
        };
        out.unsorted.clear();
        out.columns.clear();
        // TODO: entity pass (`EntityRenderPass`); core ships no entity models, so BlueMap's output has none either
        out.truncated = block_pass::render(&ctx, min, max, &mut out.unsorted, &mut out.columns).is_err();
        mesh::sort_by_material(&out.unsorted, &mut out.model);
        Ok(())
    }
}
