//! The blob around a body: header plus one zstd frame.

use std::io::Cursor;

use zstd::zstd_safe::CParameter;

use super::{CompactError, HEADER, MAGIC, MAX_BODY, MODE_MODEL, MODE_RAW, decode, encode, is_compact, trim};

/// zstd level for new blobs; see docs/17 for the size/speed trade-off.
pub const DEFAULT_LEVEL: i32 = 9;
const WINDOW_LOG: u32 = 20;
const TABLE_LOG: u32 = 18;

/// Encoder/decoder with reusable buffers and zstd contexts; keep one per thread.
pub struct CompactCodec {
    level: i32,
    cctx: zstd::bulk::Compressor<'static>,
    dctx: zstd::bulk::Decompressor<'static>,
    body: Vec<u8>,
    pub(super) enc: encode::Scratch,
    pub(super) dec: decode::Scratch,
}

impl Default for CompactCodec {
    fn default() -> Self {
        Self::new(DEFAULT_LEVEL)
    }
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

    /// Replaces `body` with the uncompressed model body of `prbm` (its first stream starts at
    /// [`super::BODY_HEADER`]); `false` (body unspecified) if the model cannot hold the tile.
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
