//! A golden map and everything needed to re-render its hires tiles: resources, the golden material ids, the
//! world and the map config's render settings. Shared by the `render_map` example and the golden test.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use bm_format::grid::{Grid, Tile};
use bm_golden::WebrootMap;
use bm_render::{HiresRenderer, RenderSettings, StateCache, TileBuffers};
use bm_resources::PackVersions;
use bm_resources::datapack::BiomeTable;
use bm_resources::packs::load_order;
use bm_resources::resource_pack::ResourcePack;
use bm_resources::texture::TextureGallery;
use bm_world::{BlockStates, ChunkArea, World};
use rayon::prelude::*;

use super::conf;

pub const JAR: &str = "C:/Users/hexay/bluemap-rs/work/bluemap/vanilla/data/minecraft-client-26.3.jar";
const DEFAULTS: &str = include_str!("../../../../assets/resourceExtensions/data/minecraft/defaultBlockstates.json");

pub fn extensions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/resourceExtensions")
}

pub struct Fixture {
    pub golden: WebrootMap,
    pub grid: Grid,
    pub settings: RenderSettings,
    pub pack: ResourcePack,
    pub gallery: TextureGallery,
    pub states: Arc<BlockStates>,
    pub biome_table: BiomeTable,
    pub world: World,
}

/// One rendered tile: PRBM bytes (uncompressed) and face count.
pub struct Rendered {
    pub tile: Tile,
    pub prbm: Vec<u8>,
    pub faces: usize,
}

impl Fixture {
    /// `config` defaults to `<golden>/../config/maps/<id>.conf`.
    pub fn load(
        world: &Path,
        dimension: &str,
        golden: &Path,
        map: Option<&str>,
        config: Option<&Path>,
        jar: &Path,
        extensions: &Path,
    ) -> Result<Self> {
        let golden_map = WebrootMap::open(golden, map)?;
        let config =
            config.map_or_else(|| golden.join(format!("../config/maps/{}.conf", golden_map.id)), Path::to_path_buf);
        let settings =
            conf::render_settings(&std::fs::read_to_string(&config).with_context(|| config.display().to_string())?)?;
        let versions = PackVersions::read(jar)?;
        let roots = [extensions.to_path_buf(), jar.to_path_buf()];
        let pack = ResourcePack::load(&load_order(&roots, versions.resource), &load_order(&roots, versions.data));
        let gallery = TextureGallery::read_textures_file(&golden_map.textures_json()?)?;
        let states = Arc::new(BlockStates::default());
        states.add_defaults_json(DEFAULTS)?;
        let biomes = Arc::new(bm_world::Biomes::default());
        let biome_table = pack.datapack.biome_table(&biomes);
        let datapack = |k: &str| pack.datapack.dimension_type(k).cloned();
        let world = World::open(world, dimension, states.clone(), biomes, &datapack)?;
        Ok(Self {
            grid: golden_map.settings.hires_grid(),
            golden: golden_map,
            settings,
            pack,
            gallery,
            states,
            biome_table,
            world,
        })
    }

    pub fn golden_tiles(&self) -> Result<Vec<Tile>> {
        let tiles: Vec<Tile> = self.golden.tiles(0)?.into_iter().collect();
        if tiles.is_empty() {
            bail!("the golden map has no hires tiles");
        }
        Ok(tiles)
    }

    /// The chunks under every tile plus the two-block border the renderers read around a tile.
    pub fn load_area(&self, tiles: &[Tile]) -> ChunkArea {
        let mins = tiles.iter().map(|&t| self.grid.tile_min(t));
        let (x0, z0) = mins.clone().fold((i32::MAX, i32::MAX), |(a, b), (x, z)| (a.min(x), b.min(z)));
        let [w, d] = self.grid.size;
        let (x1, z1) = mins.fold((i32::MIN, i32::MIN), |(a, b), (x, z)| (a.max(x + w), b.max(z + d)));
        let (cx0, cz0) = ((x0 - 2) >> 4, (z0 - 2) >> 4);
        let (cx1, cz1) = ((x1 + 1) >> 4, (z1 + 1) >> 4);
        self.world.load_area(cx0, cz0, cx1 - cx0 + 1, cz1 - cz0 + 1)
    }

    /// Renders `tiles` in parallel; `mesh_nanos` accumulates time spent meshing alone (all threads).
    pub fn render(&self, area: &ChunkArea, tiles: &[Tile], mesh_nanos: &AtomicU64) -> Result<Vec<Rendered>> {
        let cache = StateCache::new(&self.pack, &self.gallery, &self.states);
        let renderer = HiresRenderer {
            pack: &self.pack,
            states: &cache,
            settings: &self.settings,
            biomes: &self.biome_table,
            dimension: &self.world.dimension_type,
        };
        tiles
            .par_iter()
            .map_init(TileBuffers::default, |buf, &tile| {
                let start = Instant::now();
                renderer.render_tile(area, &self.states, &self.grid, tile, buf)?;
                mesh_nanos.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
                if buf.truncated {
                    eprintln!("tile {tile:?} reached the face limit");
                }
                let mut prbm = Vec::new();
                buf.model.write_prbm(&mut prbm)?;
                Ok(Rendered { tile, prbm, faces: buf.model.len() })
            })
            .collect()
    }
}
