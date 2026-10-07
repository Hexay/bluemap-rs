//! Map-data validators per storage: `ETag` per representation, `If-None-Match` → 304 without a body, a rewritten
//! tile gets a new tag, SQL (no cheap identity) sends none, and with `map_etags` off full replies are unchanged.

mod common;

use std::sync::Arc;

use bm_storage::{Compression, FileStorage, Format, GridKey, ItemKey, MapStorage, SqlConfig, SqlStorage, Storage};
use bm_web::{MapRoute, WebApp, WebOptions};
use common::{Reply, Served, get};

const GZIP: (&str, &str) = ("Accept-Encoding", "gzip");

fn serve(map: Arc<dyn MapStorage>, etags: bool, web: &std::path::Path) -> Served {
    let mut options = WebOptions::new(web);
    options.map_etags = etags;
    let mut app = WebApp::new(options).unwrap();
    app.add_map("world", MapRoute { storage: map, live: None }).unwrap();
    Served::start(app)
}

fn prbm(seed: u8) -> Vec<u8> {
    let mut m = bm_format::prbm::TileModel::default();
    let y = f32::from(seed);
    for tri in [[[0., y, 0.], [0., y, 1.], [1., y, 1.]], [[0., y, 0.], [1., y, 1.], [1., y, 0.]]] {
        tri.iter().for_each(|p| m.position.extend(p));
        m.color.extend([0.5, 0.5, 1.]);
        m.sunlight.push(15);
        m.blocklight.push(0);
        m.material.push(0);
    }
    m.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
    m.ao.extend([1.; 6]);
    let mut out = Vec::new();
    m.write_prbm(&mut out).unwrap();
    out
}

fn fill(map: &dyn MapStorage) {
    map.write_grid(GridKey::Hires, (0, 0), &prbm(1)).unwrap();
    map.write_grid(GridKey::Lowres(1), (0, 0), b"\x89PNG lowres").unwrap();
    map.write_item(&ItemKey::Settings, b"{\"a\":1}").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
}

const PATHS: [&str; 6] = [
    "tiles/0/x0/z0.prbm",
    "tiles/0/x0/z0.prbm.gz",
    "tiles/1/x0/z0.png",
    "settings.json",
    "textures.json",
    "textures.json.gz",
];

fn url(path: &str) -> String {
    format!("/maps/world/{path}")
}

fn revalidate(s: &Served, path: &str, accept: &[(&str, &str)], tag: &str) -> Reply {
    let mut h = accept.to_vec();
    h.push(("If-None-Match", tag));
    get(s.addr, &url(path), &h)
}

/// Every path and coding: a tag, a 304 for it, and no cross-match between representations.
fn assert_revalidates(s: &Served) {
    for path in PATHS {
        let gz = get(s.addr, &url(path), &[GZIP]);
        let plain = get(s.addr, &url(path), &[]);
        assert_eq!((gz.status, plain.status), (200, 200), "{path}");
        let (tg, tp) = (gz.header("etag").expect(path).to_owned(), plain.header("etag").expect(path).to_owned());
        assert!(tg.starts_with('"') && tg.ends_with('"'), "{tg}");
        let coded = gz.header("content-encoding").is_some() || path.ends_with(".gz");
        assert_eq!(tg != tp, gz.header("content-encoding").is_some(), "{path}: one tag per representation");
        assert_eq!(tg.ends_with("-gzip\""), coded, "{path} {tg}");
        for (accept, tag) in [(&[GZIP][..], &tg), (&[], &tp)] {
            let r = revalidate(s, path, accept, tag);
            assert_eq!((r.status, r.body.len()), (304, 0), "{path} {accept:?}");
            assert_eq!(r.header("etag"), Some(tag.as_str()));
            assert_eq!(revalidate(s, path, accept, &format!("W/{tag}")).status, 304);
        }
        if tg != tp {
            assert_eq!(revalidate(s, path, &[], &tg).status, 200, "{path}: gzip tag for an identity client");
        }
    }
}

fn assert_rewrite_changes_tag(s: &Served, map: &dyn MapStorage) {
    let path = "tiles/0/x0/z0.prbm";
    let old = get(s.addr, &url(path), &[GZIP]).header("etag").unwrap().to_owned();
    map.write_grid(GridKey::Hires, (0, 0), &prbm(2)).unwrap();
    let r = revalidate(s, path, &[GZIP], &old);
    assert_eq!(r.status, 200, "rewritten tile");
    assert_eq!(r.decoded(), prbm(2));
    let new = r.header("etag").unwrap().to_owned();
    assert_ne!(new, old);
    assert_eq!(revalidate(s, path, &[GZIP], &new).status, 304);
    map.write_grid(GridKey::Hires, (0, 0), &prbm(2)).unwrap();
    assert_eq!(revalidate(s, path, &[GZIP], &new).status, 200, "every write is a new version");
    map.delete_grid(GridKey::Hires, (0, 0)).unwrap();
    assert_eq!(revalidate(s, path, &[GZIP], "*").status, 204);
}

#[test]
fn compat_file_storage() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path().join("maps"), Compression::Gzip).map("world").unwrap();
    fill(map.as_ref());
    let s = serve(map.clone(), true, dir.path());
    assert_revalidates(&s);
    assert_rewrite_changes_tag(&s, map.as_ref());
}

#[test]
fn compat_file_storage_uncompressed() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path().join("maps"), Compression::None).map("world").unwrap();
    fill(map.as_ref());
    let s = serve(map.clone(), true, dir.path());
    assert_revalidates(&s);
}

#[test]
fn optimized_file_storage() {
    let dir = tempfile::tempdir().unwrap();
    let storage = FileStorage::open(dir.path().join("maps"), Compression::Gzip, Format::Optimized, false).unwrap();
    let map = storage.map("world").unwrap();
    fill(map.as_ref());
    let s = serve(map.clone(), true, dir.path());
    assert_revalidates(&s);
    assert_rewrite_changes_tag(&s, map.as_ref());
}

#[test]
fn sql_storage_sends_no_validators() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = SqlConfig::new(format!("sqlite:{}", dir.path().join("bm.db").display()));
    let storage = SqlStorage::connect(&config, rt.handle().clone()).unwrap();
    let map = storage.map("world").unwrap();
    fill(map.as_ref());
    let s = serve(map, true, dir.path());
    for path in PATHS {
        let r = get(s.addr, &url(path), &[GZIP, ("If-None-Match", "*")]);
        assert_eq!((r.status, r.header("etag")), (200, None), "{path}");
    }
    drop(s);
    storage.close();
}

#[test]
fn etags_off_keeps_full_replies_and_still_answers_304() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path().join("maps"), Compression::Gzip).map("world").unwrap();
    fill(map.as_ref());
    let (on, off) = (serve(map.clone(), true, dir.path()), serve(map, false, dir.path()));
    for path in PATHS {
        for accept in [&[GZIP][..], &[]] {
            let (a, b) = (get(on.addr, &url(path), accept), get(off.addr, &url(path), accept));
            let mut names: Vec<String> = b.headers.iter().map(|(k, _)| k.to_ascii_lowercase()).collect();
            names.sort();
            let want = if b.header("content-encoding").is_some() {
                ["connection", "content-encoding", "content-length", "content-type", "server"].as_slice()
            } else {
                ["connection", "content-length", "content-type", "server"].as_slice()
            };
            assert_eq!(names, want, "{path} {accept:?}");
            assert_eq!((&a.body, a.header("content-encoding")), (&b.body, b.header("content-encoding")));
            let r = revalidate(&off, path, accept, a.header("etag").unwrap());
            assert_eq!(r.status, 304, "{path}");
        }
    }
}
