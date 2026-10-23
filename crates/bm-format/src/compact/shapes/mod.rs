//! Encoder side of the geometry: a cell for every quad and, per axis, a shape that reproduces its position bits.
//!
//! A value is `fl32(shape + B)`, so a shape derived at a large B has lost fraction bits and may not fit the same
//! model vertex at a smaller B. A first pass therefore seeds every key with its value from the smallest |B|
//! (finest first, docs/19); a value that still does not fit joins the key's candidates behind it. Whether a quad's x or z
//! is relative to the block's hash offset is not known up front: both forms are tried, and the one whose shape
//! serves more blocks wins (an offset plant's plain shape serves one block, its hashed shape every such plant).
//!
//! Block-local values are f32 differences: f64 has more than 2 × 24 + 2 bits, so the f64 difference rounded to f32
//! is the f32 difference (no double rounding), as the earlier f64 form computed it.

mod arena;
mod lanes;

use rustc_hash::FxHashMap;

use super::CELL_LIMIT;
use super::face::{MAX_IDS, Tables, hash_offset};
use super::view::QV;
use arena::{Arena, NONE};
use lanes::{fits, on_grid, pack};

/// A quad's candidate per axis in the plain form and, for x and z, in the hashed form.
struct Pick {
    plain: [u32; 3],
    hashed: [u32; 2],
    uv: u32,
}

/// A quad's key id per axis and, for an off-grid x or z with a known origin, the key id of its value relative to
/// the block's hash offset (else `NONE`) and that offset.
struct Located {
    plain: [u32; 3],
    hashed: [u32; 2],
    offset: [f32; 2],
}

#[derive(Default)]
pub(super) struct Shapes {
    plain: [Arena; 3],
    hashed: [Arena; 2],
    uv: FxHashMap<(u128, u128), u32>,
    templates: FxHashMap<u128, u32>,
    located: Vec<Located>,
    picks: Vec<Pick>,
    pub cells: Vec<[i32; 3]>,
    /// Template of every quad.
    pub ids: Vec<u32>,
    pub tables: Tables,
    /// `(quad * 3 + axis, f32 bits)` of axes no shape reproduces.
    pub exceptions: Vec<(u32, [u32; 4])>,
}

/// f32 bits of a quad's 4 vertices × `K` components from its 6 PRBM vertices.
#[inline]
fn stored<const K: usize>(quad: &[u8]) -> [[u32; K]; 4] {
    QV.map(|v| std::array::from_fn(|c| u32::from_le_bytes(quad[4 * (K * v + c)..][..4].try_into().unwrap())))
}

/// The block a quad belongs to: its centre, nudged against the normal so that a face on the block's boundary
/// stays with the block. `None` for coordinates out of range.
#[inline]
fn cell_of(p: &[[f32; 3]; 4]) -> Option<[i32; 3]> {
    let edge = |v: usize| [0, 1, 2].map(|a| f64::from(p[v][a]) - f64::from(p[0][a]));
    let (e1, e2) = (edge(1), edge(2));
    let cross = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    let mut cell = [0i32; 3];
    for a in 0..3 {
        let sum: f64 = p.iter().map(|v| f64::from(v[a])).sum();
        let nudge = f64::from(i8::from(cross[a] > 0.0) - i8::from(cross[a] < 0.0)) / 16.0;
        let c = (sum - nudge) / 4.0;
        if c.is_nan() || c.abs() > f64::from(CELL_LIMIT) {
            return None;
        }
        // floor without the libm call baseline x86-64 makes for it
        cell[a] = c as i32 - i32::from(c < f64::from(c as i32));
    }
    Some(cell)
}

impl Shapes {
    fn clear(&mut self) {
        self.plain.iter_mut().chain(&mut self.hashed).for_each(Arena::clear);
        self.uv.clear();
        self.templates.clear();
        self.located.clear();
        self.picks.clear();
        self.cells.clear();
        self.ids.clear();
        self.tables.clear();
        self.exceptions.clear();
    }

    /// Fills cells, template ids, tables and exceptions from the PRBM position and uv payloads; `None` if the tile
    /// is out of the format's range.
    // once per tile; inlined into `encode::quads` it ran ~1.5% more instructions
    #[inline(never)]
    pub fn build(&mut self, pos: &[u8], uv: &[u8], origin: Option<[i32; 2]>) -> Option<()> {
        self.clear();
        for quad_pos in pos.as_chunks::<72>().0 {
            let p = stored::<3>(quad_pos).map(|v| v.map(f32::from_bits));
            let cell = cell_of(&p)?;
            let mut located = Located { plain: [NONE; 3], hashed: [NONE; 2], offset: [0.0; 2] };
            let mut offset = None;
            for a in 0..3 {
                let b = cell[a] as f32;
                let local = p.map(|v| v[a] - b);
                located.plain[a] = self.plain[a].offer(&local, b);
                if let (Some(origin), true) = (origin, a != 1)
                    && !on_grid(&local)
                {
                    let d = offset.get_or_insert_with(|| {
                        hash_offset(origin[0].wrapping_add(cell[0]), origin[1].wrapping_add(cell[2]))
                    })[a / 2];
                    located.hashed[a / 2] = self.hashed[a / 2].offer(&local.map(|l| l - d), b);
                    located.offset[a / 2] = d;
                }
            }
            self.located.push(located);
            self.cells.push(cell);
        }
        self.plain.iter_mut().chain(&mut self.hashed).for_each(Arena::plant);

        let (mut block, mut last) = (0u32, None);
        let quads = pos.as_chunks::<72>().0.iter().zip(uv.as_chunks::<48>().0);
        for (q, (quad_pos, quad_uv)) in quads.enumerate() {
            let (cell, located) = (self.cells[q], &self.located[q]);
            if last != Some(cell) {
                block += 1;
                last = Some(cell);
            }
            let bits = stored::<3>(quad_pos);
            let uv = stored::<2>(quad_uv);
            let mut pick = Pick { plain: [NONE; 3], hashed: [NONE; 2], uv: NONE };
            for a in 0..3 {
                let b = cell[a] as f32;
                let want = bits.map(|v| v[a]);
                let local = want.map(|w| f32::from_bits(w) - b);
                pick.plain[a] = self.plain[a].find(located.plain[a], local, block, |s| fits(s, None, b, &want));
                let head = if a == 1 { NONE } else { located.hashed[a / 2] };
                if head != NONE {
                    let d = located.offset[a / 2];
                    pick.hashed[a / 2] =
                        self.hashed[a / 2].find(head, local.map(|l| l - d), block, |s| fits(s, Some(d), b, &want));
                }
            }
            pick.uv = self.uv_id(uv.as_flattened());
            self.picks.push(pick);
        }
        self.assign(pos)
    }

    fn uv_id(&mut self, bits: &[u32]) -> u32 {
        let next = self.uv.len() as u32;
        let row: [u32; 8] = bits.try_into().unwrap();
        let key = (pack(row[..4].try_into().unwrap()), pack(row[4..].try_into().unwrap()));
        *self.uv.entry(key).or_insert_with(|| {
            self.tables.uv.push(row);
            next
        })
    }

    /// Decides each quad's form per axis and numbers shapes and templates by first use.
    fn assign(&mut self, pos: &[u8]) -> Option<()> {
        let mut zero = [NONE; 3];
        for (q, pick) in self.picks.iter().enumerate() {
            let mut template = [0, 0, 0, pick.uv];
            for a in 0..3 {
                let (p, h) = (pick.plain[a], if a == 1 { NONE } else { pick.hashed[a / 2] });
                let use_hashed = h != NONE
                    && (p == NONE || self.hashed[a / 2].blocks[h as usize] > self.plain[a].blocks[p as usize]);
                let (arena, id) = if use_hashed { (&mut self.hashed[a / 2], h) } else { (&mut self.plain[a], p) };
                let (shapes, hashed) = (&mut self.tables.shapes[a], &mut self.tables.hashed[a]);
                let mut add = |bits: [u32; 4], is_hashed: bool| {
                    shapes.push(bits);
                    hashed.push(is_hashed);
                    shapes.len() as u32 - 1
                };
                template[a] = if id == NONE {
                    self.exceptions.push(((q * 3 + a) as u32, stored::<3>(&pos[72 * q..]).map(|v| v[a])));
                    if zero[a] == NONE {
                        zero[a] = add([0; 4], false);
                    }
                    zero[a]
                } else {
                    let slot = &mut arena.table_id[id as usize];
                    if *slot == NONE {
                        *slot = add(arena.values[id as usize], use_hashed);
                    }
                    *slot
                };
            }
            let next = self.tables.templates.len() as u32;
            let templates = &mut self.tables.templates;
            self.ids.push(*self.templates.entry(pack(template)).or_insert_with(|| {
                templates.push(template);
                next
            }));
        }
        let t = &self.tables;
        (t.shapes.iter().all(|s| s.len() <= MAX_IDS) && t.uv.len() <= MAX_IDS && t.templates.len() <= MAX_IDS)
            .then_some(())
    }
}
