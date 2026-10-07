//! `MapStorageRequestHandler.writeToResponse`: pass stored bytes through when the client accepts their coding,
//! else transcode to gzip or decode to identity; `.gz` URLs always get gzip bytes without `Content-Encoding`.

use bm_storage::{Compression, Stored};
use bytes::Bytes;
use http::HeaderMap;
use http::header::ACCEPT_ENCODING;

use crate::transcode;

/// The codings named in `Accept-Encoding`. Java matches whole comma-separated tokens case-insensitively, so
/// `gzip;q=1.0` never matched there; here parameters are stripped and `q=0` means refused.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Accepted(Vec<String>);

impl Accepted {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut codings = Vec::new();
        for value in headers.get_all(ACCEPT_ENCODING) {
            let Ok(value) = value.to_str() else { continue };
            for item in value.split(',') {
                let mut parts = item.split(';').map(str::trim);
                let coding = parts.next().unwrap_or("").to_ascii_lowercase();
                let refused = parts.any(|p| {
                    p.split_once('=').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("q") && is_zero_q(v.trim()))
                });
                if !coding.is_empty() && !refused {
                    codings.push(coding);
                }
            }
        }
        Self(codings)
    }

    pub fn accepts(&self, coding: &str) -> bool {
        self.0.iter().any(|c| c == coding)
    }
}

fn is_zero_q(q: &str) -> bool {
    q.parse::<f32>().is_ok_and(|v| v == 0.0)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Encoded {
    pub body: Bytes,
    pub content_encoding: Option<&'static str>,
}

/// Picks the response bytes for stored map data. Transcoding blocks (CPU, and a bounded number of concurrent
/// transcodes): call from a blocking context.
pub fn encode(stored: Stored, is_png: bool, gz_url: bool, accepted: &Accepted) -> Result<Encoded, bm_compress::Error> {
    let c = stored.compression;
    if gz_url {
        let body = if c == Compression::Gzip { stored.data.into() } else { transcode::to_gzip(&stored)? };
        return Ok(Encoded { body, content_encoding: None });
    }
    if c != Compression::None && accepted.accepts(c.id()) {
        return Ok(Encoded { body: stored.data.into(), content_encoding: Some(c.id()) });
    }
    if c != Compression::Gzip && !is_png && accepted.accepts(Compression::Gzip.id()) {
        return Ok(Encoded { body: transcode::to_gzip(&stored)?, content_encoding: Some(Compression::Gzip.id()) });
    }
    let body = if c == Compression::None { stored.data.into() } else { transcode::decode(&stored)? };
    Ok(Encoded { body, content_encoding: None })
}
