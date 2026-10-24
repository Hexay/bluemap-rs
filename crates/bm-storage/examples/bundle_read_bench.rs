//! Hires read latency of an optimized file storage as the webserver sees it (`read_hires_packed`), N threads.
//! Usage: cargo run -p bm-storage --release --example bundle_read_bench -- <maps root> <map id> [--threads T]
//!        [--secs S]
//!   cold: one random tile of every bundle, each bundle's first touch (evict the page cache first for a disk-cold
//!   run: `vmtouch -e` or posix_fadvise DONTNEED). warm: random tiles of random bundles for S seconds.
//!   hot: random tiles of 4 bundles (one webapp view) for S seconds.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use bm_storage::{Compression, FileStorage, Format, MapStorage, Storage, Tile};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn bundles(dir: &PathBuf) -> Vec<Tile> {
    let mut out: Vec<Tile> = std::fs::read_dir(dir)
        .expect("hires dir")
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let (x, z) = name.strip_prefix('x')?.strip_suffix(".bmb")?.split_once('z')?;
            Some((x.parse().ok()?, z.parse().ok()?))
        })
        .collect();
    out.sort_unstable();
    out
}

fn tile_in(b: Tile, rng: &mut Rng) -> Tile {
    let r = rng.next();
    ((b.0 << 4) | (r & 15) as i32, (b.1 << 4) | ((r >> 4) & 15) as i32)
}

/// Runs `work(thread rng) -> Some(latency)` on `threads` threads until it returns `None`; prints a summary.
fn run(name: &str, threads: usize, work: impl Fn(&mut Rng) -> Option<(Duration, bool)> + Sync) {
    let lat = Mutex::new(Vec::new());
    let found = AtomicUsize::new(0);
    let start = Instant::now();
    std::thread::scope(|s| {
        for t in 0..threads {
            let (lat, found, work) = (&lat, &found, &work);
            s.spawn(move || {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (t as u64 + 1) * 0x2545_F491);
                let mut mine = Vec::new();
                while let Some((d, hit)) = work(&mut rng) {
                    mine.push(d);
                    found.fetch_add(usize::from(hit), Relaxed);
                }
                lat.lock().unwrap().extend(mine);
            });
        }
    });
    let wall = start.elapsed().as_secs_f64();
    let mut lat = lat.into_inner().unwrap();
    lat.sort_unstable();
    let pct = |p: f64| {
        lat.get(((lat.len() as f64 * p) as usize).min(lat.len().saturating_sub(1))).map_or(0, |d| d.as_micros())
    };
    println!(
        "{name}: {} reads ({} found) in {wall:.2}s = {:.0} req/s; p50 {} µs, p99 {} µs, max {} µs",
        lat.len(),
        found.load(Relaxed),
        lat.len() as f64 / wall,
        pct(0.5),
        pct(0.99),
        pct(1.0)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |flag: &str, default: usize| {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)?.parse().ok()).unwrap_or(default)
    };
    let (root, id) = (PathBuf::from(&args[0]), args[1].clone());
    let (threads, secs) = (opt("--threads", 32), opt("--secs", 8));
    let all = bundles(&root.join(&id).join("hires"));
    let storage = FileStorage::open(&root, Compression::Gzip, Format::Optimized, true).expect("storage");
    let map: std::sync::Arc<dyn MapStorage> = storage.map(&id).expect("map");
    println!("{} bundles, {threads} threads", all.len());
    let read = |tile: Tile| {
        let t = Instant::now();
        let hit = map.read_hires_packed(tile).expect("read").is_some();
        (t.elapsed(), hit)
    };
    let next = AtomicUsize::new(0);
    run("cold (first touch per bundle)", threads, |rng| {
        let b = *all.get(next.fetch_add(1, Relaxed))?;
        Some(read(tile_in(b, rng)))
    });
    let until = Instant::now() + Duration::from_secs(secs as u64);
    run("warm (random bundles)", threads, |rng| {
        (Instant::now() < until).then(|| read(tile_in(all[rng.next() as usize % all.len()], rng)))
    });
    let hot = &all[all.len() / 2..all.len() / 2 + 4.min(all.len() / 2)];
    let until = Instant::now() + Duration::from_secs(secs as u64);
    run("hot (one view: 4 bundles)", threads, |rng| {
        (Instant::now() < until).then(|| read(tile_in(hot[rng.next() as usize % hot.len()], rng)))
    });
}
