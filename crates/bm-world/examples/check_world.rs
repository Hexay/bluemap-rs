//! Decodes every chunk of a world's dimensions and reports errors, unknown states and decode speed. With
//! `--blocks-json` (the game version's data report), also checks every state found in the world against it.
//! Usage: cargo run -p bm-world --release --example check_world -- <world> [--blocks-json <path>]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bm_world::{Biomes, BlockStates, Chunk, ChunkSlot, DimensionType, StateId, World};

const DEFAULTS: &str = include_str!("../../../assets/resourceExtensions/data/minecraft/defaultBlockstates.json");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let world = PathBuf::from(args.first().expect("usage: check_world <world> [--blocks-json <path>]"));
    let blocks_json = args.iter().position(|a| a == "--blocks-json").map(|i| PathBuf::from(&args[i + 1]));

    let states = Arc::new(BlockStates::default());
    states.add_defaults_json(DEFAULTS).unwrap();
    let biomes = Arc::new(Biomes::default());
    let (mut failed, mut found) = (false, BTreeSet::new());
    for dimension in dimensions(&world) {
        let dim = World::open(&world, &dimension, states.clone(), biomes.clone(), &DimensionType::builtin).unwrap();
        failed |= check_dimension(&dim, &mut found);
    }
    if let Some(path) = blocks_json {
        failed |= compare_with_report(&states, &found, &path);
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

/// Decodes the dimension, adding every state id its blocks use to `found`; true if anything failed.
fn check_dimension(world: &World, found: &mut BTreeSet<StateId>) -> bool {
    let regions = world.regions().unwrap();
    if regions.is_empty() {
        return false;
    }
    let (mut chunks, mut errors, mut missing, mut unlit, mut data_versions) = (0, 0, 0, 0, BTreeSet::new());
    let mut decode = Duration::ZERO;
    for (rx, rz) in regions {
        let start = Instant::now();
        let area = world.load_area(rx * 32, rz * 32, 32, 32);
        decode += start.elapsed();
        for ((cx, cz), slot) in area.slots() {
            match slot {
                ChunkSlot::Absent => {}
                ChunkSlot::Loaded(chunk) => {
                    chunks += 1;
                    unlit += usize::from(chunk.generated && !chunk.has_light);
                    data_versions.insert(chunk.data_version);
                    collect_states(chunk, found);
                    missing += usize::from(found.remove(&StateId::MISSING));
                }
                ChunkSlot::Failed(e) => {
                    errors += 1;
                    eprintln!("{} chunk {cx},{cz}: {e}", world.dimension);
                }
            }
        }
    }
    println!(
        "{:24} {chunks:6} chunks  {:7.0} chunks/s decoded  data versions {data_versions:?}  errors {errors}  unlit {unlit}  chunks with missing states {missing}",
        world.dimension,
        chunks as f64 / decode.as_secs_f64()
    );
    errors > 0 || missing > 0
}

fn collect_states(chunk: &Chunk, found: &mut BTreeSet<StateId>) {
    for y in (chunk.min_y()..=chunk.max_y()).step_by(16) {
        found.extend((0..4096).map(|i| chunk.block(i & 15, y + (i >> 8), (i >> 4) & 15)));
    }
}

/// Every state found in the world must exist in the game's report.
fn compare_with_report(states: &BlockStates, found: &BTreeSet<StateId>, path: &Path) -> bool {
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
    let found_keys: BTreeSet<String> = found.iter().map(|&id| states.get(id).key.to_string()).collect();
    let unknown: Vec<&String> = found_keys.iter().filter(|k| !vanilla.contains(*k)).collect();
    let absent: Vec<&String> = vanilla.iter().filter(|k| !found_keys.contains(*k)).collect();
    println!(
        "report: {} states; world uses {}, {} not in the report, {} report states not in the world",
        vanilla.len(),
        found_keys.len(),
        unknown.len(),
        absent.len()
    );
    unknown.iter().take(20).for_each(|k| println!("  not in report: {k}"));
    if absent.len() <= 5 {
        absent.iter().for_each(|k| println!("  not in world: {k}"));
    }
    !unknown.is_empty()
}
