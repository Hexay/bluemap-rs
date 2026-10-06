//! Renders every hires tile a golden (Java BlueMap) webroot has, into a new webroot that `bm-golden diff-render`
//! can compare against it. Material ids come from the golden `textures.json`, settings from its map config.
//! Usage: cargo run -p bm-render --release --example render_map -- <world> <dimension> <golden webroot> <out webroot>
//!        [--map id] [--config maps/<id>.conf] [--jar client.jar] [--extensions dir]

mod conf;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use bm_compress::Compression;
use bm_format::grid::{Grid, Tile, tile_file};
use bm_golden::WebrootMap;
use bm_render::{HiresRenderer, StateCache, TileBuffers};
use bm_resources::packs::load_order;
use bm_resources::resource_pack::ResourcePack;
use bm_resources::texture::TextureGallery;
use bm_resources::PackVersions;
use bm_world::{Biomes, BlockStates, World};
use rayon::prelude::*;

const JAR: &str = "C:/Users/hexay/bluemap-rs/work/bluemap/vanilla/data/minecraft-client-26.3.jar";
const DEFAULTS: &str = include_str!("../../../../assets/resourceExtensions/data/minecraft/defaultBlockstates.json");

struct Args {
    world: PathBuf,
    dimension: String,
    golden: PathBuf,
    out: PathBuf,
    map: Option<String>,
    config: Option<PathBuf>,
    jar: PathBuf,
    extensions: PathBuf,
}

fn args() -> Result<Args> {
    let mut positional = Vec::new();
    let mut flags = std::collections::HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.strip_prefix("--") {
            Some(flag) => _ = flags.insert(flag.to_owned(), it.next().context("flag without a value")?),
            None => positional.push(a),
        }
    }
    let [world, dimension, golden, out] = <[String; 4]>::try_from(positional)
        .map_err(|_| anyhow::anyhow!("usage: render_map <world> <dimension> <golden webroot> <out webroot> [--map id]"))?;
    let path = |k: &str, default: &str| PathBuf::from(flags.get(k).map_or(default, String::as_str));
    Ok(Args {
        world: world.into(),
        dimension,
        golden: golden.into(),
        out: out.into(),
        map: flags.get("map").cloned(),
        config: flags.get("config").map(PathBuf::from),
        jar: path("jar", JAR),
        extensions: path("extensions", "assets/resourceExtensions"),
    })
}

fn main() -> Result<()> {
    let a = args()?;
    let golden = WebrootMap::open(&a.golden, a.map.as_deref())?;
    let config = a.config.clone().unwrap_or_else(|| a.golden.join(format!("../config/maps/{}.conf", golden.id)));
    let settings = conf::render_settings(&std::fs::read_to_string(&config).with_context(|| config.display().to_string())?)?;
    let grid = golden.settings.hires_grid();

    let start = Instant::now();
    let versions = PackVersions::read(&a.jar)?;
    let roots = [a.extensions.clone(), a.jar.clone()];
    let pack = ResourcePack::load(&load_order(&roots, versions.resource), &load_order(&roots, versions.data));
    let gallery = TextureGallery::read_textures_file(&golden.textures_json()?)?;
    let states = Arc::new(BlockStates::default());
    states.add_defaults_json(DEFAULTS)?;
    let biomes = Arc::new(Biomes::default());
    let biome_table = pack.datapack.biome_table(&biomes);
    let world = World::open(&a.world, &a.dimension, states.clone(), biomes, &|k| pack.datapack.dimension_type(k).cloned())?;
    println!("resources loaded in {:.2}s", start.elapsed().as_secs_f64());

    let tiles: Vec<Tile> = golden.tiles(0)?.into_iter().collect();
    if tiles.is_empty() {
        bail!("the golden map has no hires tiles");
    }
    let start = Instant::now();
    let area = load_area(&world, &grid, &tiles);
    let cache = StateCache::new(&pack, &gallery, &states);
    println!("{} chunks decoded in {:.2}s", area.width * area.depth, start.elapsed().as_secs_f64());

    let renderer = HiresRenderer {
        pack: &pack,
        states: &cache,
        settings: &settings,
        biomes: &biome_table,
        dimension: &world.dimension_type,
    };
    let map_dir = a.out.join("maps").join(&golden.id);
    let start = Instant::now();
    let rendered: Vec<Result<(usize, Vec<u8>, Tile)>> = tiles
        .par_iter()
        .map_init(TileBuffers::default, |buf, &t| {
            renderer.render_tile(&area, &states, &grid, t, buf)?;
            if buf.truncated {
                eprintln!("tile {t:?} reached the face limit");
            }
            let mut prbm = Vec::new();
            buf.model.write_prbm(&mut prbm)?;
            Ok((buf.model.len(), Compression::Gzip.compress(&prbm)?, t))
        })
        .collect();
    let render_time = start.elapsed().as_secs_f64();
    let mut faces = 0;
    for r in rendered {
        let (n, bytes, t) = r?;
        faces += n;
        let path = map_dir.join(format!("{}{}", tile_file(0, t), Compression::Gzip.file_suffix()));
        std::fs::create_dir_all(path.parent().expect("tile files have a parent"))?;
        std::fs::write(path, bytes)?;
    }
    println!(
        "{} tiles, {faces} faces rendered in {render_time:.2}s ({:.1} tiles/s incl. PRBM+gzip, {} threads)",
        tiles.len(),
        tiles.len() as f64 / render_time,
        rayon::current_num_threads()
    );
    copy_settings(&a.golden, &a.out, &golden.id)
}

/// The chunks under every tile plus the two-block border the renderers read around a tile.
fn load_area(world: &World, grid: &Grid, tiles: &[Tile]) -> bm_world::ChunkArea {
    let mins = tiles.iter().map(|&t| grid.tile_min(t));
    let (x0, z0) = mins.clone().fold((i32::MAX, i32::MAX), |(a, b), (x, z)| (a.min(x), b.min(z)));
    let (x1, z1) = mins.fold((i32::MIN, i32::MIN), |(a, b), (x, z)| (a.max(x + grid.size[0]), b.max(z + grid.size[1])));
    let (cx0, cz0) = ((x0 - 2) >> 4, (z0 - 2) >> 4);
    let (cx1, cz1) = ((x1 + 1) >> 4, (z1 + 1) >> 4);
    world.load_area(cx0, cz0, cx1 - cx0 + 1, cz1 - cz0 + 1)
}

/// Root and map `settings.json` and `textures.json`, so the output is a webroot the oracle can open.
fn copy_settings(golden: &Path, out: &Path, id: &str) -> Result<()> {
    std::fs::copy(golden.join("settings.json"), out.join("settings.json"))?;
    let map = Path::new("maps").join(id);
    for entry in std::fs::read_dir(golden.join(&map))? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("settings.json") || name.to_string_lossy().starts_with("textures.json") {
            std::fs::copy(entry.path(), out.join(&map).join(&name))?;
        }
    }
    Ok(())
}
