//! The compact codec on stored compat tiles: size, bit-exact round trip and single-thread encode/decode time.
//!
//! Usage: `cargo run --release -p bm-format --example compact_bench -- <dir with x…/z….prbm.gz> [--level 9]
//! [--reps 3] [--step 1] [--no-origin] [--streams]`. Tile coordinates come from the path; the world origin assumes
//! the default hires grid (32-block tiles offset by 2). Times are the fastest of `reps` runs per tile, summed. The
//! last line hashes every blob: it must not change when only the codec's speed does.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bm_format::compact::{BODY_HEADER, CompactCodec, STREAMS};
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

fn fastest(reps: usize, mut f: impl FnMut()) -> Duration {
    (0..reps)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag =
        |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
    let (level, reps, step) = (flag("--level").unwrap_or(9), flag("--reps").unwrap_or(3), flag("--step").unwrap_or(1));
    let use_origin = !args.iter().any(|a| a == "--no-origin");
    let streams = args.iter().any(|a| a == "--streams");
    let mut per_stream = [[0usize; 2]; STREAMS.len()];
    let root = PathBuf::from(args.first().expect("corpus directory"));
    let grid = Grid { size: [32; 2], offset: [2; 2] };
    let mut paths = Vec::new();
    collect(&root, &mut paths);
    paths.sort();

    let mut codec = CompactCodec::new(level as i32);
    let (mut encode, mut decode) = (Duration::ZERO, Duration::ZERO);
    let (mut prbm, mut blob, mut out) = (Vec::new(), Vec::new(), Vec::new());
    let mut blobs = DefaultHasher::new();
    let (mut tiles, mut raw_mode, mut quads, mut raw, mut gz, mut bytes) = (0, 0, 0, 0, 0, 0);
    for path in paths.iter().step_by(step) {
        let stored = std::fs::read(path).expect("readable tile");
        prbm.clear();
        flate2::read::GzDecoder::new(&stored[..]).read_to_end(&mut prbm).expect("gzip tile");
        let tile = path.strip_prefix(&root).ok().and_then(|rel| parse_tile_path(&rel.to_string_lossy()));
        let origin = tile.filter(|_| use_origin).map(|t| grid.tile_min(t).into());
        tiles += 1;
        gz += stored.len();
        raw += prbm.len();
        quads += u32::from_le_bytes([prbm[2], prbm[3], prbm[4], 0]) as usize / 6;

        encode += fastest(reps, || codec.encode_into(&prbm, origin, &mut blob).expect("encode"));
        bytes += blob.len();
        blob.hash(&mut blobs);
        decode += fastest(reps, || codec.decode_into(&blob, &mut out).expect("decode"));
        assert!(out == prbm, "round trip differs: {}", path.display());
        // an empty tile has nothing to model
        raw_mode += usize::from(blob[4] == 1 && prbm[2..5] != [0; 3]);
        if streams && codec.body_into(&prbm, origin, &mut out) {
            let mut at = BODY_HEADER;
            for size in &mut per_stream {
                let len = u32::from_le_bytes(out[at..at + 4].try_into().unwrap()) as usize;
                size[0] += len;
                size[1] += zstd::bulk::compress(&out[at + 4..at + 4 + len], 19).expect("zstd").len();
                at += 4 + len;
            }
        }
    }
    if streams {
        let total: usize = per_stream.iter().map(|s| s[1]).sum();
        println!("streams compressed alone (zstd 19):");
        for (name, [raw, packed]) in STREAMS.iter().zip(per_stream) {
            let share = 100.0 * packed as f64 / total as f64;
            let bits = 8.0 * packed as f64 / quads as f64;
            println!(
                "   {name:14} raw {:8.3} MB  packed {:7.3} MB  {share:5.1}%  {bits:5.2} b/quad",
                raw as f64 / 1e6,
                packed as f64 / 1e6
            );
        }
    }

    let rate = |d: Duration| (raw as f64 / 1e6 / d.as_secs_f64(), d.as_nanos() as f64 / quads as f64);
    let ((enc_mb, enc_ns), (dec_mb, dec_ns)) = (rate(encode), rate(decode));
    println!(
        "{tiles} tiles ({raw_mode} non-empty in raw mode), {:.2} M quads, PRBM {:.1} MB, stored gzip {:.1} MB; zstd level {level}",
        quads as f64 / 1e6,
        raw as f64 / 1e6,
        gz as f64 / 1e6
    );
    println!(
        "{:8.3} MB (gzip/{:.2}) | encode {enc_mb:6.0} MB/s {enc_ns:6.1} ns/quad | decode {dec_mb:6.0} MB/s {dec_ns:6.1} ns/quad",
        bytes as f64 / 1e6,
        gz as f64 / bytes as f64,
    );
    println!("blobs {:016x}", blobs.finish());
}
