//! Native unpack time of a corpus from the `corpus` example: the baseline for `bench/node.mjs` and `browser.mjs`.
//!
//! Usage: `cargo run --release -p bm-wasm --example unpack_bench -- <corpus dir> [--reps 5]`. Fastest of `reps`
//! passes over the corpus; every PRBM is checked against the manifest's FNV-1a first.

use std::path::PathBuf;
use std::time::Instant;

use bm_format::compact::BodyUnpacker;

fn fnv(data: &[u8]) -> u32 {
    data.iter().fold(0x811c_9dc5, |h, &b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// `"key":<number>` of one manifest entry.
fn field(entry: &str, key: &str) -> u64 {
    let at = entry.find(&format!("\"{key}\":")).expect("manifest field") + key.len() + 3;
    entry[at..].split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()).expect("number")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(&args[0]);
    let reps: usize = args.iter().position(|a| a == "--reps").and_then(|i| args[i + 1].parse().ok()).unwrap_or(5);
    let manifest = std::fs::read_to_string(dir.join("manifest.json")).expect("manifest");
    let tiles: Vec<(Vec<u8>, u64, u32)> = manifest
        .split('{')
        .skip(1)
        .enumerate()
        .map(|(i, e)| {
            (std::fs::read(dir.join(format!("{i}.body"))).expect("body"), field(e, "quads"), field(e, "fnv") as u32)
        })
        .collect();
    let mut unpacker = BodyUnpacker::default();
    let mut out = Vec::new();
    for (i, (body, _, want)) in tiles.iter().enumerate() {
        unpacker.unpack_into(body, &mut out).expect("unpack");
        assert_eq!(fnv(&out), *want, "tile {i} differs");
    }
    let best = (0..reps)
        .map(|_| {
            let start = Instant::now();
            for (body, ..) in &tiles {
                unpacker.unpack_into(body, &mut out).expect("unpack");
            }
            start.elapsed()
        })
        .min()
        .unwrap_or_default();
    let quads: u64 = tiles.iter().map(|t| t.1).sum();
    println!(
        "{} tiles, {:.2} M quads: native unpack {:.1} ns/quad, {:.3} ms/tile",
        tiles.len(),
        quads as f64 / 1e6,
        best.as_nanos() as f64 / quads as f64,
        best.as_secs_f64() * 1e3 / tiles.len() as f64
    );
}
