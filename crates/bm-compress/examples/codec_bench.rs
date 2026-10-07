//! Single-thread codec sizes and speeds on real hires tiles (raw PRBM from a webroot's `.prbm.gz` files).
//! Usage: cargo run -p bm-compress --profile profiling --example codec_bench -- <dir> [--every N] [--slow-every M]
//!        [--reps R] [--only name,name]
//!   `--every N` keeps every Nth tile (sorted paths); slow codecs run on every Mth kept tile only. Each tile's encode
//!   time is the minimum of R runs, which filters out noise from other processes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bm_compress::Compression;
use zstd::stream::raw::CParameter;

type Enc = Box<dyn FnMut(&[u8], &mut Vec<u8>)>;

enum Dec {
    None,
    Product(Compression),
    ZstdBulk,
}

struct Codec {
    name: String,
    slow: bool,
    dec: Dec,
    enc: Enc,
}

fn gz(level: u32) -> Enc {
    Box::new(move |data, out| {
        out.clear();
        let mut e = flate2::write::GzEncoder::new(&mut *out, flate2::Compression::new(level));
        e.write_all(data).unwrap();
        e.finish().unwrap();
    })
}

/// One reused context, like a per-thread compressor would be; `window_log` enables long-distance matching.
fn zstd_ctx(level: i32, window_log: Option<u32>) -> Enc {
    let mut c = zstd::bulk::Compressor::new(level).unwrap();
    if let Some(w) = window_log {
        c.set_parameter(CParameter::EnableLongDistanceMatching(true)).unwrap();
        c.set_parameter(CParameter::WindowLog(w)).unwrap();
    }
    Box::new(move |data, out| {
        out.clear();
        out.reserve(zstd::zstd_safe::compress_bound(data.len()));
        c.compress_to_buffer(data, out).unwrap();
    })
}

fn product(c: Compression) -> Enc {
    Box::new(move |d, o| c.compress_into(d, o).unwrap())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.to_string_lossy().ends_with(".prbm.gz") {
            out.push(p);
        }
    }
}

fn codecs() -> Vec<Codec> {
    let c = |name: &str, slow: bool, dec: Dec, enc: Enc| Codec { name: name.into(), slow, dec, enc };
    let mut v = vec![
        c("gzip-1", false, Dec::None, gz(1)),
        c("gzip-3", false, Dec::None, gz(3)),
        c("gzip-4", false, Dec::None, gz(4)),
        c("gzip-5", false, Dec::None, gz(5)),
        c("gzip-6 (product Gzip)", false, Dec::Product(Compression::Gzip), product(Compression::Gzip)),
        c("gzip-9", true, Dec::None, gz(9)),
        c("deflate-6 (product)", false, Dec::Product(Compression::Deflate), product(Compression::Deflate)),
        c("lz4-java blocks (product)", false, Dec::Product(Compression::Lz4), product(Compression::Lz4)),
        c("zstd-3 (product, stream)", false, Dec::Product(Compression::Zstd), product(Compression::Zstd)),
    ];
    for level in [1, 3, 6, 9, 12, 15, 16, 17, 18, 19] {
        v.push(c(&format!("zstd-{level} ctx"), level >= 15, Dec::ZstdBulk, zstd_ctx(level, None)));
    }
    for (level, w) in [(1, 24), (3, 24), (3, 27), (6, 24), (9, 24), (12, 24), (19, 24)] {
        v.push(c(&format!("zstd-{level} ctx + LDM w{w}"), level >= 15, Dec::ZstdBulk, zstd_ctx(level, Some(w))));
    }
    v
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |k: &str| args.iter().position(|a| a == k).map(|i| args[i + 1].clone());
    let num = |k: &str, d: usize| flag(k).map_or(d, |v| v.parse().unwrap());
    let (every, slow_every, reps) = (num("--every", 1), num("--slow-every", 4), num("--reps", 1));
    let only: Vec<String> = flag("--only").map_or(Vec::new(), |s| s.split(',').map(str::to_owned).collect());
    let mut files = Vec::new();
    walk(Path::new(&args[0]), &mut files);
    files.sort();
    let tiles: Vec<(usize, Vec<u8>)> = files
        .iter()
        .step_by(every)
        .map(|f| {
            let stored = std::fs::read(f).unwrap();
            (stored.len(), Compression::Gzip.decompress(&stored, 1 << 30).unwrap())
        })
        .collect();
    let stored: usize = tiles.iter().map(|t| t.0).sum();
    let raw: usize = tiles.iter().map(|t| t.1.len()).sum();
    println!("{} tiles, stored (Java gzip) {stored} B, raw PRBM {raw} B ({:.1}x)", tiles.len(), raw as f64 / stored as f64);
    println!("| codec | tiles | bytes | vs Java gz | raw/x | enc ms/tile | enc MB/s | dec ms/tile |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut out = Vec::new();
    let mut back = Vec::new();
    for mut c in codecs() {
        if !only.is_empty() && !only.iter().any(|o| c.name.starts_with(o.as_str())) {
            continue;
        }
        let step = if c.slow { slow_every } else { 1 };
        let (mut bytes, mut base, mut raw_n, mut n) = (0usize, 0usize, 0usize, 0usize);
        let (mut enc_ns, mut dec_ns) = (0u128, 0u128);
        for (st, data) in tiles.iter().step_by(step) {
            let mut best = u128::MAX;
            for _ in 0..reps {
                let t = Instant::now();
                (c.enc)(data, &mut out);
                best = best.min(t.elapsed().as_nanos());
            }
            enc_ns += best;
            let t = Instant::now();
            match c.dec {
                Dec::None => {}
                Dec::Product(d) => d.decompress_into(&out, 1 << 30, &mut back).unwrap(),
                Dec::ZstdBulk => {
                    back.resize(data.len(), 0);
                    let n = zstd::bulk::decompress_to_buffer(&out, &mut back[..]).unwrap();
                    back.truncate(n);
                }
            }
            dec_ns += t.elapsed().as_nanos();
            if !matches!(c.dec, Dec::None) {
                assert_eq!(&back, data);
            }
            bytes += out.len();
            base += st;
            raw_n += data.len();
            n += 1;
        }
        let ms = |ns: u128| ns as f64 / 1e6 / n as f64;
        let dec = if matches!(c.dec, Dec::None) { "-".into() } else { format!("{:.2}", ms(dec_ns)) };
        println!(
            "| {} | {n} | {bytes} | {:.3} | {:.1} | {:.2} | {:.0} | {dec} |",
            c.name,
            bytes as f64 / base as f64,
            raw_n as f64 / bytes as f64,
            ms(enc_ns),
            raw_n as f64 / 1e6 / (enc_ns as f64 / 1e9),
        );
    }
}
