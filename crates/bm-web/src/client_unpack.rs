//! Client-side unpacking of packed hires tiles (docs/18). A storage that packs its hires tiles normally has the
//! server unpack and recompress each one; a client that names [`MEDIA_TYPE`] in `Accept` gets the stored model
//! body instead and unpacks it itself. The webapp is taught to by one script (`client/unpack.js`, a WASM build of
//! the unpacker inside) that the server adds to `index.html` while it serves such a storage.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bm_storage::Compression;
use bytes::Bytes;
use http::HeaderMap;
use http::header::ACCEPT;

/// `Content-Type` of a model body, and what a client puts in `Accept` to get one.
pub(crate) const MEDIA_TYPE: &str = "application/vnd.bluemap.bmq3";
/// In the script's file name; an `index.html` that already mentions it is left alone.
const SCRIPT_STEM: &str = "bluemap-rs-unpack";
const WASM_PLACEHOLDER: &str = "__UNPACK_WASM_BASE64__";

pub(crate) struct Script {
    /// Relative to the webroot; carries [`Script::hash`], so a new build is a new URL.
    pub path: String,
    /// Of the body, 8 hex digits.
    pub hash: String,
    pub body: Bytes,
}

pub(crate) static SCRIPT: LazyLock<Script> = LazyLock::new(|| {
    let wasm = STANDARD.encode(include_bytes!("../client/unpack.wasm"));
    let body = include_str!("../client/unpack.js").replace(WASM_PLACEHOLDER, &wasm);
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    let hash = format!("{:08x}", hasher.finish() as u32);
    Script { path: format!("assets/{SCRIPT_STEM}-{hash}.js"), hash, body: body.into() }
});

/// [`SCRIPT`]'s path under any build's hash: an `index.html` cached before an upgrade still names the old one.
pub(crate) fn is_script_path(rel: &str) -> bool {
    let hash = rel.strip_prefix("assets/").and_then(|r| r.strip_prefix(SCRIPT_STEM)).and_then(|r| r.strip_prefix('-'));
    hash.and_then(|h| h.strip_suffix(".js")).is_some_and(|h| h.len() == 8 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether the request asks for model bodies.
pub(crate) fn requested(headers: &HeaderMap) -> bool {
    let names_it = |v: &str| v.to_ascii_lowercase().contains(MEDIA_TYPE);
    headers.get_all(ACCEPT).iter().any(|v| v.to_str().is_ok_and(names_it))
}

/// The ETag suffix of a model body sent in `coding`, distinct from every PRBM representation's.
pub(crate) fn etag_coding(coding: Compression) -> &'static str {
    match coding {
        Compression::Zstd => "bmq3-zstd",
        Compression::Gzip => "bmq3-gzip",
        _ => "bmq3",
    }
}

/// `index_html` with the client script loaded ahead of the webapp's own; `None` if there is nothing to add it in
/// front of or it is there already. `defer`: it downloads alongside the webapp's module script instead of
/// holding up the parser for a round trip, and deferred and module scripts run in document order, ours first.
pub(crate) fn with_script(index_html: &[u8]) -> Option<Vec<u8>> {
    let html = std::str::from_utf8(index_html).ok()?;
    let at = html.find("<script").filter(|_| !html.contains(SCRIPT_STEM))?;
    Some(format!("{}{}\n      {}", &html[..at], tag(), &html[at..]).into_bytes())
}

fn tag() -> String {
    format!("<script src=\"./{}\" defer></script>", SCRIPT.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_goes_in_front_of_the_webapp_once() {
        let html = b"<head><title>x</title>\n<script type=\"module\" src=\"./assets/index.js\"></script></head>";
        let made = String::from_utf8(with_script(html).unwrap()).unwrap();
        let tag = tag();
        assert!(made.find(&tag).unwrap() < made.find("type=\"module\"").unwrap());
        assert_eq!(
            made.replace(&tag, "").replace("\n      <script type", "<script type"),
            String::from_utf8_lossy(html)
        );
        assert_eq!(with_script(made.as_bytes()), None);
        assert_eq!(with_script(b"<html></html>"), None);
        assert!(!SCRIPT.body.windows(WASM_PLACEHOLDER.len()).any(|w| w == WASM_PLACEHOLDER.as_bytes()));
    }

    #[test]
    fn script_paths_of_any_build() {
        assert!(is_script_path(&SCRIPT.path));
        assert!(is_script_path("assets/bluemap-rs-unpack-0123abcd.js"));
        for other in [
            "assets/bluemap-rs-unpack-0123abc.js",
            "assets/bluemap-rs-unpack-0123abcg.js",
            "bluemap-rs-unpack-0123abcd.js",
            "assets/bluemap-rs-unpack-0123abcd.js.map",
        ] {
            assert!(!is_script_path(other), "{other}");
        }
    }

    #[test]
    fn accept_names_the_media_type() {
        let accept = |v: &'static str| [(ACCEPT, http::HeaderValue::from_static(v))].into_iter().collect::<HeaderMap>();
        assert!(requested(&accept("Application/vnd.bluemap.BMQ3, */*")));
        assert!(!requested(&accept("*/*")));
        assert!(!requested(&HeaderMap::new()));
    }
}
