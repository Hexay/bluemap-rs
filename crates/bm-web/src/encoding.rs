//! `MapStorageRequestHandler.writeToResponse`: pass stored bytes through when the client accepts their coding,
//! else transcode to gzip or decode to identity; `.gz` URLs always get gzip bytes without `Content-Encoding`.

use bm_storage::{Compression, Stored};
use bytes::Bytes;
use http::HeaderMap;
use http::header::ACCEPT_ENCODING;

use crate::transcode;

/// The codings named in `Accept-Encoding`. Java matches whole comma-separated tokens case-insensitively, so
/// `gzip;q=1.0` never matched there; here parameters are stripped and `q=0` means refused.
/// Only codings we can produce or pass through are tracked (a bit per [`Compression`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Accepted(u8);

impl Accepted {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut bits = 0;
        for value in headers.get_all(ACCEPT_ENCODING) {
            let Ok(value) = value.to_str() else { continue };
            for item in value.split(',') {
                let mut parts = item.split(';').map(str::trim);
                let Some(bit) = parts.next().and_then(bit) else { continue };
                let refused = parts.any(|p| {
                    p.split_once('=').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("q") && is_zero_q(v.trim()))
                });
                if !refused {
                    bits |= bit;
                }
            }
        }
        Self(bits)
    }

    pub fn accepts(&self, coding: &str) -> bool {
        bit(coding).is_some_and(|b| self.0 & b != 0)
    }
}

fn bit(coding: &str) -> Option<u8> {
    Compression::ALL.iter().position(|c| c.id().eq_ignore_ascii_case(coding)).map(|i| 1 << i)
}

fn is_zero_q(q: &str) -> bool {
    q.parse::<f32>().is_ok_and(|v| v == 0.0)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Encoded {
    pub body: Bytes,
    pub content_encoding: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Stored,
    ToGzip,
    Decode,
}

/// What [`encode`] does with data stored in `c`, and the `Content-Encoding` it labels the reply with.
fn plan(c: Compression, is_png: bool, gz_url: bool, accepted: &Accepted) -> (Op, Option<&'static str>) {
    let gzip = Compression::Gzip;
    if gz_url {
        return (if c == gzip { Op::Stored } else { Op::ToGzip }, None);
    }
    if c != Compression::None && accepted.accepts(c.id()) {
        return (Op::Stored, Some(c.id()));
    }
    if c != gzip && !is_png && accepted.accepts(gzip.id()) {
        return (Op::ToGzip, Some(gzip.id()));
    }
    (if c == Compression::None { Op::Stored } else { Op::Decode }, None)
}

/// The coding a packed hires tile (`MapStorage::read_hires_packed`) is sent in. Nothing is stored in a client
/// coding, so one is made per tile: zstd where accepted (1.8 against 5.0 ms and no larger than gzip,
/// docs/perf-exp/web-profile.md), else as [`plan`] does for uncompressed data.
pub fn packed_coding(gz_url: bool, accepted: &Accepted) -> Compression {
    let (gzip, zstd) = (Compression::Gzip, Compression::Zstd);
    if gz_url {
        gzip
    } else if accepted.accepts(zstd.id()) {
        zstd
    } else if accepted.accepts(gzip.id()) {
        gzip
    } else {
        Compression::None
    }
}

/// The coding of the bytes [`encode`] sends (`.gz` URLs get gzip bytes unlabelled), known before reading them.
pub fn body_coding(c: Compression, is_png: bool, gz_url: bool, accepted: &Accepted) -> Option<&'static str> {
    if gz_url { Some(Compression::Gzip.id()) } else { plan(c, is_png, gz_url, accepted).1 }
}

/// Picks the response bytes for stored map data. Transcoding blocks (CPU, and a bounded number of concurrent
/// transcodes): call from a blocking context.
pub fn encode(stored: Stored, is_png: bool, gz_url: bool, accepted: &Accepted) -> Result<Encoded, bm_compress::Error> {
    let (op, content_encoding) = plan(stored.compression, is_png, gz_url, accepted);
    let body = match op {
        Op::Stored => stored.data.into(),
        Op::ToGzip => transcode::to_gzip(&stored)?,
        Op::Decode => transcode::decode(&stored)?,
    };
    Ok(Encoded { body, content_encoding })
}
