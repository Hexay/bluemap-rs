//! `ArrayTileModel` operations on a [`TileModel`]: appending faces and transforming the faces from an index on,
//! with Java's float arithmetic.

use bm_format::prbm::TileModel;
use bm_math::MatrixM4f;

/// `ArrayTileModel.MAX_CAPACITY`: adding past this aborts the tile, keeping what was built.
pub const MAX_FACES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityReached;

/// One triangle's attributes; colour and light apply to the whole face.
pub struct Face {
    pub positions: [[f32; 3]; 3],
    pub uvs: [[f32; 2]; 3],
    pub aos: [f32; 3],
    pub color: [f32; 3],
    pub sunlight: i32,
    pub blocklight: i32,
    pub material: u32,
}

pub trait MeshExt {
    fn faces(&self) -> usize;
    /// Room for `count` more faces (`ensureCapacity`).
    fn reserve_faces(&self, count: usize) -> Result<(), CapacityReached>;
    fn push_face(&mut self, f: &Face);
    fn transform_from(&mut self, start: usize, m: &MatrixM4f);
    fn translate_from(&mut self, start: usize, dx: f32, dy: f32, dz: f32);
    fn scale_from(&mut self, start: usize, s: f32);
}

impl MeshExt for TileModel {
    fn faces(&self) -> usize {
        self.material.len()
    }

    fn reserve_faces(&self, count: usize) -> Result<(), CapacityReached> {
        if self.faces() + count > MAX_FACES { Err(CapacityReached) } else { Ok(()) }
    }

    fn push_face(&mut self, f: &Face) {
        self.position.extend(f.positions.as_flattened());
        self.uv.extend(f.uvs.as_flattened());
        self.ao.extend(f.aos);
        self.color.extend(f.color);
        // `(byte)` casts
        self.sunlight.push(f.sunlight as u8);
        self.blocklight.push(f.blocklight as u8);
        self.material.push(f.material);
    }

    fn transform_from(&mut self, start: usize, t: &MatrixM4f) {
        for p in self.position[start * 9..].as_chunks_mut::<3>().0 {
            let [x, y, z] = *p;
            *p = [
                t.m00 * x + t.m01 * y + t.m02 * z + t.m03,
                t.m10 * x + t.m11 * y + t.m12 * z + t.m13,
                t.m20 * x + t.m21 * y + t.m22 * z + t.m23,
            ];
        }
    }

    fn translate_from(&mut self, start: usize, dx: f32, dy: f32, dz: f32) {
        for p in self.position[start * 9..].as_chunks_mut::<3>().0 {
            p[0] += dx;
            p[1] += dy;
            p[2] += dz;
        }
    }

    fn scale_from(&mut self, start: usize, s: f32) {
        self.position[start * 9..].iter_mut().for_each(|v| *v *= s);
    }
}

/// `ArrayTileModel.sort`: faces ordered by material, stable. `out` is overwritten.
pub fn sort_by_material(model: &TileModel, out: &mut TileModel) {
    out.clear();
    let mut order: Vec<u32> = (0..model.faces() as u32).collect();
    order.sort_by_key(|&i| model.material[i as usize]);
    gather::<9, _>(&model.position, &order, &mut out.position);
    gather::<6, _>(&model.uv, &order, &mut out.uv);
    gather::<3, _>(&model.ao, &order, &mut out.ao);
    gather::<3, _>(&model.color, &order, &mut out.color);
    gather::<1, _>(&model.sunlight, &order, &mut out.sunlight);
    gather::<1, _>(&model.blocklight, &order, &mut out.blocklight);
    gather::<1, _>(&model.material, &order, &mut out.material);
}

/// `out` = the `N`-element records of `src` in `order`.
fn gather<const N: usize, T: Copy + Default>(src: &[T], order: &[u32], out: &mut Vec<T>) {
    let src = src.as_chunks::<N>().0;
    out.resize(order.len() * N, T::default());
    for (dst, &i) in out.as_chunks_mut::<N>().0.iter_mut().zip(order) {
        *dst = src[i as usize];
    }
}
