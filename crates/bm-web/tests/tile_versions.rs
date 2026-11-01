//! The batch behind the client script's fast reload (docs/18): the versions it reports are the ones in tile ETags,
//! a rewrite changes them, and a server not offering the script answers as if the route did not exist.

mod common;

use std::sync::Arc;

use bm_storage::{Format, GridKey, MapStorage};
use bm_web::{MapRoute, WebApp, WebOptions};
use common::{Served, TILE, UNPACKS_FRAMES, get, packing_map};

fn serve(storage: Arc<dyn MapStorage>, web: &std::path::Path, change: impl FnOnce(&mut WebOptions)) -> Served {
    let mut options = WebOptions::new(web);
    change(&mut options);
    let mut app = WebApp::new(options).unwrap();
    app.add_map("world", MapRoute { storage, live: None }).unwrap();
    Served::start(app)
}

const URL: &str = "/maps/world/tile-versions/0,-1,-1,1,1/1,0,0,0,1";

fn lines(body: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(body).lines().map(str::to_owned).collect()
}

#[test]
fn versions_match_tile_etags_and_follow_rewrites() {
    let dir = tempfile::tempdir().unwrap();
    let map = packing_map(dir.path(), Format::Optimized);
    map.write_grid(GridKey::Lowres(1), (0, 1), b"png").unwrap();
    let s = serve(map.clone(), dir.path(), |_| {});

    let r = get(s.addr, URL, &[]);
    assert_eq!((r.status, r.header("cache-control")), (200, Some("no-store")));
    let found = lines(&r.body);
    let version = |prefix: &str| found.iter().find_map(|l| l.strip_prefix(prefix)).unwrap().to_owned();
    assert_eq!(found.len(), 3, "the two hires tiles and one lowres, nothing for empty cells: {found:?}");

    let (hires, lowres) = (version("0 0 0 "), version("1 0 1 "));
    let tile = get(s.addr, TILE, &[UNPACKS_FRAMES, ("Accept-Encoding", "zstd")]);
    assert!(tile.header("etag").unwrap().starts_with(&format!("\"{hires}-")), "{:?}", tile.header("etag"));
    let png = get(s.addr, "/maps/world/tiles/1/x0/z1.png", &[]);
    assert!(png.header("etag").unwrap().starts_with(&format!("\"{lowres}")), "{:?}", png.header("etag"));

    map.write_grid(GridKey::Hires, (0, 0), &common::prbm_of(41)).unwrap();
    let again = lines(&get(s.addr, URL, &[]).body);
    assert!(!again.contains(&format!("0 0 0 {hires}")), "{again:?}");
}

#[test]
fn bad_requests_and_servers_without_the_script() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(packing_map(dir.path(), Format::Optimized), dir.path(), |_| {});
    assert_eq!(get(s.addr, "/maps/world/tile-versions/0,0,0,1000,1000", &[]).status, 400);
    assert_eq!(get(s.addr, "/maps/world/tile-versions/nonsense", &[]).status, 400);
    assert_eq!(get(s.addr, "/maps/world/tile-versions", &[]).status, 404);

    let off = serve(packing_map(dir.path(), Format::Optimized), dir.path(), |o| o.client_unpack = false);
    assert_eq!(get(off.addr, URL, &[]).status, 404);
    let no_etags = serve(packing_map(dir.path(), Format::Optimized), dir.path(), |o| o.map_etags = false);
    assert_eq!(get(no_etags.addr, URL, &[]).status, 404);

    let compat = tempfile::tempdir().unwrap();
    let c = serve(packing_map(compat.path(), Format::Compat), compat.path(), |_| {});
    assert_eq!(get(c.addr, URL, &[]).status, 404);
}
