//! Encoder side of the geometry: a cell for every quad and, per axis, a shape that reproduces its position bits.
//!
//! A value is `fl32(shape + B)`, so a shape first seen at a large B has lost fraction bits and may not fit the
//! same model vertex at a smaller B; then a second shape joins the candidates of that key. Whether a quad's x or z
//! is relative to the block's hash offset is not known up front: both forms are tried, and the one whose shape
//! serves more blocks wins (an offset plant's plain shape serves one block, its hashed shape every such plant).

use rustc_hash::FxHashMap;

use super::CELL_LIMIT;
use super::face::{MAX_IDS, Tables, hash_offset};
use super::view::QV;

const NONE: u32 = u32::MAX;

/// Candidate shapes of one axis in one form.
#[derive(Default)]
struct Arena {
    values: Vec<[u32; 4]>,
    /// Next candidate with the same key.
    next: Vec<u32>,
    heads: FxHashMap<[i32; 4], u32>,
    /// Blocks that used the shape, and the last of them.
    blocks: Vec<u32>,
    last_block: Vec<u32>,
    /// Id in the final table, once a quad took the shape.
    table_id: Vec<u32>,
}

impl Arena {
    fn clear(&mut self) {
        self.values.clear();
        self.next.clear();
        self.heads.clear();
        self.blocks.clear();
        self.last_block.clear();
        self.table_id.clear();
    }

    /// The candidate for `local` that `fits`, adding `local` itself if none does and it fits.
    #[inline]
    fn find(&mut self, local: [f32; 4], block: u32, fits: impl Fn(&[u32; 4]) -> bool) -> u32 {
        // any deterministic coarse key works; a value straddling a key boundary only costs a duplicate shape
        let key = local.map(|l| (l * 4096.0) as i32);
        let head = self.heads.get(&key).copied().unwrap_or(NONE);
        let mut id = head;
        while id != NONE && !fits(&self.values[id as usize]) {
            id = self.next[id as usize];
        }
        if id == NONE {
            let bits = local.map(f32::to_bits);
            if !fits(&bits) {
                return NONE;
            }
            id = self.values.len() as u32;
            self.values.push(bits);
            self.next.push(head);
            self.blocks.push(0);
            self.last_block.push(NONE);
            self.table_id.push(NONE);
            self.heads.insert(key, id);
        }
        if self.last_block[id as usize] != block {
            self.last_block[id as usize] = block;
            self.blocks[id as usize] += 1;
        }
        id
    }
}

/// A quad's candidate per axis in the plain form and, for x and z, in the hashed form.
struct Pick {
    plain: [u32; 3],
    hashed: [u32; 2],
    uv: u32,
}

#[derive(Default)]
pub(super) struct Shapes {
    plain: [Arena; 3],
    hashed: [Arena; 2],
    uv: FxHashMap<[u32; 8], u32>,
    templates: FxHashMap<[u32; 4], u32>,
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
        self.picks.clear();
        self.cells.clear();
        self.ids.clear();
        self.tables.clear();
        self.exceptions.clear();
    }

    /// Fills cells, template ids, tables and exceptions from the PRBM position and uv payloads; `None` if the tile
    /// is out of the format's range.
    pub fn build(&mut self, pos: &[u8], uv: &[u8], origin: Option<[i32; 2]>) -> Option<()> {
        self.clear();
        let mut block = 0u32;
        let mut last = None;
        for (quad_pos, quad_uv) in pos.as_chunks::<72>().0.iter().zip(uv.as_chunks::<48>().0) {
            let bits = stored::<3>(quad_pos);
            let p = bits.map(|v| v.map(f32::from_bits));
            let cell = cell_of(&p)?;
            if last != Some(cell) {
                block += 1;
                last = Some(cell);
            }
            let mut pick =
                Pick { plain: [NONE; 3], hashed: [NONE; 2], uv: self.uv_id(stored::<2>(quad_uv).as_flattened()) };
            let mut offset = None;
            for a in 0..3 {
                let b = cell[a] as f32;
                let want = bits.map(|v| v[a]);
                let local = p.map(|v| (f64::from(v[a]) - f64::from(b)) as f32);
                pick.plain[a] = self.plain[a]
                    .find(local, block, |s| (0..4).all(|v| (f32::from_bits(s[v]) + b).to_bits() == want[v]));
                let on_grid = local.iter().all(|&l| ((l * 16.0) as i32) as f32 == l * 16.0);
                if let (Some(origin), false, true) = (origin, on_grid, a != 1) {
                    let d = offset.get_or_insert_with(|| {
                        hash_offset(origin[0].wrapping_add(cell[0]), origin[1].wrapping_add(cell[2]))
                    })[a / 2];
                    let base = local.map(|l| (f64::from(l) - f64::from(d)) as f32);
                    pick.hashed[a / 2] = self.hashed[a / 2]
                        .find(base, block, |s| (0..4).all(|v| ((f32::from_bits(s[v]) + d) + b).to_bits() == want[v]));
                }
            }
            self.picks.push(pick);
            self.cells.push(cell);
        }
        self.assign(pos)
    }

    fn uv_id(&mut self, bits: &[u32]) -> u32 {
        let next = self.uv.len() as u32;
        let key: [u32; 8] = bits.try_into().unwrap();
        *self.uv.entry(key).or_insert_with(|| {
            self.tables.uv.push(key);
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
            self.ids.push(*self.templates.entry(template).or_insert_with(|| {
                templates.push(template);
                next
            }));
        }
        let t = &self.tables;
        (t.shapes.iter().all(|s| s.len() <= MAX_IDS) && t.uv.len() <= MAX_IDS && t.templates.len() <= MAX_IDS)
            .then_some(())
    }
}
