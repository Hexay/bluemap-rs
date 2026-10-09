//! BMQ3: the `optimized` storage encoding of a hires tile. It models the mesher instead of its bytes (docs/17).
//! Lossless: [`CompactCodec::decode_into`] returns the exact PRBM bytes that were encoded, for every input
//! (non-PRBM input included).
//!
//! # Blob
//! `"BMQ3"`, `u8 mode`, `u32 body_len`, then one zstd frame holding `body_len` bytes of body. Little-endian.
//! - mode 1 (raw): the body is the input verbatim. Used for anything the model cannot hold: not a quad-shaped
//!   PRBM, no quads, ao or light values the mesher never writes, coordinates out of range.
//! - mode 0 (model): every PRBM triangle pair `(v0,v1,v2),(v0,v2,v3)` is one quad, i.e. PRBM vertices
//!   `6q + {0,1,2,5}` (pos, uv and ao of `6q+3`/`6q+4` repeat `6q`/`6q+2`), and the material groups cover the
//!   vertices contiguously in whole quads. The PRBM header, attribute names and padding are canonical
//!   (`PRBMWriter`), so the decoder rebuilds them.
//!
//! # Model body
//! `u32 quads`, `u8 has_origin`, `i32 origin_x, origin_z` (world block of the tile's minimum corner),
//! `i32 x_min, z_min, depth` (frame of the column index), then streams (`u32 len` + bytes):
//! 1. groups: `(i32 material, u32 quads)` per group; starts are cumulative.
//! 2. shapes x, 3. shapes y, 4. shapes z: `u32 n`, `u8 hashed[n]`, then the n shapes' 4 f32 column by column.
//!    A shape is one axis of a quad's 4 vertices, block-local.
//! 5. uv shapes: `u32 n`, 8 f32 columns (4 vertices × uv).
//! 6. templates: `u32 n`, 4 u16 columns: x, y, z and uv shape of a template.
//! 7. template id per quad: u8 if there are at most 256 templates, else u16.
//! 8. cells: per quad `u8 step, i8 dy` ([`cells`]); 9. their i32 escapes.
//! 10. position exceptions: `u32 n`, `u32 quad * 3 + axis` each, then 4 f32 each: axes no shape reproduces.
//! 11. ao: one bit per group (occluder material), then per quad 4 × 2 bits `(level - predicted) mod 4` ([`ao`]).
//! 12. normal exceptions: normals are not stored but recomputed from the decoded positions with PRBMWriter's
//!     formula ([`crate::prbm`]); quads whose 18 normal bytes differ are listed: `u32 n, u32 quad[n], 18 B × n`.
//! 13. color: one rgb per quad (vertex 0) as 3 planes; 14. color exceptions (18 B rows, as 12).
//! 15. light exceptions (6 block + 6 sun B rows); 16. light: per quad `block << 4 | sun` of vertex 0.
//!
//! # Geometry
//! The mesher forms a position as `fl32(L + B)`: L the block-local value of the model vertex, B the block's
//! integer coordinate in the tile; blocks with a random offset d (BlueMap's `hashToFloat` of the world column) as
//! `fl32(fl32(L + d) + B)`. The decoder redoes exactly that from (template, cell), so no value is stored per quad.
//! The encoder picks cell and shape freely and keeps a choice only if it reproduces the input bits.
//!
//! # Exactness
//! Model mode reproduces its input by construction, so the encoder does not decode to check (debug builds still
//! do): `view::parse` admits only input whose every byte outside attribute payloads and groups is PRBMWriter's
//! (header, names, types, zero padding, terminator); `is_quad_shaped` pins vertices `6q+3`/`6q+4` and the group
//! starts and counts; a shape is used only where the decoder's arithmetic gives the input bits, else the axis is a
//! position exception; uv shapes are bit patterns; ao is a residual of a prediction both sides compute from the
//! decoded geometry with the same code; normal, color and light rows that differ from their prediction are stored
//! verbatim. The decoder bounds-checks everything and fails with [`CompactError`] on corrupt input, never panics.

mod ao;
mod bytes;
mod cells;
mod decode;
mod encode;
mod face;
mod shapes;
mod view;

use std::io::Cursor;

use zstd::zstd_safe::CParameter;

pub const MAGIC: &[u8; 4] = b"BMQ3";
const HEADER: usize = 9;
const MODE_MODEL: u8 = 0;
const MODE_RAW: u8 = 1;
/// Largest body accepted on decode: a full PRBM (2^24 vertices × 29 B) stays below it.
const MAX_BODY: usize = 1 << 30;
/// zstd level for new blobs; see docs/17 for the size/speed trade-off.
pub const DEFAULT_LEVEL: i32 = 9;
const WINDOW_LOG: u32 = 20;
const TABLE_LOG: u32 = 18;
/// Scratch above this many bytes is freed after each tile, so a thread does not pin its largest tile's buffers.
const SCRATCH_KEEP: usize = 1 << 20;
/// Cells further out than this make the encoder fall back to raw and the decoder fail.
const CELL_LIMIT: i32 = 1 << 20;
/// Cells in the bounding box of a tile's quads (plus a border) that the occluder grid may span.
const MAX_GRID: usize = 1 << 22;
/// Bytes of a model body before its first stream.
#[doc(hidden)]
pub const BODY_HEADER: usize = 25;
#[doc(hidden)]
pub const STREAMS: [&str; 16] = [
    "groups",
    "shapes x",
    "shapes y",
    "shapes z",
    "shapes uv",
    "templates",
    "template ids",
    "cells",
    "cell escapes",
    "position exc",
    "ao",
    "normal exc",
    "color",
    "color exc",
    "light exc",
    "light",
];

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
    enc: encode::Scratch,
    dec: decode::Scratch,
}

impl Default for CompactCodec {
    fn default() -> Self {
        Self::new(DEFAULT_LEVEL)
    }
}

/// True if `data` starts like a BMQ3 blob.
pub fn is_compact(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

impl CompactCodec {
    pub fn new(level: i32) -> Self {
        let mut cctx = zstd::bulk::Compressor::new(level).expect("zstd compression context");
        // level 9 alone sizes its tables for multi-MB inputs (~28 MB per thread); bodies are well under 1 MB
        for p in [CParameter::WindowLog(WINDOW_LOG), CParameter::HashLog(TABLE_LOG), CParameter::ChainLog(TABLE_LOG)] {
            cctx.set_parameter(p).expect("valid zstd parameter");
        }
        Self {
            level,
            cctx,
            dctx: zstd::bulk::Decompressor::new().expect("zstd decompression context"),
            body: Vec::new(),
            enc: encode::Scratch::default(),
            dec: decode::Scratch::default(),
        }
    }

    pub fn level(&self) -> i32 {
        self.level
    }

    /// Encodes `prbm` into `out` (replacing its contents). `origin` is the world block (x, z) of the tile's
    /// minimum corner; without it (or with a wrong one) blocks with a random offset cost more bytes.
    pub fn encode_into(
        &mut self,
        prbm: &[u8],
        origin: Option<[i32; 2]>,
        out: &mut Vec<u8>,
    ) -> Result<(), CompactError> {
        let mut body = std::mem::take(&mut self.body);
        let packed = match self.body_into(prbm, origin, &mut body) {
            true => self.pack(MODE_MODEL, &body, out),
            false => self.pack(MODE_RAW, prbm, out),
        };
        trim(&mut body);
        self.body = body;
        packed?;
        debug_assert!(self.reproduces(out, prbm), "lossy BMQ3 encoding");
        Ok(())
    }

    /// Replaces `body` with the uncompressed model body of `prbm` (its first stream starts at [`BODY_HEADER`]);
    /// `false` (body unspecified) if the model cannot hold the tile.
    #[doc(hidden)]
    pub fn body_into(&mut self, prbm: &[u8], origin: Option<[i32; 2]>, body: &mut Vec<u8>) -> bool {
        body.clear();
        encode::quads(prbm, origin, body, &mut self.enc)
    }

    fn reproduces(&mut self, blob: &[u8], prbm: &[u8]) -> bool {
        let mut check = Vec::new();
        self.decode_into(blob, &mut check).is_ok() && check == prbm
    }

    /// Decodes a blob from [`CompactCodec::encode_into`] into `out` (replacing its contents).
    pub fn decode_into(&mut self, blob: &[u8], out: &mut Vec<u8>) -> Result<(), CompactError> {
        if blob.len() < HEADER || !is_compact(blob) {
            return Err(CompactError::Corrupt("missing BMQ3 header"));
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
            MODE_MODEL => {
                let mut body = std::mem::take(&mut self.body);
                let decoded = decode::quads(&body, out, &mut self.dec);
                trim(&mut body);
                self.body = body;
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
        // model bodies compress several times over: try a small buffer before reserving zstd's worst case
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

/// Which group a quad belongs to and whether it is the first of one, for quads visited in order.
struct Groups<'a> {
    counts: &'a [u32],
    next: usize,
    upcoming: usize,
}

impl<'a> Groups<'a> {
    fn new(counts: &'a [u32]) -> Self {
        Self { counts, next: 0, upcoming: 0 }
    }

    /// `(starts a group, group index)` of quad `q`; call with q = 0, 1, 2, ….
    fn at(&mut self, q: usize) -> (bool, usize) {
        let mut start = false;
        while self.next < self.counts.len() && self.upcoming == q {
            self.upcoming += self.counts[self.next] as usize;
            self.next += 1;
            start = true;
        }
        (start, self.next.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests;
