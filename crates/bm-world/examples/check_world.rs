//! Decodes every chunk of a world's dimensions and reports errors, unknown states and decode speed. With
//! `--blocks-json` (a vanilla data report), also checks every interned state against the game's own list.
//! Usage: cargo run -p bm-world --release --example check_world -- <world> [--blocks-json <path>]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bm_world::chunk::{Chunk, ChunkContext};
use bm_world::dimension::{DimensionType, dimension_folder, load_dimension_type};
use bm_world::region::{Region, list_regions};
use bm_world::{Biomes, BlockStates, StateId};

const DEFAULTS: &str = include_str!("../../../assets/resourceExtensions/data/minecraft/defaultBlockstates.json");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let world = PathBuf::from(args.first().expect("usage: check_world <world> [--blocks-json <path>]"));
    let blocks_json = args.iter().position(|a| a == "--blocks-json").map(|i| PathBuf::from(&args[i + 1]));

    let states = BlockStates::default();
    states.add_defaults_json(DEFAULTS).unwrap();
    let biomes = Biomes::default();
    let mut failed = false;
    for dimension in dimensions(&world) {
        let dim_type = load_dimension_type(&world, &dimension, &DimensionType::builtin).unwrap();
        failed |= check_dimension(&world, &dimension, &dim_type, &states, &biomes);
    }
    if let Some(path) = blocks_json {
        failed |= compare_with_report(&states, &path);
    }
    std::process::exit(i32::from(failed));
}

fn dimensions(world: &Path) -> Vec<String> {
    let mut dims: BTreeSet<String> =
        ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"].map(String::from).into();
    for ns in std::fs::read_dir(world.join("dimensions")).into_iter().flatten().flatten() {
        for dim in std::fs::read_dir(ns.path()).into_iter().flatten().flatten() {
            dims.insert(format!("{}:{}", ns.file_name().to_string_lossy(), dim.file_name().to_string_lossy()));
        }
    }
    dims.into_iter().collect()
}

fn check_dimension(world: &Path, dimension: &str, ty: &DimensionType, states: &BlockStates, biomes: &Biomes) -> bool {
    let dir = dimension_folder(world, dimension).join("region");
    let regions = list_regions(&dir).unwrap();
    if regions.is_empty() {
        return false;
    }
    let ctx = ChunkContext { states, biomes, dimension: ty };
    let (mut raw, mut nbt) = (Vec::new(), Vec::new());
    let (mut chunks, mut bytes, mut errors, mut missing, mut unlit) = (0, 0, 0, 0, 0);
    let start = Instant::now();
    for (rx, rz) in regions {
        let region = Region::open(&dir, rx, rz).unwrap();
        for (lx, lz) in (0..32).flat_map(|x| (0..32).map(move |z| (x, z))) {
            match region.read_chunk_into(lx, lz, &mut raw, &mut nbt).map(|found| found.then(|| Chunk::parse(&nbt, &ctx))) {
                Ok(None) => continue,
                Ok(Some(Ok(chunk))) => {
                    chunks += 1;
                    bytes += nbt.len();
                    unlit += usize::from(chunk.generated && !chunk.has_light);
                    missing += count_missing(&chunk);
                }
                Ok(Some(Err(e))) | Err(e) => {
                    errors += 1;
                    eprintln!("{dimension} r.{rx}.{rz} chunk {lx},{lz}: {e}");
                }
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{dimension:24} {chunks:6} chunks  {:6.1} MB nbt  {:6.0} chunks/s  errors {errors}  unlit {unlit}  missing-state blocks {missing}",
        bytes as f64 / 1e6,
        chunks as f64 / secs
    );
    errors > 0 || missing > 0
}

fn count_missing(chunk: &Chunk) -> usize {
    (chunk.min_y()..=chunk.max_y())
        .step_by(16)
        .map(|y| (0..4096).filter(|&i| chunk.block(i & 15, y + (i >> 8), (i >> 4) & 15) == StateId::MISSING).count())
        .sum()
}

/// Every state the world produced must be a real vanilla state; reports how many vanilla states were seen.
fn compare_with_report(states: &BlockStates, path: &Path) -> bool {
    let seen: Vec<String> = (2..states.len() as u32).map(|i| states.get(StateId(i)).key.to_string()).collect();
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let (keys, mut vanilla) = (BlockStates::default(), BTreeSet::new());
    for (name, block) in report.as_object().unwrap() {
        for state in block["states"].as_array().unwrap() {
            let mut props: Vec<(&str, &str)> = state
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|p| p.iter().map(|(k, v)| (k.as_str(), v.as_str().unwrap())).collect())
                .unwrap_or_default();
            vanilla.insert(keys.get(keys.intern(name, &mut props)).key.to_string());
        }
    }
    let unknown: Vec<&String> = seen.iter().filter(|k| !vanilla.contains(*k)).collect();
    let seen_set: BTreeSet<&String> = seen.iter().collect();
    let absent_keys: Vec<&String> = vanilla.iter().filter(|k| !seen_set.contains(k)).collect();
    let absent = absent_keys.len();
    if absent <= 5 {
        absent_keys.iter().for_each(|k| println!("  never seen: {k}"));
    }
    println!(
        "vanilla report: {} states; registry (world + default table) {} distinct, {} not in the report, {absent} vanilla states never seen",
        vanilla.len(),
        seen.len(),
        unknown.len()
    );
    for key in unknown.iter().take(20) {
        println!("  not vanilla: {key}");
    }
    !unknown.is_empty()
}
