//! BMQ2: the `optimized` storage encoding of a hires tile. Lossless: [`CompactCodec::decode_into`] returns the
//! exact PRBM bytes that were encoded, for every input (non-PRBM input included). Measurements: docs/09.
//!
//! # Blob
//! `"BMQ2"`, `u8 mode`, `u32 body_len`, then one zstd frame holding `body_len` bytes of body. Little-endian.
//! - mode 1 (raw): the body is the input verbatim. Used for anything that is not a quad-shaped PRBM.
//! - mode 0 (quads): every PRBM triangle pair `(v0,v1,v2),(v0,v2,v3)` is one quad, i.e. PRBM vertices
//!   `6q + {0,1,2,5}` (pos, uv and ao of `6q+3`/`6q+4` repeat `6q`/`6q+2`), and the material groups cover the
//!   vertices contiguously in whole quads. The PRBM header, attribute names and padding are canonical
//!   (`PRBMWriter`), so the decoder rebuilds them.
//!
//! # Quad body
//! `u32 quads`, `u8 pos_grid`, `u8 uv_grid`, then 11 streams, each `u32 len` + bytes:
//! 1. position: per quad 4 vertices × xyz as i16 fixed point `round(v · 2^pos_grid)` (saturated), predicted and
//!    stored as wrapping i16 residuals, one 24-byte record per quad (AoS: LZ matches repeated quads whole):
//!    `v0 − previous quad's v0`, `v1 − v0`, `v2 − v1`, `v3 − (v0 + v2 − v1)` (parallelogram).
//! 2. position escapes: values whose grid value is not bit-identical (off-grid, -0.0, NaN, out of range):
//!    `u32 n`, gap-coded flat indices (first = index), then `f32 bits − grid bits` (wrapping u32); both as 4
//!    byte planes (all low bytes first). Exact for every bit pattern.
//! 3. uv, 4. uv escapes: as 1–2 with 2 components and `uv_grid`. Grids are chosen per tile from {4, 5, 8} by
//!    fewest escapes.
//! 5. ao: 4 bytes per quad.
//! 6. normal exceptions: normals are not stored but recomputed from the decoded positions with PRBMWriter's
//!    formula ([`crate::prbm`]); quads whose 18 normal bytes differ are listed: `u32 n, u32 quad[n], 18 B × n`.
//! 7. color: one rgb per quad (vertex 0) as 3 planes; 8. color exceptions (18 B rows, as 6).
//! 9. light: blocklight plane then sunlight plane, one byte per quad; 10. light exceptions (6 block + 6 sun B).
//! 11. groups: `(i32 material, u32 quads)` per group; starts are cumulative.
//!
//! The decoder bounds-checks everything and fails with [`CompactError`] on corrupt input, never panics.
//!
//! # Exactness
//! Quad mode reproduces its input by construction, so the encoder does not decode to check (debug builds still
//! do): `view::parse` admits only input whose every byte outside attribute payloads and groups is PRBMWriter's
//! (header, names, types, zero padding, terminator); `is_quad_shaped` pins vertices `6q+3`/`6q+4` and the group
//! starts and counts; positions and uvs escape every value whose grid value differs in any bit; normal, color and
//! light rows that differ from their prediction are stored verbatim, and both sides predict normals from the same
//! position bytes with the same code. Everything the decoder bounds-checks is within what the encoder writes.

mod bytes;
mod decode;
mod encode;
mod fixed;
mod view;

use std::io::Cursor;

use zstd::zstd_safe::CParameter;

pub const MAGIC: &[u8; 4] = b"BMQ2";
const HEADER: usize = 9;
const MODE_QUADS: u8 = 0;
const MODE_RAW: u8 = 1;
/// Largest body accepted on decode: a full PRBM (2^24 vertices × 29 B) stays below it.
const MAX_BODY: usize = 1 << 30;
/// zstd level for new blobs; see docs/09 for the size/speed trade-off.
pub const DEFAULT_LEVEL: i32 = 9;
const WINDOW_LOG: u32 = 20;
const TABLE_LOG: u32 = 18;
/// Scratch above this many bytes is freed after each tile, so a thread does not pin its largest tile's buffers.
const SCRATCH_KEEP: usize = 1 << 20;

fn trim<T>(v: &mut Vec<T>) {
    if v.capacity() * size_of::<T>() > SCRATCH_KEEP {
        *v = Vec::new();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompactError {
    #[error("corrupt compact tile: {0}")]
    Corrupt(&'static str),
    #[error("compact tile zstd: {0}")]
    Zstd(#[from] std::io::Error),
}

/// Encoder/decoder with reusable buffers and zstd contexts; keep one per thread.
pub struct CompactCodec {
    level: i32,
    cctx: zstd::bulk::Compressor<'static>,
    dctx: zstd::bulk::Decompressor<'static>,
    body: Vec<u8>,
    dec: decode::Scratch,
}

impl Default for CompactCodec {
    fn default() -> Self {
        Self::new(DEFAULT_LEVEL)
    }
}

/// True if `data` starts like a BMQ2 blob.
pub fn is_compact(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

impl CompactCodec {
    pub fn new(level: i32) -> Self {
        let mut cctx = zstd::bulk::Compressor::new(level).expect("zstd compression context");
        // level 9 alone sizes its tables for multi-MB inputs (~28 MB per thread); bodies are ~0.1–1 MB
        for p in [CParameter::WindowLog(WINDOW_LOG), CParameter::HashLog(TABLE_LOG), CParameter::ChainLog(TABLE_LOG)] {
            cctx.set_parameter(p).expect("valid zstd parameter");
        }
        Self {
            level,
            cctx,
            dctx: zstd::bulk::Decompressor::new().expect("zstd decompression context"),
            body: Vec::new(),
            dec: decode::Scratch::default(),
        }
    }

    pub fn level(&self) -> i32 {
        self.level
    }

    /// Encodes `prbm` into `out` (replacing its contents).
    pub fn encode_into(&mut self, prbm: &[u8], out: &mut Vec<u8>) -> Result<(), CompactError> {
        let quads = view::parse(prbm).filter(view::PrbmView::is_quad_shaped);
        let Some(v) = quads else { return self.pack(MODE_RAW, prbm, out) };
        let mut body = std::mem::take(&mut self.body);
        body.clear();
        encode::quads(&v, &mut body);
        let packed = self.pack(MODE_QUADS, &body, out);
        trim(&mut body);
        self.body = body;
        packed?;
        debug_assert!(self.reproduces(out, prbm), "lossy BMQ2 quad encoding");
        Ok(())
    }

    fn reproduces(&mut self, blob: &[u8], prbm: &[u8]) -> bool {
        let mut check = Vec::new();
        self.decode_into(blob, &mut check).is_ok() && check == prbm
    }

    /// Decodes a blob from [`CompactCodec::encode_into`] into `out` (replacing its contents).
    pub fn decode_into(&mut self, blob: &[u8], out: &mut Vec<u8>) -> Result<(), CompactError> {
        if blob.len() < HEADER || !is_compact(blob) {
            return Err(CompactError::Corrupt("missing BMQ2 header"));
        }
        let mode = blob[4];
        let len = u32::from_le_bytes(blob[5..9].try_into().unwrap()) as usize;
        if len > MAX_BODY {
            return Err(CompactError::Corrupt("body too large"));
        }
        let target = if mode == MODE_RAW { &mut *out } else { &mut self.body };
        target.clear();
        target.reserve(len);
        if len > 0 && self.dctx.decompress_to_buffer(&blob[HEADER..], target)? != len {
            return Err(CompactError::Corrupt("body length"));
        }
        match mode {
            MODE_RAW => Ok(()),
            MODE_QUADS => {
                let mut body = std::mem::take(&mut self.body);
                let decoded = decode::quads(&body, out, &mut self.dec);
                trim(&mut body);
                self.body = body;
                self.dec.trim();
                decoded
            }
            _ => Err(CompactError::Corrupt("unknown mode")),
        }
    }

    fn pack(&mut self, mode: u8, body: &[u8], out: &mut Vec<u8>) -> Result<(), CompactError> {
        out.clear();
        out.extend(MAGIC);
        out.push(mode);
        out.extend((body.len() as u32).to_le_bytes());
        if body.is_empty() {
            return Ok(());
        }
        // quad bodies compress ~40×: try a small buffer before reserving zstd's worst case
        let first = body.len() / 4 + 4096;
        if self.compress_into(body, out, first).is_ok() {
            return Ok(());
        }
        self.compress_into(body, out, zstd::zstd_safe::compress_bound(body.len()))
    }

    fn compress_into(&mut self, body: &[u8], out: &mut Vec<u8>, capacity: usize) -> Result<(), CompactError> {
        out.truncate(HEADER);
        out.reserve(capacity);
        let mut cursor = Cursor::new(&mut *out);
        cursor.set_position(HEADER as u64);
        self.cctx.compress_to_buffer(body, &mut cursor)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
