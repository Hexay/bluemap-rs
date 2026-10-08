//! Re-renders golden fixtures (Java BlueMap 5.28 output under `work/`) and requires byte-identical PRBM.
//! Skipped when the fixtures or the client jar are absent (`py -3 tools/render_golden.py` makes them).

#[path = "../examples/render_map/conf.rs"]
mod conf;
#[path = "../examples/render_map/fixture.rs"]
#[allow(dead_code)]
mod fixture;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use fixture::Fixture;

fn check(fx: &str, dimension: &str) {
    let work = fixture::work_dir();
    let world = work.join(format!("worlds/{fx}/world"));
    let golden = work.join(format!("bluemap/{fx}/web"));
    let jar = fixture::client_jar();
    if !world.is_dir() || !golden.is_dir() || !jar.is_file() {
        eprintln!("skipping {fx}: fixture not found");
        return;
    }
    let f = Fixture::load(&world, dimension, &golden, None, None, &jar, &fixture::extensions_dir())
        .unwrap();
    let tiles = f.golden_tiles().unwrap();
    let area = f.load_area(&tiles);
    let rendered = f.render(&area, &tiles, &AtomicU64::new(0)).unwrap();
    let differing: Vec<_> =
        rendered.iter().filter(|r| f.golden.tile_bytes(0, r.tile).unwrap() != r.prbm).map(|r| r.tile).collect();
    assert!(
        differing.is_empty(),
        "{fx}: {} of {} tiles differ, e.g. {:?}",
        differing.len(),
        tiles.len(),
        &differing[..differing.len().min(5)]
    );
    let (checked, bad) = f.check_lowres(&rendered).unwrap();
    assert!(
        bad.is_empty(),
        "{fx}: {} of {checked} lowres columns differ:\n{}",
        bad.len(),
        bad[..bad.len().min(10)].join("\n")
    );
}

#[test]
fn superflat() {
    check("superflat", "minecraft:overworld");
}

#[test]
fn vanilla() {
    check("vanilla", "minecraft:overworld");
}

#[test]
fn context() {
    check("context", "minecraft:overworld");
}

#[test]
fn debug() {
    check("debug", "minecraft:overworld");
}

#[test]
fn biomes() {
    check("biomes", "minecraft:overworld");
}

#[test]
fn nether() {
    check("nether", "minecraft:the_nether");
}
