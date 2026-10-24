//! Chunk ingest on one thread: region reads, decompression and chunk parsing timed apart.
//!
//! Usage: `cargo run --release -p bm-world --example chunk_bench -- <region dir> [--regions N] [--reps 3]
//! [--phase all|inflate|parse] [--no-palette-cache] [--full-digest]`. Times are the fastest of `reps` passes over all
//! chunks. `--phase` repeats only that phase (the others run once), so `perf stat` differences between `--reps`
//! isolate it. The `digest` line sums a probe of every parsed chunk: it must not change when only speed does.
//! `--full-digest` adds one over every block, light, biome and heightmap value, to compare parsers across commits.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bm_world::chunk::PaletteCache;
use bm_world::region::{Region, list_regions};
use bm_world::{BiomeId, Biomes, BlockStates, Chunk, ChunkContext, DimensionType, StateId};

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
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let dir = PathBuf::from(args.first().expect("region directory"));
    let regions: usize = flag("--regions").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let reps: usize = flag("--reps").and_then(|v| v.parse().ok()).unwrap_or(3);
    let phase = flag("--phase").map_or("all", String::as_str);
    let reps_of = |p: &str| if phase == "all" || phase == p { reps } else { 1 };

    let mut compressed = Vec::new();
    let (mut raw, mut out) = (Vec::new(), Vec::new());
    let mut read = Duration::ZERO;
    for (rx, rz) in list_regions(&dir).expect("region dir").into_iter().take(regions) {
        let file = std::fs::read(dir.join(format!("r.{rx}.{rz}.mca"))).expect("region file");
        let start = Instant::now();
        let mut region = Region::open(&dir, rx, rz).expect("region");
        region.preload((0..32).flat_map(|z| (0..32).map(move |x| (x, z))));
        for (lx, lz) in (0..32).flat_map(|z| (0..32).map(move |x| (x, z))) {
            if region.read_chunk_into(lx, lz, &mut raw, &mut out).expect("chunk") {
                compressed.push(zlib_payload(&file, lx, lz).to_vec());
            }
        }
        read += start.elapsed();
    }
    let inflate = fastest(reps_of("inflate"), || {
        for bytes in &compressed {
            bm_compress::Compression::Deflate.decompress_into(bytes, 256 << 20, &mut out).expect("inflate");
        }
    });
    let nbt: Vec<Vec<u8>> = compressed
        .iter()
        .map(|bytes| bm_compress::Compression::Deflate.decompress(bytes, 256 << 20).expect("inflate"))
        .collect();
    let (states, biomes) = (BlockStates::default(), Biomes::default());
    let dimension = DimensionType::builtin("minecraft:overworld").expect("overworld");
    let cache = PaletteCache::default();
    let palette = (!args.iter().any(|a| a == "--no-palette-cache")).then_some(&cache);
    let ctx = ChunkContext { states: &states, biomes: &biomes, dimension: &dimension, palette };
    let mut digest = 0u64;
    let parse = fastest(reps_of("parse"), || {
        digest = 0;
        for data in &nbt {
            let chunk = Chunk::parse(data, &ctx).expect("parse");
            let probe = chunk.block(5, 64, 9).0 as u64 ^ (chunk.light(3, 70, 12).0 as u64) << 32;
            digest = digest.wrapping_mul(31).wrapping_add(probe ^ chunk.block_entities.len() as u64);
        }
    });
    let mb = nbt.iter().map(Vec::len).sum::<usize>() as f64 / 1e6;
    let per = |d: Duration| d.as_nanos() as f64 / nbt.len() as f64 / 1e3;
    println!(
        "{} chunks, {mb:.1} MB NBT; first pass (read + inflate, cached file) {:.1} µs/chunk",
        nbt.len(),
        per(read)
    );
    println!("inflate {:7.1} µs/chunk {:6.0} MB/s", per(inflate), mb / inflate.as_secs_f64());
    println!("parse   {:7.1} µs/chunk {:6.0} MB/s", per(parse), mb / parse.as_secs_f64());
    println!("digest {digest:016x}");
    if args.iter().any(|a| a == "--full-digest") {
        let mut keys = HashMap::new();
        let mut full = 0;
        for data in &nbt {
            let chunk = Chunk::parse(data, &ctx).expect("parse");
            let mut key_hash = |s: StateId| *keys.entry(s).or_insert_with(|| hash(states.get(s).key.as_bytes()));
            let mut biome_hash = |b: BiomeId| hash(biomes.name(b).as_bytes());
            full = mix(full, chunk_digest(&chunk, &mut key_hash, &mut biome_hash));
        }
        println!("full digest {full:016x}");
    }
}

fn mix(d: u64, v: u64) -> u64 {
    (d ^ v).wrapping_mul(0x100_0000_01b3).rotate_left(23)
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |h, &b| mix(h, b as u64))
}

/// Every block and biome (by name, so interning order doesn't matter), light and heightmap value of `c`.
fn chunk_digest(
    c: &Chunk,
    key_hash: &mut impl FnMut(StateId) -> u64,
    biome_hash: &mut impl FnMut(BiomeId) -> u64,
) -> u64 {
    let mut d = mix(c.min_y() as u64, c.max_y() as u64);
    for (x, z) in (0..16).flat_map(|x| (0..16).map(move |z| (x, z))) {
        d = mix(
            d,
            c.world_surface_y(x, z).unwrap_or(-9999) as u64 ^ (c.ocean_floor_y(x, z).unwrap_or(-9999) as u64) << 20,
        );
        for y in c.min_y()..=c.max_y() {
            let (sky, block) = c.light(x, y, z);
            d = mix(d, key_hash(c.block(x, y, z)) ^ (sky as u64) << 56 ^ (block as u64) << 48);
            if x % 4 == 0 && z % 4 == 0 && y % 4 == 0 {
                d = mix(d, biome_hash(c.biome(x, y, z)));
            }
        }
    }
    d
}

/// The zlib stream of a chunk in a region file's bytes, sector padding included as the reader sees it.
fn zlib_payload(file: &[u8], lx: usize, lz: usize) -> &[u8] {
    let h = &file[4 * (lz * 32 + lx)..][..4];
    let offset = u32::from_be_bytes([0, h[0], h[1], h[2]]) as usize * 4096;
    let sectors = &file[offset..(offset + h[3] as usize * 4096).min(file.len())];
    assert_eq!(sectors[4], 2, "zlib chunks only");
    &sectors[5..]
}
