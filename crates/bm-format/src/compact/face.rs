//! Shape and template tables, and what a template means: block-local vertex values per axis plus the facts the AO
//! prediction reads off them. Encoder and decoder derive [`Face`]s from the same tables with the same code.

use super::CompactError;
use super::bytes::{Reader, stream};

/// Shape and template ids are stored as u16.
pub(super) const MAX_IDS: usize = 1 << 16;
/// How close to a block face a block-local value counts as on it, for the AO prediction only.
const SNAP: f32 = 1.0 / 4096.0;

#[derive(Default)]
pub(super) struct Tables {
    /// Per axis: f32 bits of the 4 vertices.
    pub shapes: [Vec<[u32; 4]>; 3],
    /// Per axis and shape: relative to the block's hash offset.
    pub hashed: [Vec<bool>; 3],
    pub uv: Vec<[u32; 8]>,
    /// x, y, z and uv shape of each template.
    pub templates: Vec<[u32; 4]>,
}

#[derive(Clone, Copy)]
pub(super) struct Face {
    /// `[axis][vertex]`
    pub local: [[f32; 4]; 3],
    pub hashed: [bool; 3],
    pub uv: u32,
    /// Per vertex and axis: +1 on the block's upper face, -1 on its lower one, 0 inside.
    pub side: [[i8; 3]; 4],
    /// Sign of the quad's normal if it is axis-aligned, else zero.
    pub normal: [i8; 3],
    /// A whole face of the block's unit cube.
    pub full: bool,
}

impl Face {
    fn new(local: [[f32; 4]; 3], hashed: [bool; 3], uv: u32) -> Self {
        // rotated models leave float noise on the block's faces (5.96e-8 for 0, 0.99999994 for 1)
        let snapped = local.map(|axis| {
            axis.map(|l| {
                if l.abs() < SNAP {
                    0.0
                } else if (l - 1.0).abs() < SNAP {
                    1.0
                } else {
                    l
                }
            })
        });
        let mut side = [[0i8; 3]; 4];
        for (v, s) in side.iter_mut().enumerate() {
            for a in 0..3 {
                let l = snapped[a][v];
                s[a] = if hashed[a] { 0 } else { i8::from(l == 1.0) - i8::from(l == 0.0) };
            }
        }
        let edge = |v: usize| [0, 1, 2].map(|a| f64::from(snapped[a][v]) - f64::from(snapped[a][0]));
        let (e1, e2) = (edge(1), edge(2));
        let cross = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let mut normal = cross.map(|c| i8::from(c > 0.0) - i8::from(c < 0.0));
        if normal.iter().filter(|&&n| n != 0).count() != 1 {
            normal = [0; 3];
        }
        let corners = side.iter().all(|s| s.iter().all(|&c| c != 0));
        let spans = (0..3).filter(|&a| side.iter().any(|s| s[a] != side[0][a])).count();
        Self { local, hashed, uv, side, normal, full: normal != [0; 3] && corners && spans == 2 }
    }

    /// f32 bits of the 4 vertices' xyz for the block at `cell`, whose hash offset is `d` (x, z).
    #[inline(always)]
    pub fn positions(&self, cell: [i32; 3], d: [f32; 2]) -> [[u32; 3]; 4] {
        let mut out = [[0u32; 3]; 4];
        for a in 0..3 {
            let b = cell[a] as f32;
            for (v, vertex) in out.iter_mut().enumerate() {
                let l = self.local[a][v];
                vertex[a] = (if self.hashed[a] { l + d[a / 2] } else { l } + b).to_bits();
            }
        }
        out
    }
}

/// BlueMap's random block offset (dx, dz) of a world block column: `ResourceModelRenderer.hashToFloat`.
#[inline]
pub(super) fn hash_offset(x: i32, z: i32) -> [f32; 2] {
    let unit = |seed: i64| {
        let hash = i64::from(x).wrapping_mul(73428767) ^ i64::from(z).wrapping_mul(4382893) ^ seed.wrapping_mul(457);
        (hash.wrapping_mul(hash.wrapping_add(456149)) & 0x00ff_ffff) as f32 / 16_777_216.0
    };
    [(unit(123984) - 0.5) * 0.75, (unit(345542) - 0.5) * 0.75]
}

impl Tables {
    pub fn clear(&mut self) {
        self.shapes.iter_mut().for_each(Vec::clear);
        self.hashed.iter_mut().for_each(Vec::clear);
        self.uv.clear();
        self.templates.clear();
    }

    /// The meaning of every template; `None` if one names a shape that does not exist.
    pub fn faces(&self, out: &mut Vec<Face>) -> Option<()> {
        out.clear();
        for t in &self.templates {
            let mut local = [[0f32; 4]; 3];
            let mut hashed = [false; 3];
            for a in 0..3 {
                local[a] = self.shapes[a].get(t[a] as usize)?.map(f32::from_bits);
                hashed[a] = self.hashed[a][t[a] as usize];
            }
            self.uv.get(t[3] as usize)?;
            out.push(Face::new(local, hashed, t[3]));
        }
        Some(())
    }

    pub fn write(&self, body: &mut Vec<u8>) {
        for a in 0..3 {
            stream(body, |out| {
                out.extend((self.shapes[a].len() as u32).to_le_bytes());
                out.extend(self.hashed[a].iter().map(|&h| u8::from(h)));
                columns(out, &self.shapes[a]);
            });
        }
        stream(body, |out| {
            out.extend((self.uv.len() as u32).to_le_bytes());
            columns(out, &self.uv);
        });
        stream(body, |out| {
            out.extend((self.templates.len() as u32).to_le_bytes());
            for c in 0..4 {
                self.templates.iter().for_each(|t| out.extend((t[c] as u16).to_le_bytes()));
            }
        });
    }

    pub fn read(&mut self, r: &mut Reader) -> Result<(), CompactError> {
        self.clear();
        for a in 0..3 {
            let mut s = r.stream(None)?;
            let n = count(&mut s)?;
            self.hashed[a].extend(s.take(n)?.iter().map(|&h| h != 0));
            read_columns(&mut s, n, &mut self.shapes[a])?;
        }
        let mut s = r.stream(None)?;
        let n = count(&mut s)?;
        read_columns(&mut s, n, &mut self.uv)?;
        let mut s = r.stream(None)?;
        let n = count(&mut s)?;
        let ids = s.take(n * 8)?;
        let id = |c: usize, t: usize| u32::from(u16::from_le_bytes([ids[2 * (c * n + t)], ids[2 * (c * n + t) + 1]]));
        self.templates.extend((0..n).map(|t| [id(0, t), id(1, t), id(2, t), id(3, t)]));
        if s.is_empty() { Ok(()) } else { Err(CompactError::Corrupt("template table")) }
    }
}

fn count(s: &mut Reader) -> Result<usize, CompactError> {
    Some(s.u32()? as usize).filter(|&n| n <= MAX_IDS).ok_or(CompactError::Corrupt("table size"))
}

/// Rows of `N` u32 column by column: values of one vertex end up adjacent.
fn columns<const N: usize>(out: &mut Vec<u8>, rows: &[[u32; N]]) {
    for c in 0..N {
        rows.iter().for_each(|row| out.extend(row[c].to_le_bytes()));
    }
}

fn read_columns<const N: usize>(s: &mut Reader, n: usize, rows: &mut Vec<[u32; N]>) -> Result<(), CompactError> {
    let data = s.take(n * N * 4)?;
    let value = |c: usize, i: usize| u32::from_le_bytes(data[4 * (c * n + i)..4 * (c * n + i) + 4].try_into().unwrap());
    rows.extend((0..n).map(|i| std::array::from_fn(|c| value(c, i))));
    if s.is_empty() { Ok(()) } else { Err(CompactError::Corrupt("shape table")) }
}
