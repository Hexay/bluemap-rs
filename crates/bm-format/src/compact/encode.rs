//! PRBM → model body (layout in the module docs of [`super`]).

use super::ao::{Grid, LEVELS, Probes, pack};
use super::bytes::stream;
use super::cells::{self, Frame};
use super::decode::predict_normals;
use super::face::Face;
use super::shapes::Shapes;
use super::view::{self, AO, BLOCKLIGHT, COLOR, NORMAL, POSITION, PrbmView, QV, SUNLIGHT, UV};
use super::{Groups, trim};

#[derive(Default)]
pub(super) struct Scratch {
    pub shapes: Shapes,
    faces: Vec<Face>,
    probes: Vec<Probes>,
    grid: Grid,
    /// Per grid cell: 1 + the group of the last full block face in it.
    marker: Vec<u16>,
    group_quads: Vec<u32>,
}

/// Appends the model body of `prbm` to `body`; `false` (body unspecified) if the model cannot hold the tile.
pub(super) fn quads(prbm: &[u8], origin: Option<[i32; 2]>, body: &mut Vec<u8>, s: &mut Scratch) -> bool {
    let Some(v) = view::parse(prbm).filter(PrbmView::is_quad_shaped) else { return false };
    let quads = v.vertices / 6;
    let (ao, block, sun) = (v.attrs[AO], v.attrs[BLOCKLIGHT], v.attrs[SUNLIGHT]);
    let levels = ao.as_chunks::<6>().0.iter().all(|q| QV.iter().all(|&i| q[i] % 64 == 63));
    let nibbles = block.iter().step_by(6).chain(sun.iter().step_by(6)).all(|&l| l <= 15);
    if quads == 0 || !levels || !nibbles || v.groups.len() >= usize::from(u16::MAX) {
        return false;
    }
    if s.shapes.build(v.attrs[POSITION], v.attrs[UV], origin).is_none() || s.grid.reset(&s.shapes.cells).is_none() {
        return false;
    }
    let (cell, ids) = (&s.shapes.cells, &s.shapes.ids);
    let frame = Frame::of(cell);
    s.group_quads.clear();
    s.group_quads.extend(v.groups.iter().map(|g| (g[2] / 6) as u32));

    body.extend((quads as u32).to_le_bytes());
    body.push(u8::from(origin.is_some()));
    let [ox, oz] = origin.unwrap_or_default();
    [ox, oz, frame.x_min, frame.z_min, frame.depth].iter().for_each(|i| body.extend(i.to_le_bytes()));
    stream(body, |out| {
        for (g, &count) in v.groups.iter().zip(&s.group_quads) {
            out.extend(g[0].to_le_bytes());
            out.extend(count.to_le_bytes());
        }
    });
    s.shapes.tables.write(body);
    let wide = s.shapes.tables.templates.len() > 256;
    stream(body, |out| match wide {
        true => ids.iter().for_each(|&id| out.extend((id as u16).to_le_bytes())),
        false => out.extend(ids.iter().map(|&id| id as u8)),
    });
    cells::encode(body, cell, &s.group_quads, frame);
    stream(body, |out| {
        out.extend((s.shapes.exceptions.len() as u32).to_le_bytes());
        s.shapes.exceptions.iter().for_each(|(key, _)| out.extend(key.to_le_bytes()));
        s.shapes.exceptions.iter().flat_map(|(_, bits)| bits).for_each(|b| out.extend(b.to_le_bytes()));
    });

    s.shapes.tables.faces(&mut s.faces).expect("the encoder's own shape ids");
    ao_stream(body, ao, s);

    let (pos, normal, color) = (v.attrs[POSITION], v.attrs[NORMAL], v.attrs[COLOR]);
    exceptions(
        body,
        quads,
        18,
        |q, row| row.copy_from_slice(&normal[18 * q..18 * q + 18]),
        |q, pred| predict_normals(&pos[72 * q..72 * q + 72], pred),
    );
    stream(body, |out| (0..3).for_each(|c| out.extend((0..quads).map(|q| color[18 * q + c]))));
    exceptions(
        body,
        quads,
        18,
        |q, row| row.copy_from_slice(&color[18 * q..18 * q + 18]),
        |q, pred| {
            pred.as_chunks_mut::<3>().0.iter_mut().for_each(|vert| vert.copy_from_slice(&color[18 * q..18 * q + 3]))
        },
    );
    let light_row = |q: usize, row: &mut [u8]| {
        row[..6].copy_from_slice(&block[6 * q..6 * q + 6]);
        row[6..].copy_from_slice(&sun[6 * q..6 * q + 6]);
    };
    exceptions(body, quads, 12, light_row, |q, pred| {
        pred[..6].fill(block[6 * q]);
        pred[6..].fill(sun[6 * q]);
    });
    stream(body, |out| out.extend((0..quads).map(|q| block[6 * q] << 4 | sun[6 * q])));

    trim(&mut s.marker);
    s.grid.trim();
    true
}

/// Occluder flag per material group, then every quad's ao as residuals of the prediction under those flags.
fn ao_stream(body: &mut Vec<u8>, ao: &[u8], s: &mut Scratch) {
    let (cell, ids, grid) = (&s.shapes.cells, &s.shapes.ids, &mut s.grid);
    s.probes.clear();
    s.probes.extend(s.faces.iter().map(|f| Probes::new(f, grid)));
    s.marker.clear();
    s.marker.resize(grid.cells(), 0);
    let mut groups = Groups::new(&s.group_quads);
    for (q, &id) in ids.iter().enumerate() {
        let group = groups.at(q).1;
        if s.faces[id as usize].full {
            s.marker[grid.index(cell[q])] = group as u16 + 1;
        }
    }
    let actual = |q: usize| QV.map(|i| (255 - ao[6 * q + i]) / 64);

    // a vertex without occlusion clears every marked cell it looks at; one that the all-occluders prediction gets
    // right confirms them
    let mut votes = vec![[0u32; 2]; s.group_quads.len()];
    for (q, &id) in ids.iter().enumerate() {
        let probes = &s.probes[id as usize];
        if !probes.any {
            continue;
        }
        let base = grid.index(cell[q]);
        let predicted = probes.predict(base, |i| s.marker[i] != 0);
        for ((targets, level), predicted) in probes.targets(base).iter().zip(actual(q)).zip(predicted) {
            let vote = if level == 0 { 1 } else { 0 };
            if level != 0 && level != predicted {
                continue;
            }
            for &i in targets.iter().flatten() {
                if let Some(group) = usize::from(s.marker[i]).checked_sub(1) {
                    votes[group][vote] += 1;
                }
            }
        }
    }
    let occluder: Vec<bool> = votes.iter().map(|[confirmed, cleared]| confirmed >= cleared).collect();

    let mut groups = Groups::new(&s.group_quads);
    for (q, &id) in ids.iter().enumerate() {
        let group = groups.at(q).1;
        if s.faces[id as usize].full && occluder[group] {
            grid.set(grid.index(cell[q]));
        }
    }
    stream(body, |out| {
        out.extend(occluder.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, &f)| b | u8::from(f) << i)));
        for (q, &id) in ids.iter().enumerate() {
            let predicted = grid.predict(&s.probes[id as usize], grid.index(cell[q]));
            let (level, mut codes) = (actual(q), [0u8; 4]);
            (0..4).for_each(|v| codes[v] = level[v].wrapping_sub(predicted[v]) & 3);
            out.push(pack(codes));
        }
    });
    debug_assert!(LEVELS.iter().enumerate().all(|(n, &l)| usize::from((255 - l) / 64) == n));
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
