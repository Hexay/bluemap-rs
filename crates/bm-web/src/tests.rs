use bm_storage::{Compression, Stored};
use http::HeaderMap;

use crate::encoding::{Accepted, Encoded, encode};
use crate::javafmt::{Arg, JavaFormat};
use crate::map_handler::parse_tile;
use crate::paths::{Resolved, java_query_string, resolve};
use crate::http_date::java_http_date;
use crate::{content_type, java_host_address};

#[test]
fn log_format_matches_java() {
    let f = JavaFormat::compile(r#"%1$s "%3$s %4$s %5$s" %6$s %7$s"#).unwrap();
    let args: Vec<Arg> = ["127.0.0.1", "1.2.3.4", "GET", "/x?", "HTTP/1.1"]
        .map(|s| Arg::Str(s.into()))
        .into_iter()
        .chain([Arg::Int(404), Arg::Str("Not Found".into())])
        .collect();
    assert_eq!(f.format(&args), r#"127.0.0.1 "GET /x? HTTP/1.1" 404 Not Found"#);
    let g = JavaFormat::compile("%s|%<S|%-5d|%.1s%%%2$s").unwrap();
    assert_eq!(g.format(&[Arg::Str("ab".into()), Arg::Int(7), Arg::Str("qrs".into())]), "ab|AB|7    |q%7");
    assert!(JavaFormat::compile("%q").is_err());
    assert!(JavaFormat::compile("%tQ%").is_err());
}

#[test]
fn query_strings_are_reencoded_like_java() {
    assert_eq!(java_query_string(None), "");
    assert_eq!(java_query_string(Some("a=b%20c&d")), "a=b+c&d");
    assert_eq!(java_query_string(Some("a=1&&b=x/y&a=2")), "a=2&b=x%2Fy");
}

#[test]
fn webroot_paths_resolve_like_java() {
    assert_eq!(resolve(""), Resolved::Inside(String::new()));
    assert_eq!(resolve("a/./b//c/../d"), Resolved::Inside("a/b/d".into()));
    assert_eq!(resolve("../x"), Resolved::Outside);
    assert_eq!(resolve("a/../../x"), Resolved::Outside);
    assert_eq!(resolve("/etc/passwd"), Resolved::Outside);
    if cfg!(windows) {
        assert_eq!(resolve("..\\x"), Resolved::Outside);
        assert_eq!(resolve("C:/Windows/win.ini"), Resolved::Outside);
        assert_eq!(resolve("index.html::$DATA"), Resolved::Invalid);
        assert_eq!(resolve("index.html."), Resolved::Invalid);
        assert_eq!(resolve("assets/nul.txt"), Resolved::Invalid);
        assert_eq!(resolve("a\u{1}b"), Resolved::Invalid);
    }
}

#[test]
#[cfg(windows)]
fn etag_path_hash_matches_java_windows_path() {
    use crate::paths::java_path_hash;
    // ETags captured from Java BlueMap 5.28 serving this webroot (hash = middle 8 hex digits)
    let root = std::path::Path::new(r"C:\Users\hexay\bluemap-rs\work\bluemap\vanilla\web");
    assert_eq!(format!("{:x}", java_path_hash(&root.join("index.html"))), "8d85a067");
    assert_eq!(format!("{:x}", java_path_hash(&root.join("settings.json"))), "cf90f6d3");
    assert_eq!(format!("{:x}", java_path_hash(&root.join(r"lang\en.conf"))), "e5d36e5b");
}

#[test]
fn http_dates_use_javas_unpadded_day() {
    assert_eq!(java_http_date(0x1a1_12a4_535f), "Tue, 6 Oct 2026 19:15:21 GMT");
}

#[test]
fn tile_paths_parse_like_the_java_regex() {
    assert_eq!(parse_tile("tiles/0/x1/2/z-3.prbm"), Some(Ok((0, 12, -3))));
    assert_eq!(parse_tile("tiles/1/x-1/z0/whatever.png"), Some(Ok((1, -1, 0))));
    assert_eq!(parse_tile("tiles/0/x0/z0"), Some(Ok((0, 0, 0))));
    assert_eq!(parse_tile("tiles/0/1/x0/z0"), Some(Err(())));
    assert_eq!(parse_tile("tiles/0/x-z0"), None);
    assert_eq!(parse_tile("tiles/0/x-/z0"), Some(Err(())));
    assert_eq!(parse_tile("tiles/0/x99999999999/z0"), Some(Err(())));
    assert_eq!(parse_tile("tiles/x0/z0"), None);
    assert_eq!(parse_tile("tiles//x0/z0"), None);
    assert_eq!(parse_tile("tiles/0/x0/z0\n"), None);
    assert_eq!(parse_tile("settings.json"), None);
}

fn accepted(v: &str) -> Accepted {
    let mut h = HeaderMap::new();
    h.insert("accept-encoding", v.parse().unwrap());
    Accepted::from_headers(&h)
}

#[test]
fn accept_encoding_tokens() {
    assert!(accepted("gzip, deflate, br, zstd").accepts("zstd"));
    assert!(accepted("GZIP;q=0.5").accepts("gzip"));
    assert!(!accepted("gzip;q=0").accepts("gzip"));
    assert!(!accepted("x-gzip").accepts("gzip"));
    assert!(!Accepted::default().accepts("gzip"));
}

#[test]
fn encoding_negotiation_follows_map_storage_request_handler() {
    let raw = b"{\"a\":1}".to_vec();
    let gz = Compression::Gzip.compress(&raw).unwrap();
    let stored = |data: &Vec<u8>, compression| Stored { data: data.clone(), compression };
    let gunzip = |e: Encoded| Compression::Gzip.decompress(&e.body, 1 << 20).unwrap();

    let pass = encode(stored(&gz, Compression::Gzip), false, false, &accepted("gzip")).unwrap();
    assert_eq!((pass.body.to_vec(), pass.content_encoding), (gz.clone(), Some("gzip")));
    let identity = encode(stored(&gz, Compression::Gzip), false, false, &Accepted::default()).unwrap();
    assert_eq!((identity.body.to_vec(), identity.content_encoding), (raw.clone(), None));
    let again = encode(stored(&gz, Compression::Gzip), false, false, &Accepted::default()).unwrap();
    assert_eq!(again.body, raw, "served from the transcode cache");
    let other = Compression::Gzip.compress(b"{\"a\":2}").unwrap();
    assert_eq!(encode(stored(&other, Compression::Gzip), false, false, &Accepted::default()).unwrap().body, b"{\"a\":2}".as_slice());
    let regz = encode(stored(&raw, Compression::None), false, false, &accepted("gzip")).unwrap();
    assert_eq!(regz.content_encoding, Some("gzip"));
    assert_eq!(gunzip(regz), raw);
    let png = encode(stored(&raw, Compression::None), true, false, &accepted("gzip")).unwrap();
    assert_eq!((png.body.to_vec(), png.content_encoding), (raw.clone(), None));
    let gz_url = encode(stored(&raw, Compression::None), true, true, &Accepted::default()).unwrap();
    assert_eq!(gz_url.content_encoding, None);
    assert_eq!(gunzip(gz_url), raw);
    let zstd = Compression::Zstd.compress(&raw).unwrap();
    let transcoded = encode(stored(&zstd, Compression::Zstd), false, false, &accepted("gzip")).unwrap();
    assert_eq!(gunzip(transcoded), raw);
}

#[test]
fn content_types() {
    assert_eq!(content_type::static_file("en.conf"), "text/plain");
    assert_eq!(content_type::static_file("index-x.js"), "text/javascript");
    assert_eq!(content_type::static_file("Quicksand.ttf"), "text/plain");
    assert_eq!(content_type::map_item("assets/a.ttf"), "font/ttf");
    assert_eq!(content_type::map_item("assets/a.b/c"), "application/octet-stream");
    assert_eq!(content_type::map_item("live/players.json"), "application/json");
}

#[test]
fn host_addresses_print_like_java() {
    assert_eq!(java_host_address("::1".parse().unwrap()), "0:0:0:0:0:0:0:1");
    assert_eq!(java_host_address("::ffff:10.0.0.1".parse().unwrap()), "10.0.0.1");
    assert_eq!(java_host_address("fe80::1:abcd".parse().unwrap()), "fe80:0:0:0:0:0:1:abcd");
}
