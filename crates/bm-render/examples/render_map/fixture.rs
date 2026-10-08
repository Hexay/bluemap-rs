//! A golden map and everything needed to re-render its hires tiles: resources, the golden material ids, the
//! world and the map config's render settings. Shared by the `render_map` example and the golden test.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use bm_format::grid::{Grid, Tile};
use bm_format::lowres::LowresTile;
use bm_golden::WebrootMap;
use bm_render::{ColumnMeta, HiresRenderer, RenderSettings, StateCache, TileBuffers};
use bm_resources::PackVersions;
use bm_resources::datapack::BiomeTable;
use bm_resources::packs::load_order;
use bm_resources::resource_pack::ResourcePack;
use bm_resources::texture::TextureGallery;
use bm_world::{BlockStates, ChunkArea, World};
use rayon::prelude::*;

use super::conf;

const DEFAULTS: &str = include_str!("../../../../assets/resourceExtensions/data/minecraft/defaultBlockstates.json");

/// The repo's `work/` folder (git-ignored), found from a checkout or any worktree inside it.
pub fn work_dir() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.ancestors().map(|a| a.join("work")).find(|w| w.is_dir()).unwrap_or_else(|| here.join("../../work"))
}

pub fn client_jar() -> PathBuf {
    work_dir().join("bluemap/vanilla/data/minecraft-client-26.3.jar")
}

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

/// One rendered tile: PRBM bytes (uncompressed), face count and the lowres column data.
pub struct Rendered {
    pub tile: Tile,
    pub prbm: Vec<u8>,
    pub faces: usize,
    pub columns: Vec<ColumnMeta>,
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
        self.render_with(area, tiles, mesh_nanos, false)
    }

    /// [`Self::render`] through `HiresRenderer::render_lowres`: columns only, empty PRBM.
    pub fn render_lowres(&self, area: &ChunkArea, tiles: &[Tile], mesh_nanos: &AtomicU64) -> Result<Vec<Rendered>> {
        self.render_with(area, tiles, mesh_nanos, true)
    }

    fn render_with(
        &self,
        area: &ChunkArea,
        tiles: &[Tile],
        mesh_nanos: &AtomicU64,
        lowres_only: bool,
    ) -> Result<Vec<Rendered>> {
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
                if lowres_only {
                    renderer.render_lowres(area, &self.states, &self.grid, tile, buf)?;
                } else {
                    renderer.render_tile(area, &self.states, &self.grid, tile, buf)?;
                }
                mesh_nanos.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
                if buf.truncated {
                    eprintln!("tile {tile:?} reached the face limit");
                }
                let mut prbm = Vec::new();
                if !lowres_only {
                    buf.model.write_prbm(&mut prbm)?;
                }
                let faces = if lowres_only { 0 } else { buf.model.len() };
                Ok(Rendered { tile, prbm, faces, columns: buf.columns.clone() })
            })
            .collect()
    }

    /// Compares every rendered column with the golden LOD 1 pixel `TileMetaConsumer` wrote for it; returns the
    /// columns compared and a description of each mismatch.
    pub fn check_lowres(&self, rendered: &[Rendered]) -> Result<(usize, Vec<String>)> {
        let size = self.golden.settings.lowres.tile_size;
        let lowres = Grid { size, offset: [0, 0] };
        let mut tiles: HashMap<Tile, LowresTile> = HashMap::new();
        let (mut checked, mut bad) = (0, Vec::new());
        for c in rendered.iter().flat_map(|r| &r.columns) {
            let t = lowres.tile_of(c.x, c.z);
            let tile = match tiles.entry(t) {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => {
                    let png = self.golden.tile_bytes(1, t)?;
                    e.insert(LowresTile::decode_png(&png, size.map(|s| s as usize))?)
                }
            };
            let (px, pz) = (c.x.rem_euclid(size[0]) as usize, c.z.rem_euclid(size[1]) as usize);
            let mut color = c.color;
            let expected = (color.straight().get_int() as u32, sign_extend(c.height & 0xFFFF), c.block_light as u8);
            let actual = (tile.color(px, pz), tile.height(px, pz), tile.block_light(px, pz));
            checked += 1;
            if expected != actual {
                bad.push(format!("column {},{}: ours {expected:08x?}, golden {actual:08x?}", c.x, c.z));
            }
        }
        Ok((checked, bad))
    }
}

/// `LowresTile.getHeight`'s read-back of a 16-bit height.
fn sign_extend(h: i32) -> i32 {
    if h > 0x8000 { h | !0xFFFF } else { h }
}
