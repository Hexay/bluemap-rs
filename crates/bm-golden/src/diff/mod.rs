//! Face-by-face comparison of two renders of the same area: Java BlueMap's golden output against bluemap-rs's.
//! Faces pair up by geometry; paired faces are compared on texture, uv, tint, AO and light. Material ids may
//! differ between the two, so textures are compared by name.

mod report;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub use report::{AspectReport, CellReport, Report};

use crate::{Face, Tile};

pub const ASPECTS: [&str; 6] = ["texture", "uv", "tint", "ao", "blocklight", "sunlight"];
const AO: usize = 3;
const BLOCKLIGHT: usize = 4;
const SUNLIGHT: usize = 5;

/// 1/4096 block: finer than any model coordinate, coarse enough to absorb f32 noise.
const QUANT: f32 = 4096.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    pos: [[i32; 3]; 3],
    normal: [i8; 3],
}

/// Per-vertex values in the same (sorted) vertex order as the key.
#[derive(Clone, Copy)]
struct Attrs<'a> {
    texture: &'a str,
    uv: [[i32; 2]; 3],
    color: [u8; 3],
    ao: [u8; 3],
    blocklight: i8,
    sunlight: i8,
    cell: [i32; 3],
}

fn canonical<'a>(f: &Face, names: &'a [String], origin: [i32; 2]) -> (Key, Attrs<'a>) {
    let q = f.pos.map(|p| p.map(|c| (c * QUANT).round() as i32));
    let mut order = [0, 1, 2];
    order.sort_by_key(|&i| q[i]);
    let key = Key { pos: order.map(|i| q[i]), normal: f.normal };
    let attrs = Attrs {
        texture: names.get(f.material as usize).map_or("?", String::as_str),
        uv: order.map(|i| f.uv[i].map(|c| (c * QUANT).round() as i32)),
        color: f.color,
        ao: order.map(|i| f.ao[i]),
        blocklight: f.blocklight,
        sunlight: f.sunlight,
        cell: face_cell(f, origin),
    };
    (key, attrs)
}

/// World cell of the block that drew the face (just behind it); `origin` is the tile's x/z min corner.
pub fn face_cell(f: &Face, origin: [i32; 2]) -> [i32; 3] {
    let n = f.normal.map(|c| c as f32 / 127.0);
    let centre = |axis: usize| f.pos.iter().map(|p| p[axis]).sum::<f32>() / 3.0 - n[axis] * 0.01;
    [centre(0).floor() as i32 + origin[0], centre(1).floor() as i32, centre(2).floor() as i32 + origin[1]]
}

/// The cell the face looks into.
fn facing(f: &Face, cell: [i32; 3]) -> [i32; 3] {
    let step = f.normal.map(|c| (c as f32 / 127.0).round() as i32);
    [cell[0] + step[0], cell[1] + step[1], cell[2] + step[2]]
}

/// Bit i set = `ASPECTS[i]` differs.
fn differing(a: &Attrs, b: &Attrs) -> u8 {
    [
        a.texture != b.texture,
        a.uv != b.uv,
        a.color != b.color,
        a.ao != b.ao,
        a.blocklight != b.blocklight,
        a.sunlight != b.sunlight,
    ]
    .iter()
    .enumerate()
    .fold(0, |m, (i, &d)| m | (u8::from(d) << i))
}

#[derive(Default)]
struct CellDiff {
    missing: u32,
    extra: u32,
    differing: u32,
    aspects: u8,
}

/// Accumulates tile pairs; `finish` turns it into a [`Report`].
#[derive(Default)]
pub struct RenderDiff {
    edge: u64,
    original: u64,
    candidate: u64,
    paired: u64,
    identical: u64,
    aspect_faces: [u64; 6],
    /// Summed |Δ| per aspect (AO per vertex, light per face).
    aspect_delta: [u64; 6],
    missing: HashMap<String, u64>,
    extra: HashMap<String, u64>,
    cells: HashMap<[i32; 3], CellDiff>,
}

impl RenderDiff {
    /// `original`/`candidate`: the same hires tile from each render (empty if absent); `*_names`: each
    /// site's texture resource paths; `origin`: the tile's world x/z min corner; `unrendered(x, z)`: the column
    /// is outside the compared area. Faces in or looking into such a column are skipped: past the area's edge
    /// one world goes on and the other ends, so BlueMap draws outer walls on one side only.
    pub fn add_tile(
        &mut self,
        original: &Tile,
        original_names: &[String],
        candidate: &Tile,
        candidate_names: &[String],
        origin: [i32; 2],
        unrendered: impl Fn(i32, i32) -> bool,
    ) {
        let original = self.compared_faces(original, original_names, origin, &unrendered);
        let candidate = self.compared_faces(candidate, candidate_names, origin, &unrendered);
        self.original += original.len() as u64;
        self.candidate += candidate.len() as u64;
        let mut unpaired: HashMap<Key, Vec<Attrs>> = HashMap::new();
        for (key, attrs) in original {
            unpaired.entry(key).or_default().push(attrs);
        }
        for (key, b) in candidate {
            let Some(candidates) = unpaired.get_mut(&key).filter(|c| !c.is_empty()) else {
                *self.extra.entry(b.texture.to_owned()).or_default() += 1;
                self.cells.entry(b.cell).or_default().extra += 1;
                continue;
            };
            let (best, mask) = candidates
                .iter()
                .enumerate()
                .map(|(i, a)| (i, differing(a, &b)))
                .min_by_key(|&(_, m)| m.count_ones())
                .expect("non-empty");
            let a = candidates.swap_remove(best);
            self.record_pair(&a, &b, mask);
        }
        for a in unpaired.into_values().flatten() {
            *self.missing.entry(a.texture.to_owned()).or_default() += 1;
            self.cells.entry(a.cell).or_default().missing += 1;
        }
    }

    /// Canonical faces of `tile`, minus (and counting) those in or looking into unrendered columns.
    fn compared_faces<'a>(
        &mut self,
        tile: &Tile,
        names: &'a [String],
        origin: [i32; 2],
        unrendered: &impl Fn(i32, i32) -> bool,
    ) -> Vec<(Key, Attrs<'a>)> {
        let mut kept = Vec::with_capacity(tile.face_count());
        for f in tile.faces() {
            let (key, attrs) = canonical(&f, names, origin);
            let [x, _, z] = facing(&f, attrs.cell);
            let [cx, _, cz] = attrs.cell;
            if unrendered(x, z) || unrendered(cx, cz) { self.edge += 1 } else { kept.push((key, attrs)) }
        }
        kept
    }

    fn record_pair(&mut self, a: &Attrs, b: &Attrs, mask: u8) {
        self.paired += 1;
        if mask == 0 {
            self.identical += 1;
            return;
        }
        for (i, n) in self.aspect_faces.iter_mut().enumerate() {
            *n += u64::from(mask >> i & 1);
        }
        self.aspect_delta[AO] += a.ao.iter().zip(b.ao).map(|(&x, y)| u64::from(x.abs_diff(y))).sum::<u64>();
        self.aspect_delta[BLOCKLIGHT] += u64::from(a.blocklight.abs_diff(b.blocklight));
        self.aspect_delta[SUNLIGHT] += u64::from(a.sunlight.abs_diff(b.sunlight));
        let cell = self.cells.entry(a.cell).or_default();
        cell.differing += 1;
        cell.aspects |= mask;
    }

    /// `top`: textures and cells to list.
    pub fn finish(self, top: usize) -> Report {
        let mean =
            |i: usize, per_face: u64| self.aspect_delta[i] as f64 / (self.aspect_faces[i] * per_face).max(1) as f64;
        let aspects = ASPECTS
            .iter()
            .enumerate()
            .map(|(i, &name)| {
                let mean_delta = match i {
                    AO => Some(mean(i, 3)),
                    BLOCKLIGHT | SUNLIGHT => Some(mean(i, 1)),
                    _ => None,
                };
                (name, AspectReport { faces: self.aspect_faces[i], mean_delta })
            })
            .collect();
        let mut cells: Vec<CellReport> = self
            .cells
            .into_iter()
            .map(|(pos, c)| CellReport {
                pos,
                missing: c.missing,
                extra: c.extra,
                differing: c.differing,
                aspects: ASPECTS.iter().enumerate().filter(|(i, _)| c.aspects >> i & 1 == 1).map(|(_, &a)| a).collect(),
            })
            .collect();
        cells.sort_by_key(|c| (std::cmp::Reverse(c.missing + c.extra + c.differing), c.pos));
        cells.truncate(top);
        Report {
            edge_faces: self.edge,
            original_faces: self.original,
            candidate_faces: self.candidate,
            paired: self.paired,
            identical: self.identical,
            aspects,
            missing: top_counts(self.missing, top),
            extra: top_counts(self.extra, top),
            cells,
        }
    }
}

fn top_counts(counts: HashMap<String, u64>, top: usize) -> Vec<(String, u64)> {
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(top);
    v
}
