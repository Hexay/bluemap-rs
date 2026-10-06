//! The fixed call sequences `MathRef.java` used to produce `data/*.rs`; each mirrors BlueMap code verbatim.
#![allow(dead_code)]

use bm_math::{MatrixM3f, MatrixM4f, VectorM3f};

pub fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

pub fn bits<const N: usize>(v: [f32; N]) -> [u32; N] {
    v.map(f32::to_bits)
}

pub fn base4() -> MatrixM4f {
    *MatrixM4f::default().translate(1.25, -3.5, 7.0).scale(2.0, 0.5, -1.0)
}

pub fn base3() -> MatrixM3f {
    *MatrixM3f::default().translate(0.25, -0.75).scale(1.5, -2.0, 1.0)
}

pub fn m3_chain() -> MatrixM3f {
    *base3().rotate(33.3, 0.0, 0.0, 1.0).rotate_yxz(10.0, 20.0, 30.0)
}

/// `Variant.init`.
pub fn variant(x: f32, y: f32, z: f32) -> MatrixM4f {
    *MatrixM4f::default().translate(-0.5, -0.5, -0.5).rotate_yxz(-x, -y, -z).translate(0.5, 0.5, 0.5)
}

/// `Rotation.init` after the angle/axis to x/y/z step.
pub fn model_rotation([ox, oy, oz]: [f32; 3], [x, y, z]: [f32; 3], rescale: bool) -> MatrixM4f {
    let mut matrix = MatrixM4f::default();
    if x != 0.0 || y != 0.0 || z != 0.0 {
        matrix.translate(-ox, -oy, -oz).rotate_yxz(x, y, z);
        if rescale {
            let mut axis = VectorM3f::default();
            let s_x = 1.0 / axis.set(1.0, 0.0, 0.0).rotate_and_scale(&matrix).absolute().max();
            let s_y = 1.0 / axis.set(0.0, 1.0, 0.0).rotate_and_scale(&matrix).absolute().max();
            let s_z = 1.0 / axis.set(0.0, 0.0, 1.0).rotate_and_scale(&matrix).absolute().max();
            matrix.identity().translate(-ox, -oy, -oz).scale(s_x, s_y, s_z).rotate_yxz(x, y, z);
        }
        matrix.translate(ox, oy, oz);
    }
    matrix
}
