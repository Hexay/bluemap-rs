//! `MapStorageRequestHandler.writeToResponse`: pass stored bytes through when the client accepts their coding,
//! else transcode to gzip or decode to identity; `.gz` URLs always get gzip bytes without `Content-Encoding`.

use bm_storage::{Compression, Stored};
use bytes::Bytes;
use http::HeaderMap;
use http::header::ACCEPT_ENCODING;

use crate::transcode;

/// `MapStorageRequestHandler.acceptsEncoding`: a coding's first listing decides (accepted when its `q` > 0), an
/// unlisted one is accepted when `*` is; the first `q=` parameter counts and an unparsable one means 0.
/// Only codings we can produce or pass through are tracked (a bit per [`Compression`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    listed: u8,
    accepted: u8,
    wildcard: bool,
}

impl Accepted {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut out = Self::default();
        for value in headers.get_all(ACCEPT_ENCODING) {
            let Ok(value) = value.to_str() else { continue };
            for item in value.split(',') {
                let mut parts = item.split(';').map(str::trim);
                let coding = parts.next().unwrap_or_default();
                let positive = quality(parts) > 0.0;
                if coding == "*" {
                    out.wildcard = positive;
                } else if let Some(bit) = bit(coding).filter(|b| out.listed & b == 0) {
                    out.listed |= bit;
                    if positive {
                        out.accepted |= bit;
                    }
                }
            }
        }
        out
    }

    pub fn accepts(&self, coding: &str) -> bool {
        bit(coding).is_some_and(|b| if self.listed & b != 0 { self.accepted & b != 0 } else { self.wildcard })
    }
}

fn bit(coding: &str) -> Option<u8> {
    Compression::ALL.iter().position(|c| c.id().eq_ignore_ascii_case(coding)).map(|i| 1 << i)
}

fn quality<'a>(mut params: impl Iterator<Item = &'a str>) -> f32 {
    params
        .find_map(|p| p.get(..2).filter(|k| k.eq_ignore_ascii_case("q=")).map(|_| &p[2..]))
        .map_or(1.0, |q| q.trim().parse().unwrap_or(0.0))
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
