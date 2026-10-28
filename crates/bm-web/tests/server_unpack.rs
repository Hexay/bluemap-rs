//! `server-unpack` (docs/18): what a client without the client-unpack opt-in gets for the packed hires tiles of an
//! optimized storage. The transcode cache is per process, so every test packs its own tile model.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use bm_storage::{Compression, FileStorage, Format, GridKey, MapStorage, Storage};
use bm_web::{MapRoute, ServerUnpack, WebApp, WebOptions};
use common::{PRBM_TYPE, RAW_TILE, Reply, Served, TILE, UNPACKS, get, prbm_of};

const ZSTD: (&str, &str) = ("Accept-Encoding", "gzip, deflate, br, zstd");
const GZIP: (&str, &str) = ("Accept-Encoding", "gzip");
const LOWRES: &str = "/maps/world/tiles/1/x0/z0.png";

/// [`common::packing_map`] with a tile model of its own, plus a lowres tile.
fn map(root: &Path, format: Format, quads: u32) -> Arc<dyn MapStorage> {
    let map = FileStorage::open(root.join("maps"), Compression::Gzip, format, false).unwrap().map("world").unwrap();
    map.write_grid(GridKey::Hires, (0, 0), &prbm_of(quads)).unwrap();
    map.write_grid(GridKey::Hires, (1, 0), b"not a PRBM").unwrap();
    map.write_grid(GridKey::Lowres(1), (0, 0), b"\x89PNG lowres").unwrap();
    map
}

fn serve(storage: Arc<dyn MapStorage>, mode: ServerUnpack, slots: usize) -> Served {
    let mut options = WebOptions::new(std::env::temp_dir().join("bm-web-no-webroot"));
    (options.server_unpack, options.server_unpack_slots) = (mode, slots);
    let mut app = WebApp::new(options).unwrap();
    app.add_map("world", MapRoute { storage, live: None }).unwrap();
    Served::start_with_blocking_threads(app, 4)
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

/// A refused request must not wait for a slot on a blocking thread: with every slot taken and a 4-thread blocking
/// pool, a flood of refused requests leaves opt-in requests prompt.
#[test]
fn refusals_hold_no_blocking_thread() {
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
