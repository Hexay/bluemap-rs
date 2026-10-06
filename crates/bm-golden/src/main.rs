use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

use anyhow::{Result, ensure};
use bm_format::grid::Tile as TilePos;
use bm_golden::diff::{RenderDiff, face_cell};
use bm_golden::{Tile, WebrootMap};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Compare bluemap-rs output against Java BlueMap's golden output")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Face-by-face hires diff of two webroots rendered from the same world and map config
    DiffRender(DiffArgs),
    /// Re-encode every tile of a webroot with bm-format: hires PRBM must be byte-identical, lowres PNG pixel-identical
    Roundtrip {
        webroot: PathBuf,
        #[arg(long)]
        map: Option<String>,
    },
}

#[derive(clap::Args)]
struct DiffArgs {
    /// Webroot rendered by Java BlueMap
    golden: PathBuf,
    /// Webroot rendered by bluemap-rs
    candidate: PathBuf,
    /// Map id (optional when the webroots hold one map)
    #[arg(long)]
    map: Option<String>,
    /// Only compare columns at least this many blocks inside the golden render's drawn area. Off by default: both
    /// renders read the same world. Use it when they don't (a world that ends leaks sky light 15 blocks sideways)
    #[arg(long)]
    inset: Option<i32>,
    /// Textures and cells to list
    #[arg(long, default_value_t = 15)]
    top: usize,
    /// Also write the report as JSON
    #[arg(long)]
    json: Option<PathBuf>,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::DiffRender(a) => diff_render(a),
        Command::Roundtrip { webroot, map } => roundtrip(&webroot, map.as_deref()),
    }
}

fn roundtrip(webroot: &std::path::Path, map: Option<&str>) -> Result<()> {
    let map = WebrootMap::open(webroot, map)?;
    let (mut out, mut mismatched, mut tiles) = (Vec::new(), 0, 0);
    for t in map.tiles(0)? {
        let original = map.tile_bytes(0, t)?;
        bm_golden::roundtrip::to_model(&bm_golden::parse(&original)?).write_prbm(&mut out)?;
        tiles += 1;
        if out != original {
            mismatched += 1;
            let at = out.iter().zip(&original).position(|(a, b)| a != b).unwrap_or(out.len().min(original.len()));
            eprintln!("hires {t:?}: {} vs {} bytes, first difference at byte {at}", out.len(), original.len());
        }
    }
    println!("hires: {tiles} tiles re-encoded, {mismatched} differ");

    let size = map.settings.lowres.tile_size.map(|s| s as usize);
    let (mut lowres, mut lowres_bad, mut java_bytes, mut our_bytes) = (0, 0, 0, 0);
    for lod in 1..=map.settings.lowres.lod_count {
        for t in map.tiles(lod)? {
            let original = map.tile_bytes(lod, t)?;
            let tile = bm_format::lowres::LowresTile::decode_png(&original, size)?;
            tile.encode_png(&mut out)?;
            lowres += 1;
            java_bytes += original.len();
            our_bytes += out.len();
            if bm_format::lowres::LowresTile::decode_png(&out, size)? != tile {
                lowres_bad += 1;
                eprintln!("lowres lod {lod} {t:?}: pixels differ after re-encoding");
            }
        }
    }
    println!("lowres: {lowres} tiles re-encoded, {lowres_bad} differ; PNG bytes java {java_bytes}, ours {our_bytes}");
    ensure!(mismatched == 0 && lowres_bad == 0, "re-encoded output differs from BlueMap's");
    Ok(())
}

fn texture_names(map: &WebrootMap) -> Result<Vec<String>> {
    Ok(bm_golden::parse_textures(&map.textures_json()?)?.into_iter().map(|t| t.resource_path).collect())
}

fn hires_tile(map: &WebrootMap, present: &BTreeSet<TilePos>, t: TilePos) -> Result<Tile> {
    if present.contains(&t) { bm_golden::parse(&map.tile_bytes(0, t)?) } else { Ok(Tile::default()) }
}

fn drawn_columns(map: &WebrootMap, tiles: &BTreeSet<TilePos>) -> Result<HashSet<(i32, i32)>> {
    let mut columns = HashSet::new();
    for &t in tiles {
        let origin = map.hires_origin(t);
        columns.extend(bm_golden::parse(&map.tile_bytes(0, t)?)?.faces().map(|f| {
            let [x, _, z] = face_cell(&f, origin);
            (x, z)
        }));
    }
    Ok(columns)
}

/// Columns whose whole (2n+1)² neighbourhood is in `columns`.
fn inset(columns: HashSet<(i32, i32)>, n: i32) -> HashSet<(i32, i32)> {
    let inside = |&(x, z): &(i32, i32)| (-n..=n).all(|dx| (-n..=n).all(|dz| columns.contains(&(x + dx, z + dz))));
    columns.iter().copied().filter(inside).collect()
}

fn diff_render(a: DiffArgs) -> Result<()> {
    let golden = WebrootMap::open(&a.golden, a.map.as_deref())?;
    let candidate = WebrootMap::open(&a.candidate, a.map.as_deref())?;
    ensure!(golden.settings.hires_grid() == candidate.settings.hires_grid(), "the webroots use different hires grids");
    let (golden_names, candidate_names) = (texture_names(&golden)?, texture_names(&candidate)?);
    let golden_tiles = golden.tiles(0)?;
    let candidate_tiles = candidate.tiles(0)?;

    let columns = match a.inset {
        Some(n) => Some(inset(drawn_columns(&golden, &golden_tiles)?, n)),
        None => None,
    };
    let unrendered = |x, z| columns.as_ref().is_some_and(|c| !c.contains(&(x, z)));
    let mut diff = RenderDiff::default();
    for &t in golden_tiles.union(&candidate_tiles) {
        diff.add_tile(
            &hires_tile(&golden, &golden_tiles, t)?,
            &golden_names,
            &hires_tile(&candidate, &candidate_tiles, t)?,
            &candidate_names,
            golden.hires_origin(t),
            unrendered,
        );
    }
    let report = diff.finish(a.top);
    print!("{report}");
    if let Some(path) = a.json {
        std::fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}
