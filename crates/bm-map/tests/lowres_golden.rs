//! Feeds every written LOD 1 column of Java BlueMap's golden lowres tiles through the layer, then compares every
//! LOD 1..n pixel it produces (seams and seam-only tiles included) with the golden PNGs.
//! `cargo test -p bm-map --test lowres_golden -- --ignored --nocapture`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bm_format::grid::{Tile, parse_tile_path};
use bm_format::lowres::LowresTile;
use bm_map::lowres::{LowresTileManager, MemoryStore};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const FIXTURES: [&str; 5] = ["vanilla", "vanilla-512", "structures", "nether", "debug"];
const CHANNELS: [&str; 7] = ["a", "r", "g", "b", "height", "light", "meta-a"];

fn bluemap_work() -> PathBuf {
    let root = Path::new(ROOT);
    // `work/` is git-ignored, so a worktree finds it in an enclosing checkout
    ["work", "../../../work"].iter().map(|w| root.join(w).join("bluemap")).find(|w| w.is_dir()).expect("work/bluemap")
}

fn png_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            png_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}

fn golden_tiles(lod_dir: &Path, size: [usize; 2]) -> BTreeMap<Tile, LowresTile> {
    let mut files = Vec::new();
    png_files(lod_dir, &mut files);
    files
        .iter()
        .map(|f| {
            let rel = f.strip_prefix(lod_dir).unwrap().to_string_lossy().replace('\\', "/");
            let tile = parse_tile_path(&rel).unwrap_or_else(|| panic!("tile path {rel}"));
            (tile, LowresTile::decode_png(&fs::read(f).unwrap(), size).unwrap())
        })
        .collect()
}

/// Mismatching pixels per channel of [`CHANNELS`].
fn diff(a: &LowresTile, b: &LowresTile, size: [usize; 2], counts: &mut [u64; 7]) {
    for z in 0..=size[1] {
        for x in 0..=size[0] {
            let (ca, cb, ma, mb) = (a.color(x, z), b.color(x, z), a.meta(x, z), b.meta(x, z));
            for (i, shift) in [24, 16, 8, 0].into_iter().enumerate() {
                counts[i] += u64::from((ca >> shift) & 0xFF != (cb >> shift) & 0xFF);
            }
            counts[4] += u64::from(ma & 0xFFFF != mb & 0xFFFF);
            counts[5] += u64::from((ma >> 16) & 0xFF != (mb >> 16) & 0xFF);
            counts[6] += u64::from(ma >> 24 != mb >> 24);
        }
    }
}

fn check_fixture(fixture: &str) -> u64 {
    let maps = bluemap_work().join(fixture).join("web/maps");
    let map = fs::read_dir(&maps).unwrap().flatten().map(|e| e.path()).find(|p| p.is_dir()).expect("a map");
    let settings: serde_json::Value = serde_json::from_slice(&fs::read(map.join("settings.json")).unwrap()).unwrap();
    let lowres = &settings["lowres"];
    let ts = [lowres["tileSize"][0].as_i64().unwrap() as i32, lowres["tileSize"][1].as_i64().unwrap() as i32];
    let (lod_count, lod_factor) =
        (lowres["lodCount"].as_u64().unwrap() as u32, lowres["lodFactor"].as_i64().unwrap() as i32);
    let size = [ts[0] as usize, ts[1] as usize];

    let goldens: Vec<_> = (1..=lod_count).map(|lod| golden_tiles(&map.join(format!("tiles/{lod}")), size)).collect();
    let mut layer = LowresTileManager::new(MemoryStore::default(), ts, lod_count, lod_factor);
    let mut columns = 0u64;
    for (&(tx, tz), tile) in &goldens[0] {
        for z in 0..size[1] {
            for x in 0..size[0] {
                if tile.meta(x, z) >> 24 == 0 {
                    continue;
                }
                columns += 1;
                let (wx, wz) = (tx * ts[0] + x as i32, tz * ts[1] + z as i32);
                layer.set_argb(wx, wz, tile.color(x, z), tile.height(x, z), i32::from(tile.block_light(x, z))).unwrap();
            }
        }
    }
    layer.flush().unwrap();
    let store = layer.into_store();
    let produced_tiles = store.tiles.len();

    let mut total = 0;
    for (i, golden) in goldens.iter().enumerate() {
        let lod = i as u32 + 1;
        let mut counts = [0u64; 7];
        let ours: BTreeMap<Tile, &LowresTile> =
            store.tiles.iter().filter(|((l, _), _)| *l == lod).map(|((_, t), d)| (*t, d)).collect();
        let missing: Vec<_> = golden.keys().filter(|t| !ours.contains_key(t)).collect();
        let extra: Vec<_> = ours.keys().filter(|t| !golden.contains_key(t)).collect();
        for (tile, data) in golden {
            if let Some(o) = ours.get(tile) {
                diff(data, o, size, &mut counts);
            }
        }
        let bad: u64 = counts.iter().sum::<u64>() + (missing.len() + extra.len()) as u64;
        total += bad;
        let detail: Vec<_> =
            CHANNELS.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(c, n)| format!("{c}={n}")).collect();
        println!(
            "{fixture} lod{lod}: {} tiles, missing {missing:?}, extra {extra:?}, channel mismatches [{}]",
            golden.len(),
            detail.join(" ")
        );
    }
    assert_eq!(store.saves, produced_tiles, "{fixture}: every tile saved exactly once");
    println!("{fixture}: {columns} columns fed, {} saves", store.saves);
    total
}

#[test]
#[ignore = "needs Java BlueMap golden webroots under work/bluemap/<fixture>/web"]
fn lowres_layers_match_java_bluemap() {
    let failures: u64 = FIXTURES.iter().map(|f| check_fixture(f)).sum();
    assert_eq!(failures, 0, "lowres pixel mismatches");
}
