//! Serves an existing BlueMap webroot (every `maps/<id>` as gzip file storage) for load tests and profiling.
//!
//! `cargo run -p bm-web --profile profiling --example serve_bench -- <webroot> [--port 8100] [--live] [--log <file>]
//! [--exit-after <secs>] [--etags] [--optimized]`
//!
//! `--etags` sends map-data ETags; `--optimized` opens the maps as an optimized storage (convert a copy first).
//!
//! A counting global allocator tracks allocations; send `stats` on stdin to print and reset them as one JSON line.
//! `--live` registers players/markers/SSE on every map and pushes a fresh players.json every second.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Duration;

use bm_storage::{Compression, FileStorage, Format, Storage};
use bm_web::{AccessLog, FileSink, LiveMap, LogSink, MapRoute, WebApp, WebOptions, WebServer};

struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_sub(layout.size(), Relaxed);
        record(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn record(size: usize) {
    ALLOCS.fetch_add(1, Relaxed);
    ALLOC_BYTES.fetch_add(size as u64, Relaxed);
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn stats_loop() {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim() == "stats" {
            let live = LIVE.load(Relaxed);
            println!(
                "{{\"allocs\":{},\"alloc_bytes\":{},\"live_bytes\":{live},\"peak_live_bytes\":{}}}",
                ALLOCS.swap(0, Relaxed),
                ALLOC_BYTES.swap(0, Relaxed),
                PEAK.swap(live, Relaxed),
            );
        }
    }
}

fn players_json(tick: u64) -> String {
    let players: Vec<String> = (0..20)
        .map(|i| {
            let t = tick as f64 * 0.37 + f64::from(i);
            format!(
                "{{\"uuid\":\"00000000-0000-0000-0000-{i:012}\",\"name\":\"player{i}\",\"foreign\":false,\
                 \"position\":{{\"x\":{},\"y\":{},\"z\":{}}},\"rotation\":{{\"pitch\":{},\"yaw\":{},\"roll\":0.0}}}}",
                t.sin() * 1234.5678901,
                64.0 + t.cos(),
                t.cos() * 987.654321,
                t * 3.3 % 90.0,
                t * 7.1 % 360.0
            )
        })
        .collect();
    format!("{{\"players\":[{}]}}", players.join(","))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let webroot = PathBuf::from(args.next().ok_or("usage: serve_bench <webroot> …")?);
    let (mut port, mut live, mut log, mut exit_after) = (8100, false, None::<String>, None::<u64>);
    let (mut etags, mut format) = (false, Format::Compat);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().unwrap_or_default().parse()?,
            "--exit-after" => exit_after = Some(args.next().unwrap_or_default().parse()?),
            "--live" => live = true,
            "--etags" => etags = true,
            "--optimized" => format = Format::Optimized,
            "--log" => log = args.next(),
            other => return Err(format!("unknown arg {other}").into()),
        }
    }
    let mut options = WebOptions::new(&webroot);
    options.map_etags = etags;
    if let Some(file) = log {
        let sinks: Vec<Box<dyn LogSink>> = vec![Box::new(FileSink::open(&file, false)?)];
        options.access_log = AccessLog::new("%1$s \"%3$s %4$s %5$s\" %6$s %7$s", sinks)?;
    }
    let mut app = WebApp::new(options)?;
    let storage = FileStorage::open(webroot.join("maps"), Compression::Gzip, format, true)?;
    let mut lives = Vec::new();
    for entry in std::fs::read_dir(webroot.join("maps"))? {
        let id = entry?.file_name().to_string_lossy().into_owned();
        let live = live.then(|| Arc::new(LiveMap::new(true).with_players().with_markers()));
        lives.extend(live.clone());
        eprintln!("map {id}");
        app.add_map(&id, MapRoute { storage: storage.map(&id)?, live })?;
    }
    if !lives.is_empty() {
        tokio::spawn(async move {
            let mut tick = 0;
            loop {
                for l in &lives {
                    l.set_players(players_json(tick));
                    l.set_markers("{}");
                }
                tick += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
    std::thread::spawn(stats_loop);
    let server = WebServer::bind("127.0.0.1", port).await?;
    eprintln!("listening on {}", server.local_addr());
    // --exit-after lets an elevated profiler run end on its own (it can't be killed from an unelevated shell)
    let shutdown = async move {
        match exit_after {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => drop(tokio::signal::ctrl_c().await),
        }
    };
    server.serve(app, shutdown).await?;
    Ok(())
}
