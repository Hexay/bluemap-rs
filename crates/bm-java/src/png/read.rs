//! `PNGImageReader`: which `BufferedImage` a decoded PNG becomes (`getImageTypes`' first type, `decodePass`).

use super::{JavaImage, Model};

/// A PNG after inflating and unfiltering, before any sample conversion.
#[derive(Clone, Copy, Debug)]
pub struct RawPng<'a> {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    /// Non-interlaced rows, each padded to a whole byte; samples big-endian, sub-byte samples MSB first.
    pub data: &'a [u8],
    /// The `PLTE` chunk's payload.
    pub palette: Option<&'a [u8]>,
    /// The `tRNS` chunk's payload.
    pub trns: Option<&'a [u8]>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("unsupported PNG colour type {0} at bit depth {1}")]
    Format(u8, u8),
    #[error("PNG pixel data is shorter than its header says")]
    Truncated,
    #[error("palette PNG without a PLTE entry")]
    NoPalette,
}

impl JavaImage {
    /// `ImageIO.read` of the PNG `raw`.
    pub fn read(raw: &RawPng) -> Result<Self, ReadError> {
        let (ct, bits) = (raw.color_type, raw.bit_depth);
        let in_bands = match (ct, bits) {
            (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) => 1,
            (4, 8 | 16) => 2,
            (2, 8 | 16) => 3,
            (6, 8 | 16) => 4,
            _ => return Err(ReadError::Format(ct, bits)),
        };
        let mut samples = unpack(raw, in_bands)?;
        let model = match (ct, raw.trns) {
            (0, Some(&[hi, lo])) => {
                let key = u16::from_be_bytes([hi, lo]);
                let (out_bits, opaque) = if bits == 16 { (16, u16::MAX) } else { (8, 255) };
                let max = (1u32 << bits) - 1;
                samples = samples
                    .iter()
                    .flat_map(|&s| {
                        let v = ((s as u32 * ((1u32 << out_bits) - 1) + max / 2) / max) as u16;
                        // JDK quirk: the tRNS key is compared with the sample after scaling to 8 bits
                        [v, if v == key { 0 } else { opaque }]
                    })
                    .collect();
                Model::GrayAlpha { bits: out_bits }
            }
            (0, _) => Model::Gray { bits },
            (2, Some(&[r0, r1, g0, g1, b0, b1])) => {
                let key = [[r0, r1], [g0, g1], [b0, b1]].map(u16::from_be_bytes);
                let opaque = if bits == 16 { u16::MAX } else { 255 };
                samples = samples
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|&p| [p[0], p[1], p[2], if p == key { 0 } else { opaque }])
                    .collect();
                Model::Rgba { bits }
            }
            (2, _) => Model::Rgb { bits },
            (3, trns) => indexed(bits, raw.palette.ok_or(ReadError::NoPalette)?, trns)?,
            (4, _) => Model::GrayAlpha { bits },
            _ => Model::Rgba { bits },
        };
        Ok(Self { width: raw.width, height: raw.height, model, samples })
    }
}

fn unpack(raw: &RawPng, bands: usize) -> Result<Vec<u16>, ReadError> {
    let (w, h, bits) = (raw.width as usize, raw.height as usize, raw.bit_depth as usize);
    let stride = (w * bands * bits).div_ceil(8);
    let data = raw.data.get(..stride * h).ok_or(ReadError::Truncated)?;
    let mut out = Vec::with_capacity(w * h * bands);
    for row in data.chunks_exact(stride.max(1)).take(h) {
        match bits {
            16 => out.extend(row.as_chunks::<2>().0.iter().map(|&s| u16::from_be_bytes(s))),
            8 => out.extend(row.iter().map(|&s| u16::from(s))),
            _ => {
                let per_byte = 8 / bits;
                let mask = (1u8 << bits) - 1;
                out.extend((0..w).map(|x| {
                    let shift = 8 - bits * (x % per_byte + 1);
                    u16::from((row[x / per_byte] >> shift) & mask)
                }));
            }
        }
    }
    Ok(out)
}

/// `parse_PLTE_chunk`/`parse_tRNS_chunk` plus the palette padding in `getImageTypes`.
fn indexed(bits: u8, plte: &[u8], trns: Option<&[u8]>) -> Result<Model, ReadError> {
    let size = 1usize << bits;
    let entries = (plte.len() / 3).min(size);
    if entries == 0 {
        return Err(ReadError::NoPalette);
    }
    // PLTE arrays are rounded up to 2, 4, 16 or 256 zeroed entries, then padded to 2^bits with the last of those
    let rounded = match entries {
        17.. => 256,
        5.. => 16,
        3.. => 4,
        _ => 2,
    };
    let mut rgb: Vec<[u8; 3]> = plte.as_chunks::<3>().0[..entries].to_vec();
    rgb.resize(rounded, [0; 3]);
    let last = rgb[rounded - 1];
    rgb.resize(size.max(rounded), last);
    let alpha = trns.map(|t| {
        let mut a = t[..t.len().min(rounded)].to_vec();
        a.resize(rgb.len(), 255);
        a
    });
    Ok(Model::Indexed { bits, rgb, alpha })
}
