//! Storage write throughput, CPU split (user/kernel) and allocations per write, on real hires tiles.
//! Usage: cargo run -p bm-storage --profile profiling --example write_bench -- <hires tiles dir> <scratch dir> [--threads T]
//!        [--sql <url>]...
//!   <hires tiles dir> is a webroot's `maps/<id>/tiles/0`; the scratch dir is wiped. `--sql` also benches a MySQL or
//!   PostgreSQL server (`tools/dbs.py`) in tables `bmbench_*`, dropped afterwards.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use bm_storage::{Compression, FileStorage, GridKey, SqlConfig, SqlStorage, Storage, Tile};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        ALLOC_BYTES.fetch_add(l.size() as u64, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        ALLOC_BYTES.fetch_add(n as u64, Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[cfg(windows)]
fn cpu_ms() -> (f64, f64) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(h: isize, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
    }
    let (mut c, mut e, mut k, mut u) = (0, 0, 0, 0);
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    (u as f64 / 1e4, k as f64 / 1e4)
}

#[cfg(not(windows))]
fn cpu_ms() -> (f64, f64) {
    (0.0, 0.0)
}

struct Tiles {
    raw: Vec<(Tile, Vec<u8>)>,
    gz: Vec<Vec<u8>>,
}

fn load(dir: &Path) -> Tiles {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p, out) } else if p.to_string_lossy().ends_with(".prbm.gz") { out.push(p) }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    let (mut raw, mut gz) = (Vec::new(), Vec::new());
    for (i, f) in files.iter().enumerate() {
        let stored = fs::read(f).unwrap();
        // synthetic distinct coordinates: only the byte stream and the digit-split depth matter here
        let tile = ((i % 40) as i32 * 7 - 140, (i / 40) as i32 * 7 - 140);
        raw.push((tile, Compression::Gzip.decompress(&stored, 1 << 30).unwrap()));
        gz.push(Compression::Gzip.compress(&raw.last().unwrap().1).unwrap());
    }
    Tiles { raw, gz }
}

/// Runs `op(i)` for every tile index over `threads` workers; prints throughput, CPU and allocations.
fn run(label: &str, n: usize, bytes: usize, threads: usize, op: impl Fn(usize) + Sync) {
    let next = AtomicUsize::new(0);
    let (a0, b0) = (ALLOCS.load(Relaxed), ALLOC_BYTES.load(Relaxed));
    let (u0, k0) = cpu_ms();
    let t = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    if i >= n {
                        break;
                    }
                    op(i);
                }
            });
        }
    });
    let wall = t.elapsed().as_secs_f64();
    let (u1, k1) = cpu_ms();
    let (a, b) = (ALLOCS.load(Relaxed) - a0, ALLOC_BYTES.load(Relaxed) - b0);
    println!(
        "| {label} | {threads} | {:.0} | {:.1} | {:.3} | {:.3} | {:.1} | {:.0} |",
        n as f64 / wall,
        bytes as f64 / 1e6 / wall,
        (u1 - u0) / n as f64,
        (k1 - k0) / n as f64,
        a as f64 / n as f64,
        b as f64 / n as f64 / 1024.0,
    );
}

/// Same steps as `fsops::write_atomic`, timed per step (µs per write, single thread).
fn atomic_breakdown(dir: &Path, data: &[Vec<u8>]) {
    let mut t = [0u128; 4];
    for (i, d) in data.iter().enumerate() {
        let target = dir.join(format!("x{}/z{}.prbm.gz", i % 16, i));
        let tmp = target.with_extension("tmp.filepart");
        let s = Instant::now();
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp).unwrap_or_else(|_| {
            fs::create_dir_all(tmp.parent().unwrap()).unwrap();
            fs::OpenOptions::new().write(true).create_new(true).open(&tmp).unwrap()
        });
        t[0] += s.elapsed().as_nanos();
        let s = Instant::now();
        f.write_all(d).unwrap();
        t[1] += s.elapsed().as_nanos();
        let s = Instant::now();
        drop(f);
        t[2] += s.elapsed().as_nanos();
        let s = Instant::now();
        fs::rename(&tmp, &target).unwrap();
        t[3] += s.elapsed().as_nanos();
    }
    let us = |x: u128| x as f64 / 1e3 / data.len() as f64;
    println!("atomic write steps, µs/write: open(create_new) {:.0}, write {:.0}, close {:.0}, rename {:.0}", us(t[0]), us(t[1]), us(t[2]), us(t[3]));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let threads: usize = args.iter().position(|a| a == "--threads").map_or(8, |i| args[i + 1].parse().unwrap());
    let tiles = load(Path::new(&args[0]));
    let scratch = PathBuf::from(&args[1]);
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let n = tiles.raw.len();
    let raw_bytes: usize = tiles.raw.iter().map(|t| t.1.len()).sum();
    let gz_bytes: usize = tiles.gz.iter().map(Vec::len).sum();
    println!("{n} tiles, raw {raw_bytes} B, gzip {gz_bytes} B; MB/s columns are of the bytes handed to the call");
    println!("| op | threads | tiles/s | MB/s | user ms/tile | kernel ms/tile | allocs/tile | alloc KiB/tile |");
    println!("|---|---|---|---|---|---|---|---|");

    for t in [1, threads] {
        let root = scratch.join(format!("file-{t}"));
        let map = FileStorage::new(&root, Compression::Gzip).map("m").unwrap();
        run("file write_grid gzip (fresh)", n, raw_bytes, t, |i| map.write_grid(GridKey::Hires, tiles.raw[i].0, &tiles.raw[i].1).unwrap());
        run("file write_grid gzip (overwrite)", n, raw_bytes, t, |i| map.write_grid(GridKey::Hires, tiles.raw[i].0, &tiles.raw[i].1).unwrap());
        run("gzip-6 compress only", n, raw_bytes, t, |i| drop(Compression::Gzip.compress(&tiles.raw[i].1).unwrap()));
        run("file write_grid_encoded (overwrite)", n, gz_bytes, t, |i| map.write_grid_encoded(GridKey::Hires, tiles.raw[i].0, &tiles.gz[i]).unwrap());
        let fm = FileStorage::new(&root, Compression::Gzip).file_map("m").unwrap();
        run("plain fs::write, non-atomic (overwrite)", n, gz_bytes, t, |i| fs::write(fm.grid_cell_path(GridKey::Hires, tiles.raw[i].0), &tiles.gz[i]).unwrap());
        let zroot = scratch.join(format!("zstd-{t}"));
        let zmap = FileStorage::new(&zroot, Compression::Zstd).map("m").unwrap();
        run("file write_grid zstd-3 (fresh)", n, raw_bytes, t, |i| zmap.write_grid(GridKey::Hires, tiles.raw[i].0, &tiles.raw[i].1).unwrap());
    }
    atomic_breakdown(&scratch.join("steps"), &tiles.gz);

    let rt = tokio::runtime::Runtime::new().unwrap();
    for t in [1, threads] {
        let db = scratch.join(format!("sqlite-{t}.db"));
        let storage = SqlStorage::connect(&SqlConfig::new(format!("sqlite:{}", db.display())), rt.handle().clone()).unwrap();
        let map = storage.map("m").unwrap();
        run("sqlite write_grid_encoded (fresh)", n, gz_bytes, t, |i| map.write_grid_encoded(GridKey::Hires, tiles.raw[i].0, &tiles.gz[i]).unwrap());
        run("sqlite write_grid_encoded (overwrite)", n, gz_bytes, t, |i| map.write_grid_encoded(GridKey::Hires, tiles.raw[i].0, &tiles.gz[i]).unwrap());
        run("sqlite write_grid gzip (overwrite)", n, raw_bytes, t, |i| map.write_grid(GridKey::Hires, tiles.raw[i].0, &tiles.raw[i].1).unwrap());
        storage.close();
        println!("sqlite db size after {t}-thread run: {} B (+wal {} B)", fs::metadata(&db).map_or(0, |m| m.len()), fs::metadata(db.with_extension("db-wal")).map_or(0, |m| m.len()));
    }
    sqlite_batched(&rt, &scratch.join("sqlite-batch.db"), &tiles);
    for (i, _) in args.iter().enumerate().filter(|(_, a)| *a == "--sql") {
        sql_server(&rt, &args[i + 1], &tiles, threads);
    }
}

fn sql_server(rt: &tokio::runtime::Runtime, url: &str, tiles: &Tiles, threads: usize) {
    use sqlx::{Connection, Executor};
    let n = tiles.gz.len();
    let gz_bytes: usize = tiles.gz.iter().map(Vec::len).sum();
    let scheme = url.split(':').next().unwrap_or(url);
    for t in [1, threads] {
        let config = SqlConfig { table_prefix: "bmbench_".into(), max_connections: t as u32, ..SqlConfig::new(url) };
        let storage = SqlStorage::connect(&config, rt.handle().clone()).unwrap();
        let map = storage.map("m").unwrap();
        run(&format!("{scheme} write_grid_encoded (fresh)"), n, gz_bytes, t, |i| map.write_grid_encoded(GridKey::Hires, tiles.raw[i].0, &tiles.gz[i]).unwrap());
        run(&format!("{scheme} write_grid_encoded (overwrite)"), n, gz_bytes, t, |i| map.write_grid_encoded(GridKey::Hires, tiles.raw[i].0, &tiles.gz[i]).unwrap());
        run(&format!("{scheme} read_grid"), n, gz_bytes, t, |i| drop(map.read_grid(GridKey::Hires, tiles.raw[i].0).unwrap()));
        storage.close();
        rt.block_on(async {
            let (_, sqlx_url) = bm_storage::Dialect::from_url(url).unwrap();
            for table in ["grid_storage_data", "item_storage_data", "grid_storage", "item_storage", "compression", "map"] {
                let drop = format!("DROP TABLE bmbench_{table}");
                if scheme.starts_with("postgres") {
                    sqlx::PgConnection::connect(&sqlx_url).await.unwrap().execute(drop.as_str()).await.unwrap();
                } else {
                    sqlx::MySqlConnection::connect(&sqlx_url).await.unwrap().execute(drop.as_str()).await.unwrap();
                }
            }
        });
    }
}

/// One transaction around all upserts on one connection: the ceiling for batched SQL writes.
fn sqlite_batched(rt: &tokio::runtime::Runtime, db: &Path, tiles: &Tiles) {
    use sqlx::Connection;
    let storage = SqlStorage::connect(&SqlConfig::new(format!("sqlite:{}", db.display())), rt.handle().clone()).unwrap();
    storage.map("m").unwrap().write_grid_encoded(GridKey::Hires, (9999, 9999), b"x").unwrap();
    storage.close();
    let n = tiles.gz.len();
    let gz_bytes: usize = tiles.gz.iter().map(Vec::len).sum();
    let url = format!("sqlite:{}", db.display());
    run("sqlite 1 tx, raw sqlx REPLACE (overwrite x2)", n, gz_bytes, 1, |i| {
        if i == 0 {
            rt.block_on(async {
                let mut c = sqlx::SqliteConnection::connect(&url).await.unwrap();
                sqlx::query("PRAGMA journal_mode=WAL").execute(&mut c).await.unwrap();
                sqlx::query("PRAGMA synchronous=NORMAL").execute(&mut c).await.unwrap();
                for _ in 0..2 {
                    let mut tx = c.begin().await.unwrap();
                    for (j, gz) in tiles.gz.iter().enumerate() {
                        let (x, z) = tiles.raw[j].0;
                        sqlx::query("REPLACE INTO bluemap_grid_storage_data (map, storage, x, z, compression, data) VALUES (1, 1, ?, ?, 1, ?)")
                            .bind(x)
                            .bind(z)
                            .bind(&gz[..])
                            .execute(&mut *tx)
                            .await
                            .unwrap();
                    }
                    tx.commit().await.unwrap();
                }
            });
        }
    });
    println!("(batched row: per-tile figures cover 2 writes of each tile)");
}
