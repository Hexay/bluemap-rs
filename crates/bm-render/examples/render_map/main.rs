//! Renders every hires tile a golden (Java BlueMap) webroot has, into a new webroot that `bm-golden diff-render`
//! can compare against it. Material ids come from the golden `textures.json`, settings from its map config.
//! Usage: cargo run -p bm-render --release --example render_map -- <world> <dimension> <golden webroot> <out webroot>
//!        [--map id] [--config maps/<id>.conf] [--jar client.jar] [--extensions dir]

mod conf;
mod fixture;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use bm_compress::Compression;
use bm_format::grid::tile_file;
use fixture::Fixture;
use rayon::prelude::*;

fn main() -> Result<()> {
    let mut positional = Vec::new();
    let mut flags = HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.strip_prefix("--") {
            Some(flag) => _ = flags.insert(flag.to_owned(), it.next().context("flag without a value")?),
            None => positional.push(a),
        }
    }
    let [world, dimension, golden, out] = <[String; 4]>::try_from(positional)
        .map_err(|_| anyhow!("usage: render_map <world> <dimension> <golden webroot> <out webroot> [--map id]"))?;
    let (golden, out) = (PathBuf::from(golden), PathBuf::from(out));
    let path = |k: &str| flags.get(k).map(PathBuf::from);

    let start = Instant::now();
    let fx = Fixture::load(
        Path::new(&world),
        &dimension,
        &golden,
        flags.get("map").map(String::as_str),
        path("config").as_deref(),
        &path("jar").unwrap_or_else(fixture::client_jar),
        &path("extensions").unwrap_or_else(fixture::extensions_dir),
    )?;
    println!("resources loaded in {:.2}s", start.elapsed().as_secs_f64());

    let tiles = fx.golden_tiles()?;
    let start = Instant::now();
    let area = fx.load_area(&tiles);
    println!("{} chunks decoded in {:.2}s", area.width * area.depth, start.elapsed().as_secs_f64());

    let mesh_nanos = AtomicU64::new(0);
    let start = Instant::now();
    let rendered = fx.render(&area, &tiles, &mesh_nanos)?;
    let compressed: Vec<Vec<u8>> =
        rendered.par_iter().map(|r| Compression::Gzip.compress(&r.prbm)).collect::<Result<_, _>>()?;
    let elapsed = start.elapsed().as_secs_f64();
    let map_dir = out.join("maps").join(&fx.golden.id);
    for (r, bytes) in rendered.iter().zip(compressed) {
        let path = map_dir.join(format!("{}{}", tile_file(0, r.tile), Compression::Gzip.file_suffix()));
        std::fs::create_dir_all(path.parent().expect("tile files have a parent"))?;
        std::fs::write(path, bytes)?;
    }
    let faces: usize = rendered.iter().map(|r| r.faces).sum();
    let mesh = mesh_nanos.into_inner() as f64 / 1e9;
    println!(
        "{} tiles, {faces} faces in {elapsed:.2}s ({:.1} tiles/s incl. PRBM and gzip, {} threads)",
        tiles.len(),
        tiles.len() as f64 / elapsed,
        rayon::current_num_threads()
    );
    println!("meshing alone: {mesh:.2} thread-seconds, {:.1} tiles/s per thread", tiles.len() as f64 / mesh);
    let lowres_nanos = AtomicU64::new(0);
    fx.render_lowres(&area, &tiles, &lowres_nanos)?;
    println!("lowres-only block pass: {:.2} thread-seconds", lowres_nanos.into_inner() as f64 / 1e9);
    match fx.check_lowres(&rendered) {
        Ok((checked, bad)) => {
            println!("lowres columns vs golden LOD 1: {checked} compared, {} differ", bad.len());
            bad.iter().take(10).for_each(|b| println!("  {b}"));
        }
        Err(e) => println!("lowres columns not compared: {e}"),
    }
    copy_settings(&golden, &out, &fx.golden.id)
}

/// Root and map `settings.json` and `textures.json`, so the output is a webroot the oracle can open.
fn copy_settings(golden: &Path, out: &Path, id: &str) -> Result<()> {
    std::fs::copy(golden.join("settings.json"), out.join("settings.json"))?;
    let map = Path::new("maps").join(id);
    std::fs::create_dir_all(out.join(&map))?;
    for entry in std::fs::read_dir(golden.join(&map))? {
        let name = entry?.file_name();
        let s = name.to_string_lossy();
        if s.starts_with("settings.json") || s.starts_with("textures.json") {
            std::fs::copy(golden.join(&map).join(&name), out.join(&map).join(&name))?;
        }
    }
    Ok(())
}
