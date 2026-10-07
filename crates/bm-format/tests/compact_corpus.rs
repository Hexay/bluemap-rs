//! BMQ2 over every Java-rendered hires tile in `work/bluemap/*/web/maps/*/tiles/0`: decoded bytes must equal the
//! decompressed PRBM. Prints sizes vs the stored gzip and timings. `BMQ_LEVELS=3,9,19` sweeps zstd levels.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bm_format::compact::{CompactCodec, DEFAULT_LEVEL};
use rayon::prelude::*;

fn tiles(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            tiles(&p, out);
        } else if p.to_string_lossy().ends_with(".prbm.gz") {
            out.push(p);
        }
    }
}

fn corpus() -> Vec<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let work = here.ancestors().map(|a| a.join("work/bluemap")).find(|p| p.is_dir()).expect("work/bluemap");
    let mut out = Vec::new();
    for fixture in std::fs::read_dir(work).unwrap().flatten() {
        for map in std::fs::read_dir(fixture.path().join("web/maps")).into_iter().flatten().flatten() {
            tiles(&map.path().join("tiles/0"), &mut out);
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct Totals {
    gz: AtomicU64,
    raw: AtomicU64,
    blob: AtomicU64,
    raw_mode: AtomicU64,
    encode_ns: AtomicU64,
    decode_ns: AtomicU64,
}

#[test]
#[ignore = "needs Java BlueMap golden renders under work/bluemap"]
fn every_golden_tile_round_trips() {
    let files = corpus();
    assert!(!files.is_empty());
    let levels: Vec<i32> = std::env::var("BMQ_LEVELS")
        .map(|s| s.split(',').map(|l| l.trim().parse().unwrap()).collect())
        .unwrap_or_else(|_| vec![DEFAULT_LEVEL]);
    for level in levels {
        let t = Totals::default();
        files.par_iter().for_each_init(
            || (CompactCodec::new(level), Vec::new(), Vec::new()),
            |(codec, blob, back), path| {
                let gz = std::fs::read(path).unwrap();
                let mut raw = Vec::new();
                flate2::read::MultiGzDecoder::new(&gz[..]).read_to_end(&mut raw).unwrap();
                let start = Instant::now();
                codec.encode_into(&raw, blob).unwrap();
                let mid = Instant::now();
                codec.decode_into(blob, back).unwrap();
                t.decode_ns.fetch_add(mid.elapsed().as_nanos() as u64, Ordering::Relaxed);
                t.encode_ns.fetch_add((mid - start).as_nanos() as u64, Ordering::Relaxed);
                assert!(*back == raw, "{} does not round-trip", path.display());
                t.gz.fetch_add(gz.len() as u64, Ordering::Relaxed);
                t.raw.fetch_add(raw.len() as u64, Ordering::Relaxed);
                t.blob.fetch_add(blob.len() as u64, Ordering::Relaxed);
                t.raw_mode.fetch_add(u64::from(blob[4] == 1), Ordering::Relaxed);
            },
        );
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let n = files.len() as u64;
        println!(
            "level {level}: {n} tiles ({} raw mode), gzip {:.2} MB, raw PRBM {:.1} MB, BMQ2 {:.2} MB = {:.3}x gzip \
             ({:.1}x smaller); encode {:.2} ms/tile (incl. verify), decode {:.2} ms/tile",
            get(&t.raw_mode),
            get(&t.gz) as f64 / 1e6,
            get(&t.raw) as f64 / 1e6,
            get(&t.blob) as f64 / 1e6,
            get(&t.blob) as f64 / get(&t.gz) as f64,
            get(&t.gz) as f64 / get(&t.blob) as f64,
            get(&t.encode_ns) as f64 / n as f64 / 1e6,
            get(&t.decode_ns) as f64 / n as f64 / 1e6,
        );
    }
}
