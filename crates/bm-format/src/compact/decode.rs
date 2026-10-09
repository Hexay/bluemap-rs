//! Model body → PRBM bytes in `PRBMWriter`'s exact layout.

use super::ao::{Grid, LEVELS, Probes, unpack};
use super::bytes::Reader;
use super::cells::{self, Frame};
use super::encode::predict_normals;
use super::face::{Face, Tables, hash_offset};
use super::{CompactError, Groups, trim};
use crate::prbm::{attribute, pad, u24};

/// PRBM vertex order of a quad's 4 stored vertices.
const EXPAND: [usize; 6] = [0, 1, 2, 0, 2, 3];
const MAX_VERTICES: usize = 0xFF_FFFF;

#[derive(Default)]
pub(super) struct Scratch {
    tables: Tables,
    faces: Vec<Face>,
    probes: Vec<Probes>,
    cells: Vec<[i32; 3]>,
    ids: Vec<u32>,
    grid: Grid,
    group_quads: Vec<u32>,
}

pub(super) fn quads(body: &[u8], out: &mut Vec<u8>, s: &mut Scratch) -> Result<(), CompactError> {
    let mut r = Reader::new(body);
    let quads = r.u32()? as usize;
    let n = quads.checked_mul(6).filter(|&n| n <= MAX_VERTICES).ok_or(CompactError::Corrupt("vertex count"))?;
    let has_origin = r.u8()? != 0;
    let mut int = || r.u32().map(|v| v as i32);
    let (origin, frame) = ([int()?, int()?], Frame { x_min: int()?, z_min: int()?, depth: int()? });

    let groups = r.stream(None)?.rest();
    s.group_quads.clear();
    s.group_quads.extend(groups.as_chunks::<8>().0.iter().map(|g| u32::from_le_bytes(g[4..].try_into().unwrap())));
    s.tables.read(&mut r)?;
    s.tables.faces(&mut s.faces).ok_or(CompactError::Corrupt("shape id"))?;
    if !has_origin && s.faces.iter().any(|f| f.hashed != [false; 3]) {
        return Err(CompactError::Corrupt("hashed shape without origin"));
    }
    let width = if s.faces.len() > 256 { 2 } else { 1 };
    let ids = r.stream(Some(quads * width))?.rest();
    s.ids.clear();
    s.ids.extend(
        ids.chunks_exact(width).map(|b| if width == 2 { u32::from(b[0]) | u32::from(b[1]) << 8 } else { b[0].into() }),
    );
    if s.ids.iter().any(|&id| id as usize >= s.faces.len()) {
        return Err(CompactError::Corrupt("template id"));
    }
    cells::decode(&mut r, quads, &s.group_quads, frame, &mut s.cells)?;
    let mut pos_exc = r.stream(None)?;
    let ao = r.stream(Some(s.group_quads.len().div_ceil(8) + quads))?.rest();
    let (occluder, ao) = ao.split_at(s.group_quads.len().div_ceil(8));
    let mut normal_exc = r.stream(None)?;
    let color = r.stream(Some(quads * 3))?.rest();
    let mut color_exc = r.stream(None)?;
    let mut light_exc = r.stream(None)?;
    let light = r.stream(Some(quads))?.rest();
    if !r.is_empty() {
        return Err(CompactError::Corrupt("trailing data"));
    }

    let (cell, grid) = (&s.cells, &mut s.grid);
    grid.reset(cell).ok_or(CompactError::Corrupt("cell range"))?;
    s.probes.clear();
    s.probes.extend(s.faces.iter().map(|f| Probes::new(f, grid)));
    let mut group = Groups::new(&s.group_quads);
    for (q, &id) in s.ids.iter().enumerate() {
        let g = group.at(q).1;
        if s.faces[id as usize].full && occluder.get(g / 8).is_some_and(|b| b >> (g % 8) & 1 != 0) {
            grid.set(grid.index(cell[q]));
        }
    }

    out.clear();
    out.reserve(n * 29 + 128);
    out.extend([1, 7]);
    u24(out, n);
    u24(out, 0);

    attribute(out, "position", 0x21);
    let pos_at = out.len();
    for (&id, &c) in s.ids.iter().zip(cell) {
        let face = &s.faces[id as usize];
        let d = match face.hashed {
            [false, _, false] => [0.0; 2],
            _ => hash_offset(origin[0].wrapping_add(c[0]), origin[1].wrapping_add(c[2])),
        };
        let p = face.positions(c, d);
        EXPAND.iter().flat_map(|&v| p[v]).for_each(|b| out.extend(b.to_le_bytes()));
    }
    let count = pos_exc.u32()? as usize;
    let keys = pos_exc.take(count.checked_mul(4).ok_or(CompactError::Corrupt("count"))?)?;
    for key in keys.as_chunks::<4>().0 {
        let key = u32::from_le_bytes(*key) as usize;
        let (q, a) = (key / 3, key % 3);
        if q >= quads {
            return Err(CompactError::Corrupt("exception index"));
        }
        let bits: [[u8; 4]; 4] = pos_exc.take(16)?.as_chunks::<4>().0.try_into().unwrap();
        for (prbm_vertex, v) in EXPAND.into_iter().enumerate() {
            let at = pos_at + 72 * q + 12 * prbm_vertex + 4 * a;
            out[at..at + 4].copy_from_slice(&bits[v]);
        }
    }
    if !pos_exc.is_empty() {
        return Err(CompactError::Corrupt("position exceptions"));
    }

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
    for &id in &s.ids {
        let uv = s.tables.uv[s.faces[id as usize].uv as usize];
        EXPAND.iter().flat_map(|&v| [uv[2 * v], uv[2 * v + 1]]).for_each(|b| out.extend(b.to_le_bytes()));
    }

    attribute(out, "ao", 0x47);
    for (q, (&id, &residual)) in s.ids.iter().zip(ao).enumerate() {
        let predicted = grid.predict(&s.probes[id as usize], grid.index(cell[q]));
        let codes = unpack(residual);
        let level: [u8; 4] = std::array::from_fn(|v| LEVELS[usize::from((predicted[v] + codes[v]) & 3)]);
        out.extend(EXPAND.map(|v| level[v]));
    }

    attribute(out, "blocklight", 0x03);
    let block_at = out.len();
    light.iter().for_each(|&l| out.extend([l >> 4; 6]));
    attribute(out, "sunlight", 0x03);
    let sun_at = out.len();
    light.iter().for_each(|&l| out.extend([l & 15; 6]));
    patch(out, &mut light_exc, quads, 12, |q, row, out| {
        out[block_at + 6 * q..block_at + 6 * q + 6].copy_from_slice(&row[..6]);
        out[sun_at + 6 * q..sun_at + 6 * q + 6].copy_from_slice(&row[6..]);
    })?;

    pad(out);
    let mut start = 0i64;
    for g in groups.as_chunks::<8>().0 {
        let count = i64::from(u32::from_le_bytes(g[4..].try_into().unwrap())) * 6;
        let field = |v: i64| i32::try_from(v).map_err(|_| CompactError::Corrupt("group"));
        let (vstart, vcount) = (field(start)?, field(count)?);
        out.extend(&g[..4]);
        out.extend(vstart.to_le_bytes());
        out.extend(vcount.to_le_bytes());
        start += count;
    }
    out.extend((-1i32).to_le_bytes());
    if groups.len() % 8 != 0 {
        return Err(CompactError::Corrupt("groups"));
    }
    trim(&mut s.cells);
    trim(&mut s.ids);
    s.grid.trim();
    Ok(())
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
