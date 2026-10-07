//! `PNGImageWriter.write` with default params and fresh metadata: `PNGMetadata.initialize`, `encodePass`,
//! `RowFilter`, `IDATOutputStream`.

use super::zlib::{crc32, deflate_into};
use super::{JavaImage, Model};

/// `PNGImageWriter.DEFAULT_COMPRESSION_LEVEL`.
const DEFLATE_LEVEL: i32 = 4;
/// `IDATOutputStream`'s chunk length.
const IDAT_CHUNK: usize = 32768;
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// What `PNGMetadata.initialize` decides for an image.
struct Header {
    color_type: u8,
    bit_depth: u8,
    plte: Option<Vec<u8>>,
    trns: Option<Vec<u8>>,
    /// `PLTE_order`: palette index → written index.
    order: Option<Vec<u8>>,
    /// Gray-ramp `IndexColorModel` with alpha written as gray+alpha: each index's alpha.
    index_alpha: Option<Vec<u8>>,
}

fn header(model: &Model) -> Header {
    let plain = |color_type, bit_depth| Header {
        color_type,
        bit_depth,
        plte: None,
        trns: None,
        order: None,
        index_alpha: None,
    };
    let (rgb, alpha, bits) = match model {
        Model::Gray { bits } => return plain(0, *bits),
        Model::Rgb { bits } => return plain(2, *bits),
        Model::GrayAlpha { bits } => return plain(4, *bits),
        Model::Rgba { bits } => return plain(6, *bits),
        Model::Indexed { bits, rgb, alpha } => (rgb, alpha, *bits),
    };
    let scale = 255 / ((1u32 << bits) - 1);
    let is_gray = rgb.iter().enumerate().all(|(i, &[r, g, b])| r == (i as u32 * scale) as u8 && r == g && r == b);
    // IndexColorModel.hasAlpha: only alphas that aren't all 255 make the model translucent
    let alpha = alpha.as_ref().filter(|a| a.iter().any(|&a| a != 255));
    match alpha {
        Some(a) if is_gray && bits == 8 => Header { index_alpha: Some(a.clone()), ..plain(4, 8) },
        None if is_gray => plain(0, bits),
        None => Header { plte: Some(rgb.concat()), ..plain(3, bits) },
        Some(a) => {
            // non-opaque entries move to the front so tRNS can stop after the last of them
            let mut order = vec![0u8; a.len()];
            let mut trns = Vec::new();
            let mut next = 0usize;
            for opaque_pass in [false, true] {
                for (i, &ai) in a.iter().enumerate() {
                    if (ai == 255) == opaque_pass {
                        order[i] = next as u8;
                        next += 1;
                        if !opaque_pass {
                            trns.push(ai);
                        }
                    }
                }
            }
            let mut plte = vec![0u8; rgb.len() * 3];
            for (i, c) in rgb.iter().enumerate() {
                let o = order[i] as usize * 3;
                plte[o..o + 3].copy_from_slice(c);
            }
            Header { plte: Some(plte), trns: Some(trns), order: Some(order), ..plain(3, bits) }
        }
    }
}

pub(super) fn write_png(image: &JavaImage, out: &mut Vec<u8>) {
    let h = header(&image.model);
    out.extend_from_slice(&SIGNATURE);
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&image.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&image.height.to_be_bytes());
    ihdr[8] = h.bit_depth;
    ihdr[9] = h.color_type;
    chunk(out, b"IHDR", &ihdr);
    if let Some(plte) = &h.plte {
        chunk(out, b"PLTE", plte);
    }
    if let Some(trns) = &h.trns {
        chunk(out, b"tRNS", trns);
    }
    let mut zlib = Vec::new();
    deflate_into(&scanlines(image, &h), DEFLATE_LEVEL, &mut zlib);
    for part in zlib.chunks(IDAT_CHUNK) {
        chunk(out, b"IDAT", part);
    }
    chunk(out, b"IEND", &[]);
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// `encodePass`: every row's filter byte and filtered bytes.
fn scanlines(image: &JavaImage, h: &Header) -> Vec<u8> {
    let bands = image.model.bands();
    let width = image.width as usize;
    let bits = h.bit_depth as usize;
    let bpp = bands * if bits == 16 { 2 } else { 1 };
    let mut row_len = width * bands;
    if bits < 8 {
        row_len = row_len.div_ceil(8 / bits);
    } else if bits == 16 {
        row_len *= 2;
    }
    if h.index_alpha.is_some() {
        row_len *= 2;
    }
    let mut out = Vec::with_capacity((row_len + 1) * image.height as usize);
    // rows carry `bpp` leading zero bytes, which the filters read as the left neighbour of the first pixel
    let mut curr = vec![0u8; row_len + bpp];
    let mut prev = vec![0u8; row_len + bpp];
    let mut filtered = vec![0u8; row_len];
    for row in image.samples.chunks_exact((width * bands).max(1)).take(image.height as usize) {
        pack_row(row, h, &mut curr[bpp..]);
        if h.color_type == 3 {
            let filter = filter_row(&curr, &prev, bpp, &mut filtered);
            out.push(filter);
            out.extend_from_slice(&filtered);
        } else {
            out.push(0);
            out.extend_from_slice(&curr[bpp..]);
        }
        std::mem::swap(&mut curr, &mut prev);
    }
    out
}

fn pack_row(samples: &[u16], h: &Header, dst: &mut [u8]) {
    let index = |s: u16| match &h.order {
        Some(order) => order[s as usize],
        None => s as u8,
    };
    match h.bit_depth {
        16 => {
            for (d, &s) in dst.as_chunks_mut::<2>().0.iter_mut().zip(samples) {
                *d = s.to_be_bytes();
            }
        }
        8 => match &h.index_alpha {
            Some(alpha) => {
                for (d, &s) in dst.as_chunks_mut::<2>().0.iter_mut().zip(samples) {
                    *d = [s as u8, alpha[s as usize]];
                }
            }
            None => {
                for (d, &s) in dst.iter_mut().zip(samples) {
                    *d = index(s);
                }
            }
        },
        bits => {
            let per_byte = 8 / bits as usize;
            for (d, group) in dst.iter_mut().zip(samples.chunks(per_byte)) {
                let packed = group.iter().fold(0u8, |acc, &s| (acc << bits) | index(s));
                // a partial last byte is left-aligned
                *d = packed << ((per_byte - group.len()) * bits as usize);
            }
        }
    }
}

/// `RowFilter.filterRow`, which only filters palette images: the filter with the smallest sum of absolute
/// differences (raw bytes for None), first one wins ties. `curr`/`prev` start with `bpp` zero bytes.
fn filter_row(curr: &[u8], prev: &[u8], bpp: usize, out: &mut [u8]) -> u8 {
    let n = out.len();
    let predict = |f: u8, i: usize| -> i32 {
        let (left, up, up_left) = (curr[i - bpp] as i32, prev[i] as i32, prev[i - bpp] as i32);
        match f {
            0 => 0,
            1 => left,
            2 => up,
            3 => (left + up) / 2,
            _ => paeth(left, up, up_left),
        }
    };
    // Java sums in an int
    let badness =
        |f: u8| -> i32 { (bpp..bpp + n).fold(0i32, |sum, i| sum.wrapping_add((curr[i] as i32 - predict(f, i)).abs())) };
    let mut best = (badness(0), 0u8);
    for f in 1..5 {
        let b = badness(f);
        if b < best.0 {
            best = (b, f);
        }
    }
    let f = best.1;
    for (k, o) in out.iter_mut().enumerate() {
        let i = bpp + k;
        *o = (curr[i] as i32 - predict(f, i)) as u8;
    }
    f
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}
