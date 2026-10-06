//! Wire conformance against Java BlueMap 5.28's own webserver (`-w`) on the same Java-rendered webroot.
//! `cargo test -p bm-web --test conformance -- --ignored --nocapture`
//! Needs `work/` (found by walking up from this crate, or `BLUEMAP_RS_WORK`): `downloads/jdk25`,
//! `downloads/bluemap-5.28-cli.jar`, `bluemap/vanilla/{config,web}`. The golden dirs are only read.
//!
//! Compared per request: status, all non-framing headers, body after undoing `Content-Encoding` (and gunzipped
//! for `.gz` URLs, whose gzip bytes come from different encoders). Intended differences (not compared):
//! - framing: Java always sends `Transfer-Encoding: chunked` (or `Content-Length: 0` without a body); we send the
//!   real `Content-Length` (download progress in the webapp) and none on 204/304, as HTTP requires.
//! - `Accept-Encoding` q-values: Java ignores `gzip;q=1.0` (exact token match), we honour it.
//! - requests Java can't parse (it drops the connection) get a 400 from hyper.

mod common;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bm_storage::{Compression, FileStorage, Storage};
use bm_web::{MapRoute, WebApp, WebOptions};
use common::{Reply, Served, gunzip, request};

const FRAMING: [&str; 3] = ["transfer-encoding", "content-length", "connection"];
const HEADERS: [(&str, &str); 2] =
    [("Cache-Control", "public, max-age=86400, stale-if-error=604800"), ("CDN-Cache-Control", "max-age=60")];

struct Java(Child);

impl Drop for Java {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn work_dir() -> PathBuf {
    if let Ok(w) = std::env::var("BLUEMAP_RS_WORK") {
        return w.into();
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|a| a.join("work"))
        .find(|w| w.join("downloads").is_dir())
        .expect("no work/ dir with downloads above this crate; set BLUEMAP_RS_WORK")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dst = to.join(e.file_name());
        if e.path().is_dir() { copy_dir(&e.path(), &dst) } else { std::fs::copy(e.path(), &dst).map(drop).unwrap() }
    }
}

fn start_java(work: &Path, web: &Path, dir: &Path, port: u16) -> Java {
    let config = dir.join("config");
    copy_dir(&work.join("bluemap/vanilla/config"), &config);
    let _ = std::fs::remove_file(config.join("storages/sql.conf"));
    let web_s = web.to_string_lossy().replace('\\', "/");
    std::fs::write(
        config.join("storages/file.conf"),
        format!("storage-type: file\nroot: \"{web_s}/maps\"\ncompression: gzip\n"),
    )
    .unwrap();
    let headers: String = HEADERS.iter().map(|(k, v)| format!("  \"{k}\": \"{v}\"\n")).collect();
    std::fs::write(
        config.join("webserver.conf"),
        format!(
            "enabled: true\nwebroot: \"{web_s}\"\nip: \"127.0.0.1\"\nport: {port}\nsse-enabled: true\n\
             additional-headers: {{\n{headers}}}\nlog: {{ file: \"data/logs/webserver.log\", append: false }}\n"
        ),
    )
    .unwrap();
    let java = work.join("downloads/jdk25/bin").join(if cfg!(windows) { "java.exe" } else { "java" });
    let child = Command::new(java)
        .args([
            "-jar",
            &work.join("downloads/bluemap-5.28-cli.jar").to_string_lossy(),
            "-c",
            "config",
            "-v",
            "26.3",
            "-w",
        ])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start java");
    let java = Java(child);
    let deadline = Instant::now() + Duration::from_secs(120);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "Java webserver did not come up on {port}");
        std::thread::sleep(Duration::from_millis(200));
    }
    java
}

fn start_ours(web: &Path) -> Served {
    let storage = FileStorage::new(web.join("maps"), Compression::Gzip).read_only(true);
    let mut options = WebOptions::new(web);
    options.additional_headers = HEADERS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut app = WebApp::new(options).unwrap();
    app.add_map("vanilla", MapRoute { storage: storage.map("vanilla").unwrap(), live: None }).unwrap();
    Served::start(app)
}

/// Existing tiles of a lod plus their `.gz` twins (paths relative to the map, without the storage suffix).
fn sample_tiles(web: &Path, lod: u32, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![web.join(format!("maps/vanilla/tiles/{lod}"))];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else if out.len() < n {
                let rel = e.path().strip_prefix(web.join("maps/vanilla")).unwrap().to_string_lossy().replace('\\', "/");
                out.push(rel.trim_end_matches(".gz").to_owned());
            }
        }
    }
    out
}

type Headers = Vec<(&'static str, String)>;
type Probe = (&'static str, String, Headers);

fn requests(web: &Path, port: u16) -> Vec<Probe> {
    let gz = || vec![("Accept-Encoding", "gzip, deflate, br, zstd".to_owned())];
    let mut r: Vec<Probe> = Vec::new();
    let mut add = |m: &'static str, t: &str, h: Headers| r.push((m, t.to_owned(), h));
    for t in [
        "/",
        "/index.html",
        "/settings.json",
        "/lang/en.conf",
        "/lang/settings.conf",
        "/assets/logo.png",
        "/assets/poi.svg",
        "/assets/index-BhhubAws.js",
        "/assets/index-C6zAnFcX.css",
        "/assets/Quicksand-BuVPtn-J.ttf",
        "/assets/manifest-vz4Wm4Dd.webmanifest",
        "/assets",
        "/assets/",
        "/lang",
        "/maps",
        "/maps/vanilla",
        "/sql.php",
        "/nope",
        "/nope/",
        "/../LICENSE",
        "/%2e%2e/x",
        "/assets%5c..%5c..%5cLICENSE",
        "/maps/../settings.json",
        "/index.html?x=1&y=a%20b",
        "/assets?x=1%202",
        "/index.html::$DATA",
        "/assets/../index.html",
    ] {
        add("GET", t, vec![]);
    }
    add("GET", &format!("http://127.0.0.1:{port}/index.html"), vec![]);
    add("HEAD", "/index.html", vec![]);
    add("POST", "/index.html", vec![("Content-Length", "0".into())]);
    add("POST", "/maps/vanilla/settings.json", vec![("Content-Length", "0".into())]);
    add("GET", "/index.html", vec![("If-Modified-Since", "Tue, 1 Jan 2999 00:00:00 GMT".into())]);
    add("GET", "/index.html", vec![("If-Modified-Since", "Mon, 1 Jan 2001 00:00:00 GMT".into())]);
    let mut map_paths: Vec<String> = [
        "settings.json",
        "textures.json",
        "textures.json.gz",
        "live/markers.json",
        "live/players.json",
        "live/players.json.gz",
        "live/sse",
        "",
        "nothing",
        "assets/x.png",
        "assets/../../settings.json",
        "tiles/0/1/x0/z0.prbm",
        "tiles/0/x9999/z9999.prbm",
        "tiles/0/x9999/z9999.prbm.gz",
        "tiles/2/x99/z-99.png",
        "tiles/9/x0/z0.png",
        "tiles/0/x-/z0.prbm",
        "tiles/0/x0/z0.prbm?q=1",
    ]
    .map(str::to_owned)
    .to_vec();
    for lod in 0..=3 {
        for t in sample_tiles(web, lod, if lod == 0 { 4 } else { 2 }) {
            map_paths.push(format!("{t}.gz"));
            map_paths.push(t);
        }
    }
    for p in map_paths {
        add("GET", &format!("/maps/vanilla/{p}"), vec![]);
        add("GET", &format!("/maps/vanilla/{p}"), gz());
    }
    add("GET", "/maps/other/settings.json", gz());
    r
}

fn normalized(reply: &Reply, gz_url: bool) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut headers: Vec<(String, String)> = reply
        .headers
        .iter()
        .filter(|(k, _)| !FRAMING.contains(&k.to_ascii_lowercase().as_str()))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();
    headers.sort();
    let mut body = reply.decoded();
    if gz_url && reply.status == 200 {
        body = gunzip(&body);
    }
    (reply.status, headers, body)
}

#[test]
#[ignore = "needs Java + work/ golden webroot; run explicitly"]
fn matches_java_bluemap_webserver() {
    let work = work_dir();
    let web = work.join("bluemap/vanilla/web");
    let dir = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let _java = start_java(&work, &web, dir.path(), port);
    let ours = start_ours(&web);
    let java_addr: SocketAddr = ([127, 0, 0, 1], port).into();

    let (mut same, mut diffs) = (0, Vec::new());
    for (method, target, headers) in requests(&web, port) {
        let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let target_ours = target.replace(&format!(":{port}/"), &format!(":{}/", ours.addr.port()));
        let (j, o) = (request(java_addr, method, &target, &h), request(ours.addr, method, &target_ours, &h));
        let gz_url = target.split('?').next().unwrap().ends_with(".gz");
        let (jn, on) = (normalized(&j, gz_url), normalized(&o, gz_url));
        let label = format!("{method} {target} {h:?}");
        if jn == on {
            same += 1;
            println!("same  {:3} {label}", j.status);
        } else {
            let detail = format!(
                "{label}\n  java {} {:?} body {}B\n  ours {} {:?} body {}B",
                jn.0,
                jn.1,
                jn.2.len(),
                on.0,
                on.1,
                on.2.len()
            );
            println!("DIFF  {detail}");
            diffs.push(detail);
        }
    }
    println!("{same} identical, {} different", diffs.len());
    assert!(diffs.is_empty(), "differences:\n{}", diffs.join("\n"));
}
