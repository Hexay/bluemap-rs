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
    /// Relative to the webroot; carries a hash of the body, so a new build is a new URL.
    pub path: String,
    pub body: Bytes,
}

pub(crate) static SCRIPT: LazyLock<Script> = LazyLock::new(|| {
    let wasm = STANDARD.encode(include_bytes!("../client/unpack.wasm"));
    let body = include_str!("../client/unpack.js").replace(WASM_PLACEHOLDER, &wasm);
    let mut hash = DefaultHasher::new();
    body.hash(&mut hash);
    Script { path: format!("assets/{SCRIPT_STEM}-{:08x}.js", hash.finish() as u32), body: body.into() }
});

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

/// `index_html` with the client script loaded ahead of the webapp's own (a module, so it runs later anyway);
/// `None` if there is nothing to add it in front of or it is there already.
pub(crate) fn with_script(index_html: &[u8]) -> Option<Vec<u8>> {
    let html = std::str::from_utf8(index_html).ok()?;
    let at = html.find("<script").filter(|_| !html.contains(SCRIPT_STEM))?;
    Some(format!("{}<script src=\"./{}\"></script>\n      {}", &html[..at], SCRIPT.path, &html[at..]).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_goes_in_front_of_the_webapp_once() {
        let html = b"<head><title>x</title>\n<script type=\"module\" src=\"./assets/index.js\"></script></head>";
        let made = String::from_utf8(with_script(html).unwrap()).unwrap();
        let tag = format!("<script src=\"./{}\"></script>", SCRIPT.path);
        assert!(made.find(&tag).unwrap() < made.find("type=\"module\"").unwrap());
        assert_eq!(made.replace(&tag, "").replace("\n      <script type", "<script type"), String::from_utf8_lossy(html));
        assert_eq!(with_script(made.as_bytes()), None);
        assert_eq!(with_script(b"<html></html>"), None);
        assert!(!SCRIPT.body.windows(WASM_PLACEHOLDER.len()).any(|w| w == WASM_PLACEHOLDER.as_bytes()));
    }

    #[test]
    fn accept_names_the_media_type() {
        let accept = |v: &'static str| [(ACCEPT, http::HeaderValue::from_static(v))].into_iter().collect::<HeaderMap>();
        assert!(requested(&accept("Application/vnd.bluemap.BMQ3, */*")));
        assert!(!requested(&accept("*/*")));
        assert!(!requested(&HeaderMap::new()));
    }
}
