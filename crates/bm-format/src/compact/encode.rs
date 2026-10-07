//! PRBM → quad body (layout in the module docs of [`super`]).

use super::bytes::stream;
use super::fixed;
use super::view::{AO, BLOCKLIGHT, COLOR, NORMAL, POSITION, PrbmView, QV, SUNLIGHT, UV};
use crate::prbm::{normal_byte, surface_normal};

/// Reused per-codec buffers.
#[derive(Default)]
pub(super) struct Scratch {
    pos: Vec<u32>,
    uv: Vec<u32>,
    q: Vec<i16>,
}

/// Appends the quad body of a [`PrbmView::is_quad_shaped`] tile to `body`.
pub(super) fn quads(v: &PrbmView, body: &mut Vec<u8>, s: &mut Scratch) {
    let quads = v.vertices / 6;
    gather(v.attrs[POSITION], 3, &mut s.pos);
    gather(v.attrs[UV], 2, &mut s.uv);
    let (gp, gu) = (fixed::pick_grid(&s.pos), fixed::pick_grid(&s.uv));
    body.extend((quads as u32).to_le_bytes());
    body.extend([gp, gu]);
    fixed::encode(body, &s.pos, 3, gp, &mut s.q);
    fixed::encode(body, &s.uv, 2, gu, &mut s.q);

    let ao = v.attrs[AO];
    stream(body, |out| (0..quads).for_each(|q| out.extend(QV.map(|i| ao[6 * q + i]))));

    let pos = v.attrs[POSITION];
    let normal = v.attrs[NORMAL];
    exceptions(body, quads, 18, |q, row| row.copy_from_slice(&normal[18 * q..18 * q + 18]), |q, pred| {
        predict_normals(&pos[72 * q..72 * q + 72], pred)
    });

    let color = v.attrs[COLOR];
    stream(body, |out| (0..3).for_each(|c| out.extend((0..quads).map(|q| color[18 * q + c]))));
    exceptions(body, quads, 18, |q, row| row.copy_from_slice(&color[18 * q..18 * q + 18]), |q, pred| {
        pred.chunks_exact_mut(3).for_each(|vert| vert.copy_from_slice(&color[18 * q..18 * q + 3]))
    });

    let (block, sun) = (v.attrs[BLOCKLIGHT], v.attrs[SUNLIGHT]);
    stream(body, |out| {
        out.extend((0..quads).map(|q| block[6 * q]));
        out.extend((0..quads).map(|q| sun[6 * q]));
    });
    let light_row = |q: usize, row: &mut [u8]| {
        row[..6].copy_from_slice(&block[6 * q..6 * q + 6]);
        row[6..].copy_from_slice(&sun[6 * q..6 * q + 6]);
    };
    exceptions(body, quads, 12, light_row, |q, pred| {
        pred[..6].fill(block[6 * q]);
        pred[6..].fill(sun[6 * q]);
    });

    stream(body, |out| {
        for &[material, _, count] in &v.groups {
            out.extend(material.to_le_bytes());
            out.extend((count / 6).to_le_bytes());
        }
    });
}

/// Quad vertices `QV` of a per-vertex f32 attribute with `k` components, as bit patterns.
fn gather(attr: &[u8], k: usize, out: &mut Vec<u32>) {
    out.clear();
    let stride = 4 * k;
    for quad in attr.chunks_exact(6 * stride) {
        for i in QV {
            let vert = &quad[i * stride..(i + 1) * stride];
            out.extend(vert.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())));
        }
    }
}

/// PRBMWriter normals of a quad's two triangles from its 6 PRBM vertex positions (72 bytes) → 18 bytes.
pub(super) fn predict_normals(pos: &[u8], pred: &mut [u8]) {
    let f = |i: usize| f32::from_le_bytes(pos[4 * i..4 * i + 4].try_into().unwrap());
    for t in 0..2 {
        let p: [f32; 9] = std::array::from_fn(|i| f(9 * t + i));
        let n = surface_normal(&p).map(normal_byte);
        pred[9 * t..9 * t + 9].chunks_exact_mut(3).for_each(|vert| vert.copy_from_slice(&n));
    }
}

/// Quads whose `size`-byte row differs from its prediction, verbatim: `u32 n, u32 index[n], rows[n]`.
fn exceptions(
    body: &mut Vec<u8>,
    quads: usize,
    size: usize,
    actual: impl Fn(usize, &mut [u8]),
    predict: impl Fn(usize, &mut [u8]),
) {
    let (mut row, mut pred) = (vec![0; size], vec![0; size]);
    let mut bad = Vec::new();
    for q in 0..quads {
        actual(q, &mut row);
        predict(q, &mut pred);
        if pred != row {
            bad.extend(&row);
            bad.extend((q as u32).to_le_bytes());
        }
    }
    let n = bad.len() / (size + 4);
    stream(body, |out| {
        out.extend((n as u32).to_le_bytes());
        bad.chunks_exact(size + 4).for_each(|e| out.extend(&e[size..]));
        bad.chunks_exact(size + 4).for_each(|e| out.extend(&e[..size]));
    });
}
