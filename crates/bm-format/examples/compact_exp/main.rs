//! Size experiments on BMQ3 model bodies (docs/19): zstd levels, block splits, stream layouts and cross-tile
//! prefixes, measured on the bodies the shipped encoder writes. Sizes are zstd frames only (no blob header).
//!
//! Usage: `cargo run --release -p bm-format --example compact_exp -- <dir with x…/z….prbm.gz, or a tree of
//! webroots> [--step N] [--threads 4] [--only name,name] [--time]`. `--time` adds single-thread zstd
//! compress/decompress ns per quad per level.

mod layouts;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bm_format::compact::CompactCodec;
use bm_format::grid::{Grid, Tile, parse_tile_path};
use layouts::Body;
use rayon::prelude::*;
use zstd::zstd_safe::{CCtx, CParameter, DCtx, InBuffer, OutBuffer, ResetDirective, zstd_sys::ZSTD_EndDirective};

/// Tiles of several maps (a fixture webroot tree) are told apart by their hires directory: x is offset by
/// `MAP_STRIDE × map`, so neighbour lookups never cross maps.
const MAP_STRIDE: i32 = 1 << 20;

type Map = HashMap<Tile, Body>;

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

fn load(root: &Path, step: usize) -> Map {
    let grid = Grid { size: [32; 2], offset: [2; 2] };
    let mut paths = Vec::new();
    collect(root, &mut paths);
    paths.sort();
    let split = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        let cut = s.rfind("/tiles/0/").map_or(0, |i| i + "/tiles/0/".len());
        (s[..cut].to_owned(), s[cut..].to_owned())
    };
    let mut maps: Vec<String> = paths.iter().map(|p| split(p).0).collect();
    maps.sort();
    maps.dedup();
    paths
        .par_iter()
        .step_by(step)
        .filter_map(|path| {
            let mut prbm = Vec::new();
            let stored = std::fs::read(path).expect("readable tile");
            flate2::read::GzDecoder::new(&stored[..]).read_to_end(&mut prbm).expect("gzip tile");
            let (map, rel) = split(path);
            let rel = if map.is_empty() { path.strip_prefix(root).ok()?.to_string_lossy().into_owned() } else { rel };
            let tile = parse_tile_path(&rel)?;
            let map = maps.binary_search(&map).unwrap() as i32;
            let mut bytes = Vec::new();
            CompactCodec::default().body_into(&prbm, Some(grid.tile_min(tile).into()), &mut bytes).then_some(())?;
            let quads = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
            Some(((tile.0 + MAP_STRIDE * map, tile.1), Body { quads, bytes }))
        })
        .collect()
}

fn cctx(level: i32, window: u32) -> CCtx<'static> {
    let mut c = CCtx::create();
    c.set_parameter(CParameter::CompressionLevel(level)).unwrap();
    c.set_parameter(CParameter::WindowLog(window)).unwrap();
    c
}

/// One frame of `parts` concatenated; with `flush`, a block ends after every part.
fn frame<'a>(c: &mut CCtx<'a>, parts: &[Vec<u8>], prefix: Option<&'a [u8]>, flush: bool) -> Vec<u8> {
    let total: usize = parts.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(zstd::zstd_safe::compress_bound(total) + 1024 * parts.len());
    c.reset(ResetDirective::SessionOnly).unwrap();
    c.set_pledged_src_size(Some(total as u64)).unwrap();
    if let Some(p) = prefix {
        c.ref_prefix(p).unwrap();
    }
    for (i, part) in parts.iter().enumerate() {
        let op = match (i + 1 == parts.len(), flush) {
            (true, _) => ZSTD_EndDirective::ZSTD_e_end,
            (false, true) => ZSTD_EndDirective::ZSTD_e_flush,
            (false, false) => ZSTD_EndDirective::ZSTD_e_continue,
        };
        let mut input = InBuffer::around(part);
        loop {
            let pos = out.len();
            let left = c.compress_stream2(&mut OutBuffer::around_pos(&mut out, pos), &mut input, op).unwrap();
            if input.pos == part.len() && (left == 0 || op == ZSTD_EndDirective::ZSTD_e_continue) {
                break;
            }
        }
    }
    out
}

enum Kind {
    /// The body as one frame, laid out by the function, optionally with a block per part.
    Layout(fn(&Body) -> Vec<Vec<u8>>, bool),
    /// The shipped body with the first present tile of the function's candidates as zstd prefix.
    Prefix(fn(Tile) -> Vec<Tile>),
}

fn shipped(b: &Body) -> Vec<Vec<u8>> {
    vec![b.bytes.clone()]
}

/// The tile at x - 1, which may itself have a prefix: the size bound of unlimited cross-tile context.
fn left((x, z): Tile) -> Vec<Tile> {
    vec![(x - 1, z)]
}

/// Odd tiles take an even neighbour: one hop for random access.
fn checker((x, z): Tile) -> Vec<Tile> {
    if (x + z) & 1 == 1 { vec![(x - 1, z), (x + 1, z)] } else { vec![] }
}

/// Every tile of a 16×16 bundle takes the bundle's (8, 8) tile.
fn key((x, z): Tile) -> Vec<Tile> {
    let k = ((x & !15) | 8, (z & !15) | 8);
    if k == (x, z) { vec![] } else { vec![k] }
}

fn variants() -> Vec<(&'static str, Kind, i32, u32)> {
    use Kind::*;
    let mut v: Vec<_> = [9, 12, 15, 17, 19, 22]
        .map(|l| (&*format!("z{l}").leak(), Layout(shipped, false), l, 20))
        .into_iter()
        .collect();
    for l in [9, 19] {
        let name = |n: &str| &*format!("{n}{l}").leak();
        v.push((name("flush"), Layout(layouts::cut, true), l, 20));
        v.push((name("inter"), Layout(layouts::interleaved, false), l, 20));
        v.push((name("planes"), Layout(layouts::cell_planes, false), l, 20));
        v.push((name("cells2d"), Layout(layouts::cells_2d, false), l, 20));
        v.push((name("left"), Prefix(left), l, 21));
        v.push((name("checker"), Prefix(checker), l, 21));
        v.push((name("key"), Prefix(key), l, 21));
    }
    v
}

fn size(m: &'static Map, t: Tile, kind: &Kind, c: &mut CCtx<'static>) -> usize {
    match kind {
        Kind::Layout(layout, flush) => frame(c, &layout(&m[&t]), None, *flush).len(),
        Kind::Prefix(candidates) => {
            let prefix = candidates(t).into_iter().find_map(|k| m.get(&k)).map(|b| b.bytes.as_slice());
            frame(c, &shipped(&m[&t]), prefix, false).len()
        }
    }
}

fn time(m: &Map, tiles: &[Tile], level: i32, quads: usize) -> (f64, f64) {
    let (mut c, mut d) = (cctx(level, 20), DCtx::create());
    let (mut enc, mut dec) = (0u128, 0u128);
    for t in tiles {
        let body = &m[t].bytes;
        let start = Instant::now();
        let f = frame(&mut c, std::slice::from_ref(body), None, false);
        enc += start.elapsed().as_nanos();
        let mut out = Vec::with_capacity(body.len());
        let start = Instant::now();
        d.decompress(&mut out, &f).unwrap();
        dec += start.elapsed().as_nanos();
        assert!(&out == body);
    }
    (enc as f64 / quads as f64, dec as f64 / quads as f64)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let step = flag("--step").and_then(|v| v.parse().ok()).unwrap_or(1);
    let threads = flag("--threads").and_then(|v| v.parse().ok()).unwrap_or(4);
    let only: Option<Vec<&str>> = flag("--only").map(|v| v.split(',').collect());
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    let m: &'static Map = Box::leak(Box::new(load(Path::new(args.first().expect("corpus directory")), step)));
    let mut tiles: Vec<Tile> = m.keys().copied().collect();
    tiles.sort();
    let quads: usize = m.values().map(|b| b.quads).sum();
    let raw: usize = m.values().map(|b| b.bytes.len()).sum();
    println!("{} tiles, {:.2} M quads, bodies {:.1} MB", tiles.len(), quads as f64 / 1e6, raw as f64 / 1e6);
    let mut base = None;
    for (name, kind, level, window) in variants() {
        if only.as_ref().is_some_and(|o| !o.contains(&name)) {
            continue;
        }
        let start = Instant::now();
        let total: usize = tiles.par_iter().map_init(|| cctx(level, window), |c, &t| size(m, t, &kind, c)).sum();
        let base = *base.get_or_insert(total);
        println!(
            "{name:10} {:9.3} MB  {:.4}  {:5.2} b/quad  ({:.1} s)",
            total as f64 / 1e6,
            total as f64 / base as f64,
            8.0 * total as f64 / quads as f64,
            start.elapsed().as_secs_f64()
        );
    }
    if args.iter().any(|a| a == "--time") {
        for level in [9, 12, 15, 17, 19] {
            let (enc, dec) = time(m, &tiles, level, quads);
            println!("time z{level:<3} zstd compress {enc:7.1} ns/quad  decompress {dec:5.1} ns/quad");
        }
    }
}
