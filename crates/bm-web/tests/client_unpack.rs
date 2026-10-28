//! Client-side unpacking (docs/18): a client that asks for it gets the hires tiles of an optimized storage as
//! stored and unpacks them to the PRBM everyone else is sent; `index.html` loads the script that asks, only while
//! such a storage is served; and the server option turns all of it off.

mod common;

use std::path::Path;
use std::sync::Arc;

use bm_format::compact::BodyUnpacker;
use bm_storage::{Format, MapStorage};
use bm_web::{MapRoute, WebApp, WebOptions};
use common::{PRBM_TYPE, RAW_TILE, Served, TILE, UNPACKS, UNPACKS_FRAMES, get, packing_map as map, prbm};

const MEDIA_TYPE: Option<&str> = Some("application/vnd.bluemap.bmq3");
const FRAME_TYPE: Option<&str> = Some("application/vnd.bluemap.bmq3+zstd");
const CODINGS: [(&str, Option<&str>); 3] =
    [("gzip, deflate, br, zstd", Some("zstd")), ("gzip", Some("gzip")), ("identity", None)];

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

/// The current script decodes zstd frames itself: a browser without zstd gets the stored frame, untouched.
#[test]
fn the_frame_goes_as_stored_to_scripts_that_decode_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized), dir.path(), true);
    let (mut unpacker, mut out) = (BodyUnpacker::default(), Vec::new());
    let zstd_reply = get(s.addr, TILE, &[UNPACKS_FRAMES, ("Accept-Encoding", "gzip, deflate, br, zstd")]);
    assert_eq!((zstd_reply.header("content-type"), zstd_reply.header("content-encoding")), (MEDIA_TYPE, Some("zstd")));
    for accept_encoding in ["gzip, deflate, br", "identity"] {
        let r = get(s.addr, TILE, &[UNPACKS_FRAMES, ("Accept-Encoding", accept_encoding)]);
        assert_eq!((r.status, r.header("content-type")), (200, FRAME_TYPE), "{accept_encoding}");
        assert_eq!((r.header("content-encoding"), r.header("vary")), (None, Some("Accept, Accept-Encoding")));
        assert!(r.body == zstd_reply.body, "the stored frame, byte for byte");
        let body = bm_compress::Compression::Zstd.decompress(&r.body, 1 << 30).unwrap();
        unpacker.unpack_into(&body, &mut out).unwrap();
        assert!(out == prbm(), "{accept_encoding}");

        let tag = r.header("etag").unwrap();
        assert!(tag.ends_with("-bmq3-frame\""), "{tag}");
        let inm = ("If-None-Match", tag);
        assert_eq!(get(s.addr, TILE, &[UNPACKS_FRAMES, ("Accept-Encoding", accept_encoding), inm]).status, 304);
        assert_eq!(get(s.addr, TILE, &[("Accept-Encoding", accept_encoding), inm]).status, 200, "PRBM client");
        let raw = get(s.addr, RAW_TILE, &[UNPACKS_FRAMES, ("Accept-Encoding", accept_encoding)]);
        assert_eq!((raw.header("content-type"), raw.decoded()), (PRBM_TYPE, b"not a PRBM".to_vec()));
    }
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
    assert_eq!(js.header("cache-control"), Some("public, max-age=31536000, immutable"));
    assert!(String::from_utf8_lossy(&js.body).contains("application/vnd.bluemap.bmq3"));
    let old_build = get(packed.addr, "/assets/bluemap-rs-unpack-0123abcd.js", &[]);
    assert!((old_build.status, &old_build.body) == (200, &js.body), "an index cached before an upgrade names it");
    let plain_index = get(serve(map(&compat, Format::Compat), &compat, true).addr, "/", &[]);
    assert_eq!(script_tag(&plain_index.body), None);
    assert_ne!(index.header("etag"), plain_index.header("etag"));
    assert_eq!(plain_index.header("cache-control"), None, "additional-headers decide, like Java");

    let compat_served = serve(map(&compat, Format::Compat), &compat, true);
    assert_eq!(get(compat_served.addr, &format!("/{script}"), &[]).status, 404);
    let r = get(compat_served.addr, TILE, &[UNPACKS, ("Accept-Encoding", "gzip")]);
    assert_eq!((r.header("content-type"), r.header("vary")), (PRBM_TYPE, Some("Accept-Encoding")));
}

/// The script's hash is in the page, so the page must revalidate, and an upgrade must change its validator.
#[test]
fn index_with_the_script_revalidates() {
    let dir = tempfile::tempdir().unwrap();
    let s = serve(map(dir.path(), Format::Optimized), dir.path(), true);
    let index = get(s.addr, "/index.html", &[]);
    assert_eq!(index.header("cache-control"), Some("no-cache"));
    let (etag, modified) = (index.header("etag").unwrap(), index.header("last-modified").unwrap());
    let script = script_tag(&index.body).unwrap();
    let hash = script.trim_start_matches("assets/bluemap-rs-unpack-").trim_end_matches(".js");
    assert!(etag.ends_with(&format!("-{hash}")), "{etag}");
    let again = get(s.addr, "/index.html", &[("If-None-Match", etag), ("If-Modified-Since", modified)]);
    assert_eq!((again.status, again.header("cache-control")), (304, Some("no-cache")));
    // what a browser holding the page of a build with another script sends
    let stale_tag = etag.replace(hash, "0123abcd");
    let older = get(s.addr, "/index.html", &[("If-None-Match", &stale_tag), ("If-Modified-Since", modified)]);
    assert_eq!(older.status, 200, "same file, same mtime, other script");
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
