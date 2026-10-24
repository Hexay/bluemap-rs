//! What `createElementFace` derives from the model and the variant alone, resolved once per variant: the rotated
//! facing's y and the neighbours each corner's AO tests.

use bm_math::VectorM3f;
use bm_resources::blockstate::Variant;
use bm_resources::model::{BakedElement, Direction};

use crate::relative::{Offset, RelativeOffsets};

/// The faces of an element in render order, each with its corners (indices into the element's 8 corners).
pub(crate) const FACES: [(Direction, [usize; 4]); 6] = [
    (Direction::Down, [0, 2, 3, 1]),
    (Direction::Up, [5, 7, 6, 4]),
    (Direction::North, [2, 0, 4, 6]),
    (Direction::South, [1, 3, 7, 5]),
    (Direction::West, [0, 1, 5, 4]),
    (Direction::East, [3, 2, 6, 7]),
];

/// Filler for unused entries.
const NONE: Offset = Offset { d: [0; 3], slot: 13 };

/// The element's corners, `x` slowest: from/to per axis.
pub(crate) fn corners(e: &BakedElement) -> [[f32; 3]; 8] {
    let ([fx, fy, fz], [tx, ty, tz]) = (e.from, e.to);
    [[fx, fy, fz], [fx, fy, tz], [tx, fy, fz], [tx, fy, tz], [fx, ty, fz], [fx, ty, tz], [tx, ty, fz], [tx, ty, tz]]
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FaceInfo {
    /// y of the face direction, rotated by the element and the variant.
    pub facing_y: f32,
    /// The variant-relative cullface neighbour, if the face has one.
    pub cull: Option<Offset>,
    /// The variant-relative neighbour the face takes its light from.
    pub light: Offset,
    /// Per corner (in [`FACES`] order), the variant-relative neighbours whose `occluding` darkens it.
    pub ao: [Probes; 4],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Probes {
    offsets: [Offset; 4],
    len: u8,
}

impl Probes {
    pub fn iter(&self) -> impl Iterator<Item = Offset> + '_ {
        self.offsets[..self.len as usize].iter().copied()
    }
}

/// [`FaceInfo`] per element, indexed by [`Direction::index`].
pub(crate) fn resolve(elements: &[BakedElement], variant: &Variant, relative: &RelativeOffsets) -> Vec<[FaceInfo; 6]> {
    elements
        .iter()
        .map(|e| {
            let c = corners(e);
            let none = Probes { offsets: [NONE; 4], len: 0 };
            let mut infos = [FaceInfo { facing_y: 0.0, cull: None, light: NONE, ao: [none; 4] }; 6];
            for (dir, ks) in FACES {
                let mut facing = VectorM3f::default();
                facing.set_i(dir.to_vector());
                facing.rotate_and_scale(&e.rotation);
                if variant.transformed {
                    facing.rotate_and_scale(&variant.transform);
                }
                infos[dir.index()] = FaceInfo {
                    facing_y: facing.y,
                    cull: e.face(dir).and_then(|f| f.cullface).map(|d| relative.get(d.to_vector())),
                    light: relative.get(dir.to_vector()),
                    ao: ks.map(|k| probes(c[k], dir, relative)),
                };
            }
            infos
        })
        .collect()
}

/// `testAo`'s neighbour tests for one corner, in element space.
fn probes(vertex: [f32; 3], dir: Direction, relative: &RelativeOffsets) -> Probes {
    let side = |c: f32| {
        if c == 16.0 {
            1
        } else if c == 0.0 {
            -1
        } else {
            0
        }
    };
    let (x, y, z) = (side(vertex[0]), side(vertex[1]), side(vertex[2]));
    let [dx, dy, dz] = dir.to_vector();
    let tests = [
        (x * dx + y * dy > 0, [x, y, 0]),
        (x * dx + z * dz > 0, [x, 0, z]),
        (y * dy + z * dz > 0, [0, y, z]),
        (x * dx + y * dy + z * dz > 0, [x, y, z]),
    ];
    let mut p = Probes { offsets: [NONE; 4], len: 0 };
    for (_, o) in tests.into_iter().filter(|t| t.0) {
        p.offsets[p.len as usize] = relative.get(o);
        p.len += 1;
    }
    p
}
