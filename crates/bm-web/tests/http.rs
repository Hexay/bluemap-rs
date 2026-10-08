//! End-to-end behaviour over real sockets, against a temp webroot with file storage.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use bm_storage::{Compression, FileStorage, GridKey, ItemKey, Storage};
use bm_web::{AccessLog, Level, LiveMap, LogSink, MapRegistry, MapRoute, WebApp, WebOptions};
use common::{Capture, Served, get, gunzip, request};

const CACHE: &str = "public, max-age=86400, stale-if-error=604800";

struct Fixture {
    _dir: tempfile::TempDir,
    served: Served,
    live: Arc<LiveMap>,
    maps: MapRegistry,
    log: Capture,
    prbm: Vec<u8>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("assets/logo.png"), b"custom logo").unwrap();
    std::fs::write(web.join("settings.json"), b"{\"maps\":[\"world\"]}").unwrap();
    std::fs::write(web.join("sql.php"), b"<?php").unwrap();
    let storage = FileStorage::new(web.join("maps"), Compression::Gzip);
    let map = storage.map("world").unwrap();
    let prbm = b"PRBM-bytes".repeat(50);
    map.write_grid(GridKey::Hires, (-12, 5), &prbm).unwrap();
    map.write_grid(GridKey::Lowres(1), (0, 0), b"\x89PNG lowres").unwrap();
    map.write_item(&ItemKey::Settings, b"{\"name\":\"world\"}").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
    map.write_item(&ItemKey::Players, b"{}").unwrap();
    map.write_item(&ItemKey::asset("playerheads/a.png"), b"head").unwrap();

    let log = Capture::default();
    let mut options = WebOptions::new(&web);
    options.additional_headers =
        vec![("Cache-Control".into(), CACHE.into()), ("CDN-Cache-Control".into(), "max-age=60".into())];
    let sinks: Vec<Box<dyn LogSink>> = vec![Box::new(log.clone())];
    options.access_log = AccessLog::new(r#"%1$s %2$s "%3$s %4$s %5$s" %6$s %7$s"#, sinks).unwrap();
    let mut app = WebApp::new(options).unwrap();
    let live = Arc::new(LiveMap::new(true).with_markers());
    app.add_map("world", MapRoute { storage: map.clone(), live: Some(live.clone()) }).unwrap();
    app.add_map("plain", MapRoute { storage: map, live: None }).unwrap();
    let maps = app.maps();
    Fixture { _dir: dir, served: Served::start(app), live, maps, log, prbm }
}

#[test]
fn static_files_disk_wins_over_embedded() {
    let f = fixture();
    let a = f.served.addr;
    let index = get(a, "/", &[]);
    assert_eq!(index.status, 200);
    assert_eq!(index.header("content-type"), Some("text/html"));
    assert!(String::from_utf8_lossy(&index.body).contains("<html"), "embedded index.html");
    assert_eq!(index.header("content-length").map(|l| l.parse::<usize>().unwrap()), Some(index.body.len()));
    assert!(index.header("etag").is_some() && index.header("last-modified").is_some());
    assert_eq!(index.header("server"), Some("BlueMap/5.28"));
    assert_eq!(index.header("cache-control"), Some(CACHE));

    assert_eq!(get(a, "/assets/logo.png", &[]).body, b"custom logo");
    assert_eq!(get(a, "/lang/en.conf", &[]).header("content-type"), Some("text/plain"));
    assert_eq!(get(a, "/settings.json", &[]).body, b"{\"maps\":[\"world\"]}");

    let redirect = get(a, "/assets?x=1%202", &[]);
    assert_eq!((redirect.status, redirect.header("location")), (303, Some("/assets/?x=1+2")));
    assert_eq!(get(a, "/lang", &[]).header("location"), Some("/lang/"), "embedded-only dir redirects");
    assert_eq!(get(a, "/sql.php", &[]).status, 403);
    assert_eq!(get(a, "/missing.js", &[]).status, 404);
    assert_eq!(get(a, "/../Cargo.toml", &[]).status, 403);
    assert_eq!(get(a, "/%2e%2e/%2e%2e/x", &[]).status, 403);
    assert_eq!(request(a, "POST", "/index.html", &[("Content-Length", "0")]).status, 400);
    let not_found = get(a, "/nope", &[]);
    assert_eq!((not_found.body.len(), not_found.header("cache-control")), (0, Some(CACHE)));

    let etag = index.header("etag").unwrap().to_owned();
    assert_eq!(get(a, "/", &[("If-None-Match", &etag)]).status, 304);
    let lm = index.header("last-modified").unwrap().to_owned();
    assert_eq!(get(a, "/", &[("If-Modified-Since", &lm)]).status, 304);
}

#[test]
fn static_files_gzip_and_disk_changes() {
    let f = fixture();
    let a = f.served.addr;
    let identity = get(a, "/index.html", &[]);
    assert_eq!((identity.header("content-encoding"), identity.header("vary")), (None, None));
    let gz = get(a, "/index.html", &[("Accept-Encoding", "br, gzip;q=0.8")]);
    assert_eq!((gz.header("content-encoding"), gz.header("vary")), (Some("gzip"), Some("Accept-Encoding")));
    assert_eq!(gunzip(&gz.body), identity.body);
    let tag = gz.header("etag").unwrap();
    assert_eq!(tag, format!("{}-gzip", identity.header("etag").unwrap()));
    assert_eq!(get(a, "/index.html", &[("If-None-Match", tag)]).status, 304);
    assert_eq!(get(a, "/index.html", &[("Accept-Encoding", "gzip;q=0")]).header("content-encoding"), None);
    assert_eq!(get(a, "/assets/logo.png", &[("Accept-Encoding", "gzip")]).header("content-encoding"), None);

    let web = f._dir.path().join("web");
    assert_eq!(get(a, "/settings.json", &[]).body, b"{\"maps\":[\"world\"]}");
    assert_eq!(get(a, "/new.txt", &[]).status, 404);
    std::fs::write(web.join("settings.json"), b"{\"maps\":[]}").unwrap();
    std::fs::write(web.join("new.txt"), b"new").unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(get(a, "/settings.json", &[]).body, b"{\"maps\":[]}");
    assert_eq!(get(a, "/new.txt", &[]).body, b"new");
}

#[test]
fn map_data_encoding_negotiation() {
    let f = fixture();
    let a = f.served.addr;
    let gz = [("Accept-Encoding", "gzip, deflate, br, zstd")];
    let tile = get(a, "/maps/world/tiles/0/x-1/2/z5.prbm", &gz);
    assert_eq!((tile.status, tile.header("content-encoding")), (200, Some("gzip")));
    assert_eq!(tile.header("content-type"), Some("application/octet-stream"));
    assert_eq!(gunzip(&tile.body), f.prbm);
    assert_eq!(tile.header("content-length").unwrap(), tile.body.len().to_string());

    let identity = get(a, "/maps/world/tiles/0/x-1/2/z5.prbm", &[]);
    assert_eq!((identity.header("content-encoding"), identity.body.clone()), (None, f.prbm.clone()));
    let gz_url = get(a, "/maps/world/tiles/0/x-1/2/z5.prbm.gz", &gz);
    assert_eq!(gz_url.header("content-encoding"), None);
    assert_eq!(gunzip(&gz_url.body), f.prbm);

    let lowres = get(a, "/maps/world/tiles/1/x0/z0.png", &gz);
    assert_eq!((lowres.header("content-type"), lowres.header("content-encoding")), (Some("image/png"), None));
    assert_eq!(lowres.body, b"\x89PNG lowres");
    assert_eq!(get(a, "/maps/world/tiles/0/x9/z9.prbm", &gz).status, 204);
    assert_eq!(get(a, "/maps/world/tiles/7/x9/z9.png", &gz).status, 204);

    let settings = get(a, "/maps/world/settings.json", &gz);
    assert_eq!(
        (settings.header("content-encoding"), settings.decoded()),
        (Some("gzip"), b"{\"name\":\"world\"}".to_vec())
    );
    assert_eq!(get(a, "/maps/world/textures.json", &[]).body, b"[]");
    assert_eq!(get(a, "/maps/world/textures.json.gz", &[]).header("content-encoding"), None);
    assert_eq!(gunzip(&get(a, "/maps/world/textures.json.gz", &[]).body), b"[]");
    assert_eq!(get(a, "/maps/world/assets/playerheads/a.png", &[]).body, b"head");
    assert_eq!(get(a, "/maps/world/assets/../../settings.json", &[]).status, 404);
    assert_eq!(get(a, "/maps/world/nothing", &[]).status, 404);
    assert_eq!(get(a, "/maps/world/", &[]).status, 404);
    assert_eq!(get(a, "/maps/other/settings.json", &[]).status, 404);
    // storage maps without live data serve the stored live files
    assert_eq!(get(a, "/maps/plain/live/players.json", &[]).body, b"{}");
    assert_eq!(get(a, "/maps/plain/live/sse", &[]).status, 404);
}

#[test]
fn live_json_and_sse() {
    let f = fixture();
    let a = f.served.addr;
    let empty = get(a, "/maps/world/live/markers.json", &[]);
    assert_eq!((empty.status, empty.body.len()), (200, 0));
    f.live.set_markers("{\"set\":{}}");
    let markers = get(a, "/maps/world/live/markers.json", &[("Accept-Encoding", "gzip")]);
    assert_eq!(markers.body, b"{\"set\":{}}");
    assert_eq!(markers.header("content-type"), Some("application/json"));
    assert_eq!(markers.header("cache-control"), Some("no-store"), "additional-headers must not override");
    assert_eq!(markers.header("surrogate-control"), Some("no-store"));
    // players are not live for this map: storage copy
    assert_eq!(get(a, "/maps/world/live/players.json", &[]).body, b"{}");

    let mut s = TcpStream::connect(a).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(b"GET /maps/world/live/sse HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut chunk).unwrap();
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf).to_lowercase();
    assert!(head.contains("content-type: text/event-stream") && head.contains("x-accel-buffering: no"), "{head}");
    while f.live.sse_clients() == 0 {
        std::thread::yield_now();
    }
    f.live.tile_updated(3, -4, 0);
    f.live.set_markers("{\"set\":{}}");
    f.live.set_markers("a\nb");
    let want = b"event: tile\ndata: {\"x\":3,\"y\":-4,\"lod\":0}\n\nevent: marker\ndata: a\ndata: b\n\n";
    while !String::from_utf8_lossy(&buf).contains("data: b\n\n") {
        let n = s.read(&mut chunk).unwrap();
        assert!(n > 0, "stream closed early");
        buf.extend_from_slice(&chunk[..n]);
    }
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let body = common::dechunk(&buf[split + 4..]);
    assert_eq!(String::from_utf8_lossy(&body), String::from_utf8_lossy(want));
}

#[test]
fn live_routes_of_maps_loaded_later() {
    let f = fixture();
    let a = f.served.addr;
    assert_eq!(get(a, "/maps/plain/live/markers.json", &[]).status, 404, "storage has no markers.json");
    let late = Arc::new(LiveMap::new(true).with_markers());
    late.set_markers("{\"late\":{}}");
    assert!(f.maps.set_live("plain", late));
    assert_eq!(get(a, "/maps/plain/live/markers.json", &[]).body, b"{\"late\":{}}");
    assert!(!f.maps.set_live("unknown", Arc::new(LiveMap::new(false))));
    assert_eq!(get(a, "/maps/world/live/markers.json", &[]).status, 200, "other routes stay");
}

#[test]
fn access_log_uses_java_format() {
    let f = fixture();
    let a = f.served.addr;
    get(a, "/index.html?a=b%20c&d", &[("X-Forwarded-For", "1.2.3.4, 5.6.7.8")]);
    get(a, "/x%20y", &[]);
    let lines = f.log.0.lock().unwrap().clone();
    assert_eq!(lines[0], (Level::Info, r#"127.0.0.1 1.2.3.4 "GET /index.html?a=b+c&d HTTP/1.1" 200 OK"#.into()));
    assert_eq!(lines[1], (Level::Info, r#"127.0.0.1 127.0.0.1 "GET /x y? HTTP/1.1" 404 Not Found"#.into()));
}

#[test]
fn absolute_form_targets_and_bad_requests() {
    let f = fixture();
    let a = f.served.addr;
    let abs = get(a, &format!("http://{a}/maps/world/settings.json"), &[]);
    assert_eq!(abs.body, b"{\"name\":\"world\"}");
    assert_eq!(request(a, "OPTIONS", "*", &[]).status, 400);
    // an oversized body is refused without being buffered (#828)
    let big = request(a, "POST", "/maps/world/settings.json", &[("Content-Length", "100000000")]);
    assert_eq!(big.status, 413);
}

#[test]
fn bind_conflict_fails_loudly() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let first = bm_web::WebServer::bind("127.0.0.1", 0).await.unwrap();
        let port = first.local_addr().port();
        let err = bm_web::WebServer::bind("127.0.0.1", port.into()).await.err().expect("port in use");
        assert!(err.to_string().contains("failed to bind the webserver to 127.0.0.1"), "{err}");
        assert!(bm_web::WebServer::bind("127.0.0.1", 70000).await.is_err());
        assert!(bm_web::WebServer::bind("no-such-host.invalid", 0).await.is_err());
    });
}
