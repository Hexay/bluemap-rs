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
//!
//! # Without the `codec` feature
//! Only [`BodyUnpacker`] and [`model_frame`] remain (no zstd, no C): what a client needs to turn a model body it
//! was sent into PRBM (`crates/bm-wasm`).

// the encoder's halves of the shared modules have no caller then
#![cfg_attr(not(feature = "codec"), allow(dead_code))]

mod ao;
mod bytes;
mod cells;
#[cfg(feature = "codec")]
mod codec;
mod decode;
#[cfg(feature = "codec")]
mod encode;
mod face;
#[cfg(feature = "codec")]
mod shapes;
#[cfg(feature = "codec")]
mod view;

#[cfg(feature = "codec")]
pub use codec::{CompactCodec, DEFAULT_LEVEL};

pub const MAGIC: &[u8; 4] = b"BMQ3";
const HEADER: usize = 9;
const MODE_MODEL: u8 = 0;
const MODE_RAW: u8 = 1;
/// Largest body accepted on decode: a full PRBM (2^24 vertices × 29 B) stays below it.
const MAX_BODY: usize = 1 << 30;
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

/// True if `data` starts like a BMQ3 blob.
pub fn is_compact(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

/// The zstd frame of a model-mode blob: its content is a model body for [`BodyUnpacker`]. `None` for raw-mode,
/// empty and malformed blobs.
pub fn model_frame(blob: &[u8]) -> Option<&[u8]> {
    let model = blob.len() > HEADER && is_compact(blob) && blob[4] == MODE_MODEL && blob[5..HEADER] != [0; 4];
    model.then(|| &blob[HEADER..])
}

/// Model body → PRBM with reusable buffers.
#[derive(Default)]
pub struct BodyUnpacker(decode::Scratch);

impl BodyUnpacker {
    /// Replaces `out` with the PRBM bytes of `body`.
    pub fn unpack_into(&mut self, body: &[u8], out: &mut Vec<u8>) -> Result<(), CompactError> {
        decode::quads(body, out, &mut self.0)
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

#[cfg(all(test, feature = "codec"))]
mod tests;
