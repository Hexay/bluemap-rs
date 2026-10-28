//! Model bodies of stored compat tiles for the WASM unpack benchmark (`crates/bm-wasm/bench/`), and the native
//! unpack time of the same bodies as its baseline.
//!
//! Usage: `cargo run --release -p bm-wasm --example corpus -- <dir with x…/z….prbm.gz> <out dir> [--step 1]
//! [--reps 5]`. Writes `<n>.body` per model-mode tile, its stored zstd frame as `<n>.frame`, and `manifest.json` (`[{name, quads, prbm, fnv}]`, fnv =
//! FNV-1a 32 of the PRBM). Tile origins assume the default hires grid (32-block tiles offset by 2).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bm_format::compact::{BodyUnpacker, CompactCodec};
use bm_format::grid::{Grid, parse_tile_path};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable directory").flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.to_string_lossy().ends_with(".prbm.gz") {
            out.push(path);
        }
    }
}

fn fnv(data: &[u8]) -> u32 {
    data.iter().fold(0x811c_9dc5, |h, &b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag =
        |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
    let (step, reps): (usize, usize) = (flag("--step").unwrap_or(1), flag("--reps").unwrap_or(5));
    let root = PathBuf::from(&args[0]);
    let out_dir = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out_dir).expect("output directory");
    let grid = Grid { size: [32; 2], offset: [2; 2] };
    let mut paths = Vec::new();
    collect(&root, &mut paths);
    paths.sort();

    let mut codec = CompactCodec::default();
    let mut unpacker = BodyUnpacker::default();
    let (mut prbm, mut body, mut out, mut blob) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut quads, mut native, mut entries, mut body_bytes) = (0usize, Duration::ZERO, Vec::new(), 0usize);
    for path in paths.iter().step_by(step) {
        prbm.clear();
        flate2::read::GzDecoder::new(&std::fs::read(path).expect("tile")[..]).read_to_end(&mut prbm).expect("gzip");
        let tile = path.strip_prefix(&root).ok().and_then(|rel| parse_tile_path(&rel.to_string_lossy()));
        if !codec.body_into(&prbm, tile.map(|t| grid.tile_min(t).into()), &mut body) {
            continue;
        }
        let best = (0..reps)
            .map(|_| {
                let start = Instant::now();
                unpacker.unpack_into(&body, &mut out).expect("unpack");
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        assert!(out == prbm, "round trip differs: {}", path.display());
        let n = u32::from_le_bytes([prbm[2], prbm[3], prbm[4], 0]) as usize / 6;
        let name = format!("{}.body", entries.len());
        std::fs::write(out_dir.join(&name), &body).expect("write body");
        codec.encode_into(&prbm, tile.map(|t| grid.tile_min(t).into()), &mut blob).expect("encode");
        let frame = bm_format::compact::model_frame(&blob).expect("model-mode blob");
        std::fs::write(out_dir.join(format!("{}.frame", entries.len())), frame).expect("write frame");
        entries.push(format!(r#"{{"name":"{name}","quads":{n},"prbm":{},"fnv":{}}}"#, prbm.len(), fnv(&prbm)));
        (quads, native, body_bytes) = (quads + n, native + best, body_bytes + body.len());
    }
    let mut manifest = std::fs::File::create(out_dir.join("manifest.json")).expect("manifest");
    writeln!(manifest, "[{}]", entries.join(",\n")).expect("write manifest");
    println!(
        "{} bodies, {:.2} M quads, bodies {:.1} MB | native unpack {:.1} ns/quad, {:.3} ms/tile",
        entries.len(),
        quads as f64 / 1e6,
        body_bytes as f64 / 1e6,
        native.as_nanos() as f64 / quads as f64,
        native.as_secs_f64() * 1e3 / entries.len() as f64
    );
}
