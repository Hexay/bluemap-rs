//! `server-unpack` (docs/18): what a client without the client-unpack opt-in gets for the packed hires tiles of an
//! optimized storage. The transcode cache is per process, so every test packs its own tile model.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use bm_storage::{Compression, FileStorage, Format, GridKey, MapStorage, Storage};
use bm_web::{MapRoute, ServerUnpack, WebApp, WebOptions};
use common::{Conn, PRBM_TYPE, RAW_TILE, Reply, Served, TILE, UNPACKS, get, prbm_of};

const ZSTD: (&str, &str) = ("Accept-Encoding", "gzip, deflate, br, zstd");
const GZIP: (&str, &str) = ("Accept-Encoding", "gzip");
const LOWRES: &str = "/maps/world/tiles/1/x0/z0.png";

/// Held by the timing tests and the flood: on a small CI runner the flood's 200 connections starve the others.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// [`common::packing_map`] with a tile model of its own, plus a lowres tile.
fn map(root: &Path, format: Format, quads: u32) -> Arc<dyn MapStorage> {
    let map = FileStorage::open(root.join("maps"), Compression::Gzip, format, false).unwrap().map("world").unwrap();
    map.write_grid(GridKey::Hires, (0, 0), &prbm_of(quads)).unwrap();
    map.write_grid(GridKey::Hires, (1, 0), b"not a PRBM").unwrap();
    map.write_grid(GridKey::Lowres(1), (0, 0), b"\x89PNG lowres").unwrap();
    map
}

/// `slots` with no queue (a refusal is immediate), on a 4-thread blocking pool.
fn serve(storage: Arc<dyn MapStorage>, mode: ServerUnpack, slots: usize) -> Served {
    serve_queued(storage, mode, slots, 0, 4)
}

fn serve_queued(
    storage: Arc<dyn MapStorage>,
    mode: ServerUnpack,
    slots: usize,
    queue: usize,
    threads: usize,
) -> Served {
    serve_trusting(storage, mode, slots, queue, threads, &[])
}

fn serve_trusting(
    storage: Arc<dyn MapStorage>,
    mode: ServerUnpack,
    slots: usize,
    queue: usize,
    threads: usize,
    trusted_proxies: &[&str],
) -> Served {
    let mut options = WebOptions::new(std::env::temp_dir().join("bm-web-no-webroot"));
    (options.server_unpack, options.server_unpack_slots, options.server_unpack_queue) = (mode, slots, queue);
    options.trusted_proxies = trusted_proxies.iter().map(|p| p.parse().unwrap()).collect();
    let mut app = WebApp::new(options).unwrap();
    app.add_map("world", MapRoute { storage, live: None }).unwrap();
    Served::start_with_blocking_threads(app, threads)
}

fn assert_busy(r: &Reply) {
    assert_eq!((r.status, r.header("retry-after")), (503, Some("1")));
    assert_eq!((r.header("cache-control"), r.header("cdn-cache-control")), (Some("no-store"), Some("no-store")));
    assert_eq!(r.header("vary"), Some("Accept, Accept-Encoding"));
}

#[test]
fn limited_is_the_default_and_unpacks_while_a_slot_is_free() {
    assert_eq!(bm_config::WebserverConfig::default().server_unpack, ServerUnpack::Limited);
    let options = WebOptions::new(".");
    assert_eq!(options.server_unpack, ServerUnpack::Limited);
    let cores = std::thread::available_parallelism().map_or(4, usize::from);
    assert_eq!(options.server_unpack_slots, (cores / 4).max(1));
    assert_eq!(options.server_unpack_queue, (64 * options.server_unpack_slots).clamp(128, 256));
    assert!(options.trusted_proxies.is_empty());

    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized, 41), ServerUnpack::Limited, 1);
    let r = get(s.addr, TILE, &[GZIP]);
    assert_eq!((r.status, r.header("content-type")), (200, PRBM_TYPE));
    assert!(r.decoded() == prbm_of(41));
}

#[test]
fn limited_refuses_at_once_without_a_slot() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized, 42), ServerUnpack::Limited, 0);
    for target in [TILE, RAW_TILE, &format!("{TILE}.gz")] {
        assert_busy(&get(s.addr, target, &[ZSTD]));
    }
    assert_busy(&get(s.addr, &format!("{TILE}.gz"), &[UNPACKS]));
    // the opt-in, a raw tile for it, and everything but hires tiles are not limited
    let packed = get(s.addr, TILE, &[UNPACKS, ZSTD]);
    assert_eq!((packed.status, packed.header("content-type")), (200, Some("application/vnd.bluemap.bmq3")));
    let raw = get(s.addr, RAW_TILE, &[UNPACKS, ZSTD]);
    assert_eq!((raw.status, raw.decoded()), (200, b"not a PRBM".to_vec()));
    assert_eq!(get(s.addr, LOWRES, &[ZSTD]).status, 200);

    let compat_dir = tempfile::tempdir().unwrap();
    let compat = serve(map(compat_dir.path(), Format::Compat, 42), ServerUnpack::Limited, 0);
    assert!(get(compat.addr, TILE, &[ZSTD]).decoded() == prbm_of(42), "compat tiles are stored as served");
}

#[test]
fn transcode_cache_hits_bypass_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let storage = map(dir.path(), Format::Optimized, 43);
    let warm = serve(storage.clone(), ServerUnpack::Always, 0);
    let first = get(warm.addr, TILE, &[ZSTD]);
    assert_eq!(first.status, 200);
    // identity bodies are never cached
    let identity = ("Accept-Encoding", "identity");
    let identity_tag = get(warm.addr, TILE, &[identity]).header("etag").unwrap().to_owned();
    drop(warm);
    let s = serve(storage, ServerUnpack::Limited, 0);
    let hit = get(s.addr, TILE, &[ZSTD]);
    assert!((hit.status, &hit.body) == (200, &first.body));
    assert_busy(&get(s.addr, TILE, &[GZIP]));
    assert_busy(&get(s.addr, TILE, &[identity]));
    assert_eq!(get(s.addr, TILE, &[identity, ("If-None-Match", &identity_tag)]).status, 304, "nothing to unpack");
}

#[test]
fn never_answers_406_for_packed_tiles_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized, 44), ServerUnpack::Never, 1);
    for target in [TILE.to_owned(), format!("{TILE}.gz")] {
        let r = get(s.addr, &target, &[ZSTD]);
        assert_eq!((r.status, r.header("content-type")), (406, Some("text/plain; charset=utf-8")), "{target}");
        assert_eq!(r.header("cache-control"), Some("no-store"));
        let body = String::from_utf8(r.body).unwrap();
        assert!(body.contains("bluemap-rs webapp") && body.contains("server-unpack") && body.ends_with(".conf\n"));
        assert_eq!(body.lines().count(), 1);
    }
    assert_eq!(get(s.addr, TILE, &[UNPACKS, ZSTD]).status, 200);
    let raw = get(s.addr, RAW_TILE, &[GZIP]);
    assert_eq!((raw.status, raw.decoded()), (200, b"not a PRBM".to_vec()), "stored raw: nothing to unpack");
    assert_eq!(get(s.addr, LOWRES, &[]).status, 200);
}

#[test]
fn always_ignores_the_slots() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized, 45), ServerUnpack::Always, 0);
    for coding in [ZSTD, GZIP, ("Accept-Encoding", "identity")] {
        let r = get(s.addr, TILE, &[coding]);
        assert!((r.status, r.decoded()) == (200, prbm_of(45)), "{coding:?}");
    }
}

/// A view load from one viewer without the script (49 distinct hires tiles at once) on a single slot: every tile
/// waits its turn, none is refused.
#[test]
fn one_viewers_burst_queues_instead_of_failing() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::open(dir.path().join("maps"), Compression::Gzip, Format::Optimized, false)
        .unwrap()
        .map("world")
        .unwrap();
    let tiles: Vec<(i32, i32)> = (-3..4).flat_map(|x| (-3..4).map(move |z| (x, z))).collect();
    for (i, &tile) in tiles.iter().enumerate() {
        map.write_grid(GridKey::Hires, tile, &prbm_of(100 + i as u32)).unwrap();
    }
    let s = serve_queued(map, ServerUnpack::Limited, 1, 64, 512);
    let replies: Vec<Reply> = std::thread::scope(|scope| {
        let reqs: Vec<_> = tiles
            .iter()
            .map(|&(x, z)| scope.spawn(move || get(s.addr, &format!("/maps/world/tiles/0/x{x}/z{z}.prbm"), &[GZIP])))
            .collect();
        reqs.into_iter().map(|r| r.join().unwrap()).collect()
    });
    for (i, r) in replies.iter().enumerate() {
        assert!((r.status, r.decoded()) == (200, prbm_of(100 + i as u32)), "tile {i}: {}", r.status);
    }
}

/// One client floods the single slot (closed loop, identity bodies, which are never cached) and keeps the queue
/// full; a viewer at another address then loads 50 tiles at once. Both come through a proxy on 127.0.0.1: trusted,
/// the viewer gets its fair share and all 50 tiles; untrusted, both are one client and the viewer is crowded out.
#[test]
fn a_flooding_client_leaves_room_for_another() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::open(dir.path().join("maps"), Compression::Gzip, Format::Optimized, false)
        .unwrap()
        .map("world")
        .unwrap();
    map.write_grid(GridKey::Hires, (0, 0), &prbm_of(500)).unwrap();
    let view: Vec<i32> = (0..50).collect();
    for &x in &view {
        map.write_grid(GridKey::Hires, (x, 100), &prbm_of(300 + x as u32)).unwrap();
    }
    let flood_ip = ("X-Forwarded-For", "203.0.113.1");
    let viewer_ip = ("X-Forwarded-For", "203.0.113.2");
    let identity = ("Accept-Encoding", "identity");
    let load_view = |trusted: &[&str]| -> Vec<u16> {
        let s = serve_trusting(map.clone(), ServerUnpack::Limited, 1, 128, 512, trusted);
        let (stop, refused) = (AtomicBool::new(false), AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..200 {
                scope.spawn(|| {
                    let mut conn = Conn::open(s.addr);
                    while !stop.load(Relaxed) {
                        if conn.get(TILE, &[identity, flood_ip]).status == 503 {
                            refused.fetch_add(1, Relaxed);
                        }
                    }
                });
            }
            let t = Instant::now();
            while refused.load(Relaxed) == 0 {
                assert!(t.elapsed() < Duration::from_secs(20), "the flood never filled the queue");
                std::thread::yield_now();
            }
            let reqs: Vec<_> = view
                .iter()
                .map(|&x| {
                    scope.spawn(move || {
                        let r = get(s.addr, &format!("/maps/world/tiles/0/x{x}/z100.prbm"), &[identity, viewer_ip]);
                        assert!(r.status != 200 || r.decoded() == prbm_of(300 + x as u32), "tile {x}");
                        r.status
                    })
                })
                .collect();
            let statuses = reqs.into_iter().map(|r| r.join().unwrap()).collect();
            stop.store(true, Relaxed);
            statuses
        })
    };
    let fair = load_view(&["127.0.0.1"]);
    assert_eq!(fair, vec![200; 50], "trusted proxy: the viewer has its own share");
    let shared = load_view(&[]);
    assert!(shared.contains(&503), "untrusted proxy: one client, crowded out ({shared:?})");
}

/// A queued request gives up at the deadline; with the queue full, the next is refused at once.
#[test]
fn the_queue_is_bounded_in_time_and_length() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let s = serve_queued(map(dir.path(), Format::Optimized, 47), ServerUnpack::Limited, 0, 2, 16);
    std::thread::scope(|scope| {
        let t = Instant::now();
        let waiters: Vec<_> = (0..2).map(|_| scope.spawn(|| get(s.addr, TILE, &[ZSTD]))).collect();
        std::thread::sleep(Duration::from_millis(150));
        // timed alone: on a slow CI runner the spawns and sleep above can eat most of the deadline
        let asked = Instant::now();
        let refused = get(s.addr, TILE, &[ZSTD]);
        assert_busy(&refused);
        assert!(asked.elapsed() < bm_web::SERVER_UNPACK_WAIT, "refused at once, not after waiting");
        for w in waiters {
            assert_busy(&w.join().unwrap());
        }
        assert!(t.elapsed() >= bm_web::SERVER_UNPACK_WAIT);
    });
}

/// A refused request must not wait for a slot on a blocking thread: with every slot taken, no queue and a 4-thread
/// blocking pool, a flood of refused requests leaves opt-in requests prompt.
#[test]
fn refusals_hold_no_blocking_thread() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized, 46), ServerUnpack::Limited, 0);
    let (stop, refused, other) =
        (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let flood: Vec<_> = (0..64)
        .map(|_| {
            let (stop, refused, other, addr) = (stop.clone(), refused.clone(), other.clone(), s.addr);
            std::thread::spawn(move || {
                while !stop.load(Relaxed) {
                    let counter = if get(addr, TILE, &[ZSTD]).status == 503 { &refused } else { &other };
                    counter.fetch_add(1, Relaxed);
                }
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let mut slowest = Duration::ZERO;
    for _ in 0..50 {
        let t = Instant::now();
        assert_eq!(get(s.addr, TILE, &[UNPACKS, ZSTD]).status, 200);
        slowest = slowest.max(t.elapsed());
    }
    stop.store(true, Relaxed);
    flood.into_iter().for_each(|t| t.join().unwrap());
    assert!(refused.load(Relaxed) > 100 && other.load(Relaxed) == 0, "{refused:?} refused, {other:?} other");
    assert!(slowest < Duration::from_secs(2), "slowest opt-in request {slowest:?}");
}
