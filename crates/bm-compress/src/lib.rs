//! BlueMap's storage compressions (`core/storage/compression/Compression.java`) and Minecraft region chunk
//! compression types. Every decoder refuses output over a caller-given limit. `*_into` functions replace the
//! contents of `out` so callers can reuse one buffer per thread.

pub mod lz4_block;

use std::io::{self, Read, Write};

use flate2::read::{MultiGzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Compression {
    None,
    Gzip,
    /// zlib-wrapped deflate (Java's `DeflaterOutputStream`), also HTTP's `deflate` and chunk compression 2.
    Deflate,
    Zstd,
    /// lz4-java's block stream, not the LZ4 frame format; see [`lz4_block`].
    Lz4,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("corrupt {0:?} stream: {1}")]
    Corrupt(Compression, io::Error),
    #[error("decompressed output exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("lz4 block stream: {0}")]
    Lz4(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One below Java's `Deflater.DEFAULT_COMPRESSION` (6): zlib-rs bytes never match Java's anyway, and 5 halves
/// hires gzip CPU for +0.5% size (docs/12-perf-profile-rs.md).
const DEFLATE_LEVEL: u32 = 5;
/// airlift `ZstdOutputStream`'s level.
const ZSTD_LEVEL: i32 = 3;

impl Compression {
    pub const ALL: [Self; 5] = [Self::None, Self::Gzip, Self::Deflate, Self::Zstd, Self::Lz4];

    /// Config value and HTTP content-coding name.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Zstd => "zstd",
            Self::Lz4 => "lz4",
        }
    }

    /// Registry key, as stored in the SQL `compression` table.
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "bluemap:none",
            Self::Gzip => "bluemap:gzip",
            Self::Deflate => "bluemap:deflate",
            Self::Zstd => "bluemap:zstd",
            Self::Lz4 => "bluemap:lz4",
        }
    }

    pub fn file_suffix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Gzip => ".gz",
            Self::Deflate => ".deflate",
            Self::Zstd => ".zst",
            Self::Lz4 => ".lz4",
        }
    }

    /// Accepts an id (`gzip`) or a key (`bluemap:gzip`), like BlueMap's `Key` parsing.
    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.strip_prefix("bluemap:").unwrap_or(id);
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    /// Region file chunk header compression byte. 127 (custom, named in the chunk) is not supported.
    pub fn from_chunk_type(t: u8) -> Option<Self> {
        match t {
            1 => Some(Self::Gzip),
            2 => Some(Self::Deflate),
            3 => Some(Self::None),
            4 => Some(Self::Lz4),
            _ => None,
        }
    }

    /// By magic bytes; [`Compression::None`] when nothing matches (PRBM, JSON and PNG start differently).
    pub fn detect(data: &[u8]) -> Self {
        match data {
            [0x1f, 0x8b, ..] => Self::Gzip,
            [0x28, 0xb5, 0x2f, 0xfd, ..] => Self::Zstd,
            [b0 @ 0x78, b1, ..] if (u16::from(*b0) << 8 | u16::from(*b1)) % 31 == 0 => Self::Deflate,
            _ if data.starts_with(lz4_block::MAGIC) => Self::Lz4,
            _ => Self::None,
        }
    }

    pub fn compress(self, data: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.compress_into(data, &mut out)?;
        Ok(out)
    }

    pub fn compress_into(self, data: &[u8], out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        let level = flate2::Compression::new(DEFLATE_LEVEL);
        let written = match self {
            Self::None => {
                out.extend_from_slice(data);
                Ok(())
            }
            Self::Gzip => {
                let mut encoder = GzEncoder::new(&mut *out, level);
                encoder.write_all(data).and_then(|()| encoder.finish().map(drop))
            }
            Self::Deflate => {
                let mut encoder = ZlibEncoder::new(&mut *out, level);
                encoder.write_all(data).and_then(|()| encoder.finish().map(drop))
            }
            Self::Zstd => zstd::stream::copy_encode(data, &mut *out, ZSTD_LEVEL),
            Self::Lz4 => {
                lz4_block::compress_into(data, out);
                Ok(())
            }
        };
        written.map_err(|e| Error::Corrupt(self, e))
    }

    pub fn decompress(self, data: &[u8], limit: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.decompress_into(data, limit, &mut out)?;
        Ok(out)
    }

    pub fn decompress_into(self, data: &[u8], limit: usize, out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        match self {
            Self::None => read_capped(data, limit, out, self),
            // Java's GZIPInputStream reads concatenated members; flate2's GzDecoder stops after the first
            Self::Gzip => {
                out.reserve(gzip_size_hint(data, limit));
                read_capped(MultiGzDecoder::new(data), limit, out, self)
            }
            Self::Deflate => read_capped(ZlibDecoder::new(data), limit, out, self),
            Self::Zstd => {
                let decoder = zstd::stream::read::Decoder::new(data).map_err(|e| Error::Corrupt(self, e))?;
                read_capped(decoder, limit, out, self)
            }
            Self::Lz4 => lz4_block::decompress_into(data, limit, out),
        }
    }
}

/// Gzip at a chosen level (0–9), e.g. 9 for content compressed once and served many times.
pub fn gzip_with_level(data: &[u8], level: u32) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(data.len() / 3), flate2::Compression::new(level));
    encoder.write_all(data).expect("writing to a Vec cannot fail");
    encoder.finish().expect("writing to a Vec cannot fail")
}

/// The last member's ISIZE trailer (size mod 2^32), capped by `limit` and deflate's maximum ratio (~1032:1).
fn gzip_size_hint(data: &[u8], limit: usize) -> usize {
    data.last_chunk::<4>()
        .map_or(0, |t| u32::from_le_bytes(*t) as usize)
        .min(limit.saturating_add(1))
        .min(data.len().saturating_mul(1032))
}

fn read_capped(reader: impl Read, limit: usize, out: &mut Vec<u8>, c: Compression) -> Result<()> {
    reader.take(limit as u64 + 1).read_to_end(out).map_err(|e| Error::Corrupt(c, e))?;
    if out.len() > limit {
        return Err(Error::TooLarge { limit });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
