//! Model body → PRBM bytes in `PRBMWriter`'s exact layout.

use super::ao::{Grid, LEVELS, Probes, unpack};
use super::bytes::Reader;
use super::cells::{self, Frame};
use super::face::{Face, Tables, hash_offset};
use super::{CompactError, Groups, trim};
use crate::prbm::{attribute, normal_bytes, pad, u24};

/// PRBM vertex order of a quad's 4 stored vertices.
const EXPAND: [usize; 6] = [0, 1, 2, 0, 2, 3];
const MAX_VERTICES: usize = 0xFF_FFFF;
/// Name, type byte and bytes per quad of the PRBM attributes, in file order.
const ATTRIBUTES: [(&str, u8, usize); 7] = [
    ("position", 0x21, 72),
    ("normal", 0x63, 18),
    ("color", 0x67, 18),
    ("uv", 0x11, 48),
    ("ao", 0x47, 6),
    ("blocklight", 0x03, 6),
    ("sunlight", 0x03, 6),
];

#[derive(Default)]
pub(super) struct Scratch {
    tables: Tables,
    faces: Vec<Face>,
    probes: Vec<Probes>,
    /// Per uv shape: the uv bytes of a quad's 6 PRBM vertices.
    uv_rows: Vec<[u8; 48]>,
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
    s.uv_rows.clear();
    s.uv_rows.extend(s.tables.uv.iter().map(|uv| {
        let mut row = [0; 48];
        let vertices = row.as_chunks_mut::<8>().0.iter_mut().zip(EXPAND);
        vertices.for_each(|(b, v)| put(b, &[uv[2 * v], uv[2 * v + 1]]));
        row
    }));

    out.clear();
    out.reserve(n * 29 + 128 + groups.len() / 8 * 12);
    out.extend([1, 7]);
    u24(out, n);
    u24(out, 0);
    let at = ATTRIBUTES.map(|(name, ty, size)| {
        attribute(out, name, ty);
        let at = out.len();
        out.resize(at + quads * size, 0);
        at
    });
    let [pos, normal, color_out, uv, ao_out, block, sun] = payloads(out, at, quads);

    let rows = pos.as_chunks_mut::<72>().0.iter_mut().zip(normal.as_chunks_mut::<18>().0);
    let rows = rows.zip(uv.as_chunks_mut::<48>().0).zip(ao_out.as_chunks_mut::<6>().0);
    for ((((pos, normal), uv), ao_row), ((&id, &c), &residual)) in rows.zip(s.ids.iter().zip(cell).zip(ao)) {
        let face = &s.faces[id as usize];
        let d = match face.hashed {
            [false, _, false] => [0.0; 2],
            _ => hash_offset(origin[0].wrapping_add(c[0]), origin[1].wrapping_add(c[2])),
        };
        let p = face.positions(c, d);
        // plain indexing below: the iterator forms of these loops were left as calls
        for (i, v) in EXPAND.into_iter().enumerate() {
            put(&mut pos[12 * i..12 * i + 12], &p[v]);
        }
        let f = p.map(|v| v.map(f32::from_bits));
        let (first, second) = (triangle_normal(f[0], f[1], f[2]), triangle_normal(f[0], f[2], f[3]));
        for i in 0..3 {
            normal[3 * i..3 * i + 3].copy_from_slice(&first);
            normal[9 + 3 * i..12 + 3 * i].copy_from_slice(&second);
        }
        *uv = s.uv_rows[face.uv as usize];

        let predicted = grid.predict(&s.probes[id as usize], grid.index(c));
        let codes = unpack(residual);
        let level = [0, 1, 2, 3].map(|v| LEVELS[usize::from((predicted[v] + codes[v]) & 3)]);
        *ao_row = EXPAND.map(|v| level[v]);
    }
    for (q, row) in color_out.as_chunks_mut::<18>().0.iter_mut().enumerate() {
        let c = [color[q], color[quads + q], color[2 * quads + q]];
        row.as_chunks_mut::<3>().0.iter_mut().for_each(|b| *b = c);
    }
    for ((block, sun), &l) in block.as_chunks_mut::<6>().0.iter_mut().zip(sun.as_chunks_mut::<6>().0).zip(light) {
        (*block, *sun) = ([l >> 4; 6], [l & 15; 6]);
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
            let at = 72 * q + 12 * prbm_vertex + 4 * a;
            pos[at..at + 4].copy_from_slice(&bits[v]);
        }
        predict_normals(&pos[72 * q..72 * q + 72], &mut normal[18 * q..18 * q + 18]);
    }
    if !pos_exc.is_empty() {
        return Err(CompactError::Corrupt("position exceptions"));
    }
    patch(&mut normal_exc, quads, 18, |q, row| normal[18 * q..18 * q + 18].copy_from_slice(row))?;
    patch(&mut color_exc, quads, 18, |q, row| color_out[18 * q..18 * q + 18].copy_from_slice(row))?;
    patch(&mut light_exc, quads, 12, |q, row| {
        block[6 * q..6 * q + 6].copy_from_slice(&row[..6]);
        sun[6 * q..6 * q + 6].copy_from_slice(&row[6..]);
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

/// PRBMWriter normals of a quad's two triangles from its 6 PRBM vertex positions (72 bytes) → 18 bytes.
pub(super) fn predict_normals(pos: &[u8], pred: &mut [u8]) {
    let f = |i: usize| f32::from_le_bytes(pos[4 * i..4 * i + 4].try_into().unwrap());
    for t in 0..2 {
        let p: [f32; 9] = std::array::from_fn(|i| f(9 * t + i));
        let n = normal_bytes(&p);
        pred[9 * t..9 * t + 9].as_chunks_mut::<3>().0.iter_mut().for_each(|vert| *vert = n);
    }
}

#[inline(always)]
fn triangle_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [u8; 3] {
    normal_bytes(&[a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]])
}

/// `values` as little-endian bytes filling `out`.
#[inline(always)]
fn put(out: &mut [u8], values: &[u32]) {
    out.as_chunks_mut::<4>().0.iter_mut().zip(values).for_each(|(b, v)| *b = v.to_le_bytes());
}

/// The attribute payloads of `out`, which start at `at` and hold `quads` rows each.
fn payloads(out: &mut [u8], at: [usize; 7], quads: usize) -> [&mut [u8]; 7] {
    let (mut rest, mut consumed) = (out, 0);
    std::array::from_fn(|i| {
        let (_, tail) = std::mem::take(&mut rest).split_at_mut(at[i] - consumed);
        let (payload, tail) = tail.split_at_mut(quads * ATTRIBUTES[i].2);
        (rest, consumed) = (tail, at[i] + payload.len());
        payload
    })
}

/// Applies an exception stream (`u32 n, u32 index[n], rows[n]`) via `apply(quad, row)`.
fn patch(
    exc: &mut Reader,
    quads: usize,
    size: usize,
    mut apply: impl FnMut(usize, &[u8]),
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
        apply(q, row);
    }
    Ok(())
}
