//! `FileRequestHandler`: the webroot, with the bundled webapp behind it (files on disk win). Lookups and bodies
//! come from [`StaticCache`]; compressible files go out gzipped to clients that accept it (Java never compresses).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use axum::body::Body;
use http::header::{
    CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LOCATION,
    VARY,
};
use http::{HeaderMap, HeaderValue, Method, Response, StatusCode};
use tokio_util::io::ReaderStream;

use crate::client_unpack::{SCRIPT, with_script};
use crate::content_type;
use crate::encoding::Accepted;
use crate::http_date::parse_http_date;
use crate::paths::{Resolved, java_path_hash, join, resolve, unnormalized};
use crate::response::{empty, header_static};
use crate::static_cache::{Content, FileEntry, StaticCache};

/// Appended to the Java ETag of a gzipped body: a strong tag must differ per representation.
const GZIP_TAG: &str = "-gzip";

pub(crate) struct StaticFiles {
    /// Normalized like Java's `webRoot.normalize()`: the ETag hashes this path's string form.
    root: PathBuf,
    cache: StaticCache,
    /// The `index.html` last served with the client script: the file it was made from, and the result.
    scripted_index: Mutex<Option<(Arc<FileEntry>, Arc<FileEntry>)>>,
}

static CLIENT_SCRIPT: LazyLock<Arc<FileEntry>> = LazyLock::new(|| FileEntry::generated(SCRIPT.body.clone()));

impl StaticFiles {
    pub fn new(root: &Path, embedded: bool) -> Self {
        let root: PathBuf = root.components().collect();
        Self { cache: StaticCache::new(&root, embedded), root, scripted_index: Mutex::default() }
    }

    /// `index` with the client script ([`crate::client_unpack`]), or as it is if the script can't be added.
    fn with_client_script(&self, index: Arc<FileEntry>) -> Arc<FileEntry> {
        let mut last = self.scripted_index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, made)) = last.as_ref().filter(|(source, _)| Arc::ptr_eq(source, &index)) {
            return made.clone();
        }
        let Content::Memory(html) = &index.content else { return index };
        let Some(html) = with_script(html) else { return index };
        let made = index.rewritten(html.into());
        *last = Some((index, made.clone()));
        made
    }

    /// `route_path` is the router's path (leading `/` stripped, `/` for the root); `query` is Java's re-encoded one.
    /// `client_unpack`: serve the client script and load it from `index.html`.
    pub async fn handle(
        &self,
        method: &Method,
        route_path: &str,
        query: &str,
        headers: &HeaderMap,
        client_unpack: bool,
    ) -> Response<Body> {
        if !method.as_str().eq_ignore_ascii_case("GET") {
            return empty(StatusCode::BAD_REQUEST);
        }
        let path = route_path.strip_prefix('/').unwrap_or(route_path);
        let path = path.strip_suffix('/').unwrap_or(path);
        let rel = match resolve(path) {
            Resolved::Inside(rel) => rel,
            Resolved::Outside => return empty(StatusCode::FORBIDDEN),
            Resolved::Invalid => return empty(StatusCode::NOT_FOUND),
        };
        let mut node = self.cache.lookup(&rel).await;
        if client_unpack && node.file.is_none() && rel == SCRIPT.path {
            node.file = Some(CLIENT_SCRIPT.clone());
        }
        if !route_path.ends_with('/') && node.is_dir {
            return redirect(path, query);
        }
        let with_index = |p: &str| if p.is_empty() { "index.html".to_owned() } else { format!("{p}/index.html") };
        // the ETag hashes the path as requested, before normalization, like Java
        let java_rel = unnormalized(path);
        let (rel, java_path, file) = match node.file {
            Some(file) => (rel, java_rel, file),
            None => {
                let index = with_index(&rel);
                match self.cache.lookup(&index).await.file {
                    Some(file) => (index, with_index(&java_rel), file),
                    None => return empty(StatusCode::NOT_FOUND),
                }
            }
        };
        if rel.ends_with(".php") {
            return empty(StatusCode::FORBIDDEN);
        }
        let file = if client_unpack && rel == "index.html" { self.with_client_script(file) } else { file };
        let etag = format!("{:x}{:x}{:x}", file.len, java_path_hash(&join(&self.root, &java_path)), file.mtime_ms);
        if not_modified(headers, file.mtime_ms, &etag) {
            return empty(StatusCode::NOT_MODIFIED);
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        let content_type = content_type::static_file(name);
        let gzip = match compressible(content_type) && Accepted::from_headers(headers).accepts("gzip") {
            true => gzipped(&file).await,
            false => None,
        };
        let (mut res, etag) = match gzip {
            Some(gz) => {
                let mut res = Response::new(Body::from(gz));
                header_static(&mut res, CONTENT_ENCODING, "gzip");
                header_static(&mut res, VARY, "Accept-Encoding");
                (res, etag + GZIP_TAG)
            }
            None => match identity_body(&file).await {
                Some(res) => (res, etag),
                None => return empty(StatusCode::NOT_FOUND),
            },
        };
        let h = res.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&etag) {
            h.insert(ETAG, v);
        }
        if let Some(lm) = &file.last_modified {
            h.insert(LAST_MODIFIED, lm.clone());
        }
        header_static(&mut res, CONTENT_TYPE, content_type);
        res
    }
}

fn redirect(path: &str, query: &str) -> Response<Body> {
    let mut res = empty(StatusCode::SEE_OTHER);
    let q = if query.is_empty() { String::new() } else { format!("?{query}") };
    if let Ok(loc) = HeaderValue::from_str(&format!("/{path}/{q}")) {
        res.headers_mut().insert(LOCATION, loc);
    }
    res
}

fn compressible(content_type: &str) -> bool {
    !matches!(content_type, "image/png" | "image/jpeg")
}

async fn gzipped(file: &Arc<FileEntry>) -> Option<bytes::Bytes> {
    if let Some(ready) = file.gzip_ready() {
        return ready;
    }
    let file = file.clone();
    tokio::task::spawn_blocking(move || file.gzip()).await.ok().flatten()
}

/// `None` when a streamed file vanished.
async fn identity_body(file: &FileEntry) -> Option<Response<Body>> {
    match &file.content {
        Content::Memory(data) => Some(Response::new(Body::from(data.clone()))),
        Content::Disk(path) => {
            let f = tokio::fs::File::open(path).await.ok()?;
            let mut res = Response::new(Body::from_stream(ReaderStream::with_capacity(f, 64 * 1024)));
            res.headers_mut().insert(CONTENT_LENGTH, file.len.into());
            Some(res)
        }
    }
}

/// `If-Modified-Since` (1 s slack) is checked first, then an exact `If-None-Match`; also accepts the tag quoted
/// or with our gzip suffix.
fn not_modified(headers: &HeaderMap, mtime_ms: i64, etag: &str) -> bool {
    let since = headers.get(IF_MODIFIED_SINCE).and_then(|v| v.to_str().ok()).and_then(parse_http_date);
    if since.is_some_and(|since| since + 1000 >= mtime_ms) {
        return true;
    }
    let matches = |t: &str| t == etag || t.strip_suffix(GZIP_TAG) == Some(etag);
    headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| {
        v == etag || v.split(',').any(|t| matches(t.trim().trim_start_matches("W/").trim_matches('"')))
    })
}
