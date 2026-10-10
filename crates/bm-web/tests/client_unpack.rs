//! Client-side unpacking (docs/18): a client that asks for it gets the hires tiles of an optimized storage as
//! stored and unpacks them to the PRBM everyone else is sent; `index.html` loads the script that asks, only while
//! such a storage is served; and the server option turns all of it off.

mod common;

use std::path::Path;
use std::sync::Arc;

use bm_format::compact::BodyUnpacker;
use bm_format::prbm::TileModel;
use bm_storage::{Compression, FileStorage, Format, GridKey, MapStorage, Storage};
use bm_web::{MapRoute, WebApp, WebOptions};
use common::{Served, get};

const UNPACKS: (&str, &str) = ("Accept", "application/vnd.bluemap.bmq3, */*");
const MEDIA_TYPE: Option<&str> = Some("application/vnd.bluemap.bmq3");
const PRBM_TYPE: Option<&str> = Some("application/octet-stream");
const TILE: &str = "/maps/world/tiles/0/x0/z0.prbm";
const RAW_TILE: &str = "/maps/world/tiles/0/x1/z0.prbm";
const CODINGS: [(&str, Option<&str>); 3] =
    [("gzip, deflate, br, zstd", Some("zstd")), ("gzip", Some("gzip")), ("identity", None)];

fn prbm() -> Vec<u8> {
    let mut m = TileModel::default();
    for q in 0..40 {
        let (x, z) = ((q % 8) as f32, (q / 8) as f32);
        for tri in [[[x, 64., z], [x, 64., z + 1.], [x + 1., 64., z + 1.]], [[x, 64., z], [x + 1., 64., z + 1.], [x + 1., 64., z]]]
        {
            tri.iter().for_each(|p| m.position.extend(p));
            m.color.extend([0.5, 0.5, 1.]);
            m.sunlight.push(15);
            m.blocklight.push(2);
            m.material.push(q / 10);
        }
        m.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
        m.ao.extend([1., 1., 0.5, 1., 0.5, 0.75]);
    }
    let mut out = Vec::new();
    m.write_prbm(&mut out).unwrap();
    out
}

/// A map with one tile the model holds and one it stores raw.
fn map(root: &Path, format: Format) -> Arc<dyn MapStorage> {
    let map = FileStorage::open(root.join("maps"), Compression::Gzip, format, false).unwrap().map("world").unwrap();
    map.write_grid(GridKey::Hires, (0, 0), &prbm()).unwrap();
    map.write_grid(GridKey::Hires, (1, 0), b"not a PRBM").unwrap();
    map
}

fn serve(storage: Arc<dyn MapStorage>, web: &Path, client_unpack: bool) -> Served {
    let mut options = WebOptions::new(web);
    options.client_unpack = client_unpack;
    let mut app = WebApp::new(options).unwrap();
    app.add_map("world", MapRoute { storage, live: None }).unwrap();
    Served::start(app)
}

fn script_tag(index_html: &[u8]) -> Option<String> {
    let html = String::from_utf8_lossy(index_html);
    let at = html.find("<script src=\"./assets/bluemap-rs-unpack-")?;
    Some(html[at + 15..].split('"').next().unwrap().to_owned())
}

#[test]
fn asked_for_tiles_arrive_packed_in_every_coding() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized), dir.path(), true);
    let (mut unpacker, mut out) = (BodyUnpacker::default(), Vec::new());
    for (accept_encoding, coding) in CODINGS {
        let accept = [UNPACKS, ("Accept-Encoding", accept_encoding)];
        // twice: transcoded bodies come from the cache the second time
        for _ in 0..2 {
            let r = get(s.addr, TILE, &accept);
            assert_eq!((r.status, r.header("content-type")), (200, MEDIA_TYPE), "{accept_encoding}");
            assert_eq!(r.header("content-encoding"), coding);
            assert_eq!(r.header("vary"), Some("Accept, Accept-Encoding"));
            unpacker.unpack_into(&r.decoded(), &mut out).unwrap();
            assert!(out == prbm(), "{accept_encoding}");
        }
        let raw = get(s.addr, RAW_TILE, &accept);
        assert_eq!((raw.header("content-type"), raw.decoded()), (PRBM_TYPE, b"not a PRBM".to_vec()));
        let plain = get(s.addr, TILE, &[("Accept-Encoding", accept_encoding)]);
        assert_eq!(plain.header("content-type"), PRBM_TYPE);
        assert!(plain.decoded() == prbm());
    }
    let gz = get(s.addr, &format!("{TILE}.gz"), &[UNPACKS]);
    assert!(common::gunzip(&gz.body) == prbm(), ".gz URLs stay PRBM");
}

#[test]
fn each_form_has_its_own_validator() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized), dir.path(), true);
    let zstd = ("Accept-Encoding", "gzip, deflate, br, zstd");
    let tag = get(s.addr, TILE, &[UNPACKS, zstd]).header("etag").unwrap().to_owned();
    assert!(tag.ends_with("-bmq3-zstd\""), "{tag}");
    assert_eq!(get(s.addr, TILE, &[UNPACKS, zstd, ("If-None-Match", &tag)]).status, 304);
    assert_eq!(get(s.addr, TILE, &[zstd, ("If-None-Match", &tag)]).status, 200, "a packed tag for a PRBM client");
    let raw_tag = get(s.addr, RAW_TILE, &[UNPACKS, zstd]).header("etag").unwrap().to_owned();
    assert!(raw_tag.ends_with("-zstd\"") && !raw_tag.contains("bmq3"), "{raw_tag}");
    assert_eq!(get(s.addr, RAW_TILE, &[UNPACKS, zstd, ("If-None-Match", &raw_tag)]).status, 304);
}

#[test]
fn index_loads_the_script_only_for_a_packing_storage() {
    let dir = tempfile::tempdir().unwrap();
    let (opt, compat) = (dir.path().join("opt"), dir.path().join("compat"));
    let packed = serve(map(&opt, Format::Optimized), &opt, true);
    let index = get(packed.addr, "/", &[]);
    let script = script_tag(&index.body).expect("script tag");
    assert!(String::from_utf8_lossy(&index.body).find(&script) < String::from_utf8_lossy(&index.body).find("module"));
    let js = get(packed.addr, &format!("/{script}"), &[]);
    assert_eq!((js.status, js.header("content-type")), (200, Some("text/javascript")));
    assert!(String::from_utf8_lossy(&js.body).contains("application/vnd.bluemap.bmq3"));
    let plain_index = get(serve(map(&compat, Format::Compat), &compat, true).addr, "/", &[]);
    assert_eq!(script_tag(&plain_index.body), None);
    assert_ne!(index.header("etag"), plain_index.header("etag"));

    let compat_served = serve(map(&compat, Format::Compat), &compat, true);
    assert_eq!(get(compat_served.addr, &format!("/{script}"), &[]).status, 404);
    let r = get(compat_served.addr, TILE, &[UNPACKS, ("Accept-Encoding", "gzip")]);
    assert_eq!((r.header("content-type"), r.header("vary")), (PRBM_TYPE, Some("Accept-Encoding")));
}

#[test]
fn the_server_option_turns_it_off() {
    assert!(bm_config::WebserverConfig::default().client_unpack, "webserver.conf without the key");
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized), dir.path(), false);
    assert_eq!(script_tag(&get(s.addr, "/", &[]).body), None);
    let r = get(s.addr, TILE, &[UNPACKS, ("Accept-Encoding", "gzip")]);
    assert_eq!((r.header("content-type"), r.header("vary")), (PRBM_TYPE, Some("Accept-Encoding")));
    assert!(r.decoded() == prbm());
}
