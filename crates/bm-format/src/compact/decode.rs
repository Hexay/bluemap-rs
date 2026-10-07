//! Quad body → PRBM bytes in `PRBMWriter`'s exact layout.

use super::CompactError;
use super::bytes::Reader;
use super::encode::predict_normals;
use super::fixed;
use crate::prbm::{attribute, pad, u24};

/// PRBM vertex order of a quad's 4 stored vertices.
const EXPAND: [usize; 6] = [0, 1, 2, 0, 2, 3];
const MAX_VERTICES: usize = 0xFF_FFFF;

#[derive(Default)]
pub(super) struct Scratch {
    pos: Vec<u32>,
    uv: Vec<u32>,
}

pub(super) fn quads(body: &[u8], out: &mut Vec<u8>, s: &mut Scratch) -> Result<(), CompactError> {
    let mut r = Reader::new(body);
    let quads = r.u32()? as usize;
    let n = quads.checked_mul(6).filter(|&n| n <= MAX_VERTICES).ok_or(CompactError::Corrupt("vertex count"))?;
    let (gp, gu) = (r.u8()?, r.u8()?);
    fixed::decode(&mut r, quads, 3, gp, &mut s.pos)?;
    fixed::decode(&mut r, quads, 2, gu, &mut s.uv)?;
    let ao = r.stream(Some(quads * 4))?.take(quads * 4)?;
    let mut normal_exc = r.stream(None)?;
    let color = r.stream(Some(quads * 3))?.take(quads * 3)?;
    let mut color_exc = r.stream(None)?;
    let light = r.stream(Some(quads * 2))?.take(quads * 2)?;
    let mut light_exc = r.stream(None)?;
    let mut groups = r.stream(None)?;
    if !r.is_empty() {
        return Err(CompactError::Corrupt("trailing data"));
    }

    out.clear();
    out.reserve(n * 29 + 128);
    out.extend([1, 7]);
    u24(out, n);
    u24(out, 0);

    attribute(out, "position", 0x21);
    let pos_at = out.len();
    expand(out, &s.pos, quads, 3);

    attribute(out, "normal", 0x63);
    let normal_at = out.len();
    let mut quad_pos = [0u8; 72];
    let mut normals = [0u8; 18];
    for q in 0..quads {
        quad_pos.copy_from_slice(&out[pos_at + 72 * q..pos_at + 72 * q + 72]);
        predict_normals(&quad_pos, &mut normals);
        out.extend(normals);
    }
    patch(out, &mut normal_exc, quads, 18, |q, row, out| {
        out[normal_at + 18 * q..normal_at + 18 * q + 18].copy_from_slice(row)
    })?;

    attribute(out, "color", 0x67);
    let color_at = out.len();
    for q in 0..quads {
        let c = [color[q], color[quads + q], color[2 * quads + q]];
        (0..6).for_each(|_| out.extend(c));
    }
    patch(out, &mut color_exc, quads, 18, |q, row, out| {
        out[color_at + 18 * q..color_at + 18 * q + 18].copy_from_slice(row)
    })?;

    attribute(out, "uv", 0x11);
    expand(out, &s.uv, quads, 2);

    attribute(out, "ao", 0x47);
    for a in ao.as_chunks::<4>().0 {
        out.extend(EXPAND.map(|i| a[i]));
    }

    attribute(out, "blocklight", 0x03);
    let block_at = out.len();
    light[..quads].iter().for_each(|&l| out.extend([l; 6]));
    attribute(out, "sunlight", 0x03);
    let sun_at = out.len();
    light[quads..].iter().for_each(|&l| out.extend([l; 6]));
    patch(out, &mut light_exc, quads, 12, |q, row, out| {
        out[block_at + 6 * q..block_at + 6 * q + 6].copy_from_slice(&row[..6]);
        out[sun_at + 6 * q..sun_at + 6 * q + 6].copy_from_slice(&row[6..]);
    })?;

    pad(out);
    let mut start = 0i64;
    while !groups.is_empty() {
        let (material, count) = (groups.u32()?, i64::from(groups.u32()?) * 6);
        let field = |v: i64| i32::try_from(v).map_err(|_| CompactError::Corrupt("group"));
        let (vstart, vcount) = (field(start)?, field(count)?);
        out.extend(material.to_le_bytes());
        out.extend(vstart.to_le_bytes());
        out.extend(vcount.to_le_bytes());
        start += count;
    }
    out.extend((-1i32).to_le_bytes());
    Ok(())
}

/// Writes the 6 PRBM vertices of every quad from its 4 stored ones (`k` f32 components each).
fn expand(out: &mut Vec<u8>, values: &[u32], quads: usize, k: usize) {
    for quad in values.chunks_exact(4 * k).take(quads) {
        for i in EXPAND {
            quad[i * k..(i + 1) * k].iter().for_each(|b| out.extend(b.to_le_bytes()));
        }
    }
}

/// Applies an exception stream (`u32 n, u32 index[n], rows[n]`) via `apply(quad, row, out)`.
fn patch(
    out: &mut Vec<u8>,
    exc: &mut Reader,
    quads: usize,
    size: usize,
    apply: impl Fn(usize, &[u8], &mut Vec<u8>),
) -> Result<(), CompactError> {
    let n = exc.u32()? as usize;
    let index = exc.take(n.checked_mul(4).ok_or(CompactError::Corrupt("count"))?)?;
    let rows = exc.take(n.checked_mul(size).ok_or(CompactError::Corrupt("count"))?)?;
    if !exc.is_empty() {
        return Err(CompactError::Corrupt("exception stream"));
    }
    for (i, row) in index.as_chunks::<4>().0.iter().zip(rows.chunks_exact(size)) {
        let q = u32::from_le_bytes(*i) as usize;
        if q >= quads {
            return Err(CompactError::Corrupt("exception index"));
        }
        apply(q, row, out);
    }
    Ok(())
}
