//! AO predicted from the tile's own quads, after `ResourceModelRenderer.testAo`: a corner's level is
//! `1 - 0.25 * min(3, occluding blocks among the up to four around the corner in front of the face)`.
//! The decoder has no blocks, so "occluding" is "the cell holds a full block face of a material flagged as
//! occluder". Stored per vertex: `(actual - predicted) mod 4`.

use super::face::Face;
use super::{CELL_LIMIT, MAX_GRID};

/// PRBM ao byte of `n` occluders.
pub(super) const LEVELS: [u8; 4] = [255, 191, 127, 63];
const DISABLED: isize = isize::MIN;

/// One bit per cell of the tile's bounding box plus a one-cell border.
#[derive(Default)]
pub(super) struct Grid {
    bits: Vec<u64>,
    min: [i32; 3],
    size: [usize; 3],
}

impl Grid {
    /// Clears the grid to span `cells`; `None` if they lie too far apart.
    pub fn reset(&mut self, cells: &[[i32; 3]]) -> Option<()> {
        let (mut min, mut max) = ([i32::MAX; 3], [i32::MIN; 3]);
        for c in cells {
            for a in 0..3 {
                min[a] = min[a].min(c[a]);
                max[a] = max[a].max(c[a]);
            }
        }
        if cells.is_empty() || min.iter().chain(&max).any(|v| v.abs() > CELL_LIMIT) {
            return None;
        }
        self.min = min.map(|v| v - 1);
        self.size = [0, 1, 2].map(|a| (max[a] - min[a]) as usize + 3);
        let total = self.size.iter().try_fold(1usize, |n, &s| n.checked_mul(s)).filter(|&n| n <= MAX_GRID)?;
        self.bits.clear();
        self.bits.resize(total.div_ceil(64), 0);
        Some(())
    }

    pub fn cells(&self) -> usize {
        self.size.iter().product()
    }

    #[inline]
    pub fn index(&self, c: [i32; 3]) -> usize {
        let at = |a: usize| (c[a] - self.min[a]) as usize;
        (at(0) * self.size[1] + at(1)) * self.size[2] + at(2)
    }

    fn delta(&self, d: [i8; 3]) -> isize {
        (d[0] as isize * self.size[1] as isize + d[1] as isize) * self.size[2] as isize + d[2] as isize
    }

    #[inline]
    pub fn set(&mut self, i: usize) {
        self.bits[i / 64] |= 1 << (i % 64);
    }

    #[inline]
    fn get(&self, i: usize) -> bool {
        self.bits[i / 64] >> (i % 64) & 1 != 0
    }

    #[inline]
    pub fn predict(&self, probes: &Probes, base: usize) -> [u8; 4] {
        probes.predict(base, |i| self.get(i))
    }

    pub fn trim(&mut self) {
        super::trim(&mut self.bits);
    }
}

/// Per vertex the grid offsets of `testAo`'s neighbours: three across an edge, then the one across the corner.
pub(super) struct Probes {
    at: [[isize; 4]; 4],
    pub any: bool,
}

impl Probes {
    pub fn new(face: &Face, grid: &Grid) -> Self {
        let mut at = [[DISABLED; 4]; 4];
        for (v, probes) in at.iter_mut().enumerate() {
            let s = face.side[v];
            let [x, y, z] = [0, 1, 2].map(|a| s[a] * face.normal[a]);
            let tests = [(x + y, [s[0], s[1], 0]), (x + z, [s[0], 0, s[2]]), (y + z, [0, s[1], s[2]]), (x + y + z, s)];
            for (probe, (toward, offset)) in probes.iter_mut().zip(tests) {
                if toward > 0 {
                    *probe = grid.delta(offset);
                }
            }
        }
        Self { at, any: at.iter().flatten().any(|&p| p != DISABLED) }
    }

    /// Per vertex and probe the grid index it reads for a quad whose cell is at `base`, if enabled.
    #[inline]
    pub fn targets(&self, base: usize) -> [[Option<usize>; 4]; 4] {
        self.at.map(|vertex| vertex.map(|d| (d != DISABLED).then(|| base.wrapping_add_signed(d))))
    }

    /// Predicted occluder count of the 4 vertices, `hit` telling whether a grid index is occluding.
    #[inline]
    pub fn predict(&self, base: usize, hit: impl Fn(usize) -> bool) -> [u8; 4] {
        if !self.any {
            return [0; 4];
        }
        self.at.map(|vertex| {
            let test = |d: isize| d != DISABLED && hit(base.wrapping_add_signed(d));
            let edges = u8::from(test(vertex[0])) + u8::from(test(vertex[1])) + u8::from(test(vertex[2]));
            let corner = vertex[3] != DISABLED && (edges >= 2 || test(vertex[3]));
            (edges + u8::from(corner)).min(3)
        })
    }
}

/// `(actual - predicted) mod 4` of 4 vertices in one byte, and back.
pub(super) fn pack(codes: [u8; 4]) -> u8 {
    codes[0] | codes[1] << 2 | codes[2] << 4 | codes[3] << 6
}

pub(super) fn unpack(byte: u8) -> [u8; 4] {
    [byte & 3, byte >> 2 & 3, byte >> 4 & 3, byte >> 6]
}
