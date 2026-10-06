//! `FileRequestHandler`: the webroot, with the bundled webapp behind it (files on disk win).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::body::Body;
use chrono::{DateTime, Utc};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LOCATION};
use http::{HeaderMap, HeaderValue, Method, Response, StatusCode};
use tokio_util::io::ReaderStream;

use crate::content_type;
use crate::paths::{Resolved, java_path_hash, join, resolve, unnormalized};
use crate::response::{empty, header_static};
use crate::webapp::{embedded_file, embedded_is_dir};

pub(crate) struct StaticFiles {
    /// Normalized like Java's `webRoot.normalize()`: the ETag hashes this path's string form.
    root: PathBuf,
    embedded: bool,
}

enum Source {
    Disk(tokio::fs::File, u64),
    Embedded(bytes::Bytes),
}

impl StaticFiles {
    pub fn new(root: &Path, embedded: bool) -> Self {
        Self { root: root.components().collect(), embedded }
    }

    /// `route_path` is the router's path (leading `/` stripped, `/` for the root); `query` is Java's re-encoded one.
    pub async fn handle(&self, method: &Method, route_path: &str, query: &str, headers: &HeaderMap) -> Response<Body> {
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
        if !route_path.ends_with('/') && self.is_dir(&rel).await {
            let mut res = empty(StatusCode::SEE_OTHER);
            let q = if query.is_empty() { String::new() } else { format!("?{query}") };
            if let Ok(loc) = HeaderValue::from_str(&format!("/{path}/{q}")) {
                res.headers_mut().insert(LOCATION, loc);
            }
            return res;
        }
        let with_index = |p: &str| if p.is_empty() { "index.html".to_owned() } else { format!("{p}/index.html") };
        // the ETag hashes the path as requested, before normalization, like Java
        let java_rel = unnormalized(path);
        let mut found = None;
        for (candidate, java_path) in [(rel.clone(), java_rel.clone()), (with_index(&rel), with_index(&java_rel))] {
            if let Some(f) = self.open(&candidate).await {
                found = Some((candidate, java_path, f));
                break;
            }
        }
        let Some((rel, java_path, (source, mtime_ms))) = found else { return empty(StatusCode::NOT_FOUND) };
        if rel.ends_with(".php") {
            return empty(StatusCode::FORBIDDEN);
        }
        let size = match &source {
            Source::Disk(_, len) => *len,
            Source::Embedded(data) => data.len() as u64,
        };
        let etag = self.etag(&java_path, size, mtime_ms);
        if not_modified(headers, mtime_ms, &etag) {
            return empty(StatusCode::NOT_MODIFIED);
        }
        let mut res = match source {
            Source::Disk(file, len) => {
                let mut res = Response::new(Body::from_stream(ReaderStream::with_capacity(file, 64 * 1024)));
                res.headers_mut().insert(CONTENT_LENGTH, len.into());
                res
            }
            Source::Embedded(data) => Response::new(Body::from(data)),
        };
        let h = res.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&etag) {
            h.insert(ETAG, v);
        }
        if mtime_ms > 0
            && let Ok(v) = HeaderValue::from_str(&java_http_date(mtime_ms))
        {
            h.insert(LAST_MODIFIED, v);
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        header_static(&mut res, CONTENT_TYPE, content_type::static_file(name));
        res
    }

    async fn is_dir(&self, rel: &str) -> bool {
        tokio::fs::metadata(join(&self.root, rel)).await.is_ok_and(|m| m.is_dir())
            || (self.embedded && embedded_is_dir(rel))
    }

    async fn open(&self, rel: &str) -> Option<(Source, i64)> {
        let path = join(&self.root, rel);
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            if meta.is_dir() {
                return None;
            }
            let mtime =
                meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
            let file = tokio::fs::File::open(&path).await.ok()?;
            return Some((Source::Disk(file, meta.len()), i64::try_from(mtime).unwrap_or(0)));
        }
        let f = self.embedded.then(|| embedded_file(rel)).flatten()?;
        Some((Source::Embedded(f.data), f.last_modified_ms))
    }

    /// Java's (unquoted) `hex(size) + hex(path.hashCode()) + hex(lastModified)`.
    fn etag(&self, rel: &str, size: u64, mtime_ms: i64) -> String {
        format!("{size:x}{:x}{mtime_ms:x}", java_path_hash(&join(&self.root, rel)))
    }
}

/// `If-Modified-Since` (1 s slack) is checked first, then an exact `If-None-Match`; also accepts the tag quoted.
fn not_modified(headers: &HeaderMap, mtime_ms: i64, etag: &str) -> bool {
    let since = headers.get(IF_MODIFIED_SINCE).and_then(|v| v.to_str().ok()).and_then(parse_http_date);
    if since.is_some_and(|since| since + 1000 >= mtime_ms) {
        return true;
    }
    headers
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag || v.split(',').any(|t| t.trim().trim_start_matches("W/").trim_matches('"') == etag))
}

/// `DateTimeFormatter.RFC_1123_DATE_TIME`: the day of month is not zero-padded (`Tue, 6 Oct 2026 …`).
pub(crate) fn java_http_date(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map_or_else(String::new, |t| t.format("%a, %-d %b %Y %H:%M:%S GMT").to_string())
}

fn parse_http_date(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(s.trim()).ok().map(|t| t.timestamp_millis())
}
