//! Matrix builders and quaternions against Java BlueMap (see `data/matrix.rs`).

mod common;
#[allow(clippy::type_complexity)]
#[rustfmt::skip]
#[path = "data/matrix.rs"]
mod data;

use bm_math::{MatrixM3f, quat};
use common::*;

#[test]
fn variant_transforms() {
    let q = [0.0, 90.0, 180.0, 270.0];
    let mut n = 0;
    for x in q {
        for y in q {
            for z in q {
                let (input, expected) = data::VARIANT[n];
                assert_eq!(input, bits([x, y, z]));
                assert_eq!(bits(variant(x, y, z).to_array()), expected, "variant {x} {y} {z}");
                n += 1;
            }
        }
    }
    assert_eq!(n, data::VARIANT.len());
}

#[test]
fn euler_rotations() {
    for (input, quats, m4s, m3s) in data::EULER {
        let [p, y, r] = input.map(f);
        let got_quats = [quat::euler_xyz(p, y, r), quat::euler_zyx(p, y, r), quat::euler_yxz(p, y, r)];
        assert_eq!(got_quats.map(|q| q.map(f64::to_bits)), *quats, "quats {p} {y} {r}");
        let got_m4 = [
            base4().rotate_xyz(p, y, r).to_array(),
            base4().rotate_zyx(p, y, r).to_array(),
            base4().rotate_yxz(p, y, r).to_array(),
        ];
        assert_eq!(got_m4.map(bits), *m4s, "m4 {p} {y} {r}");
        let got_m3 = [
            base3().rotate_xyz(p, y, r).to_array(),
            base3().rotate_zyx(p, y, r).to_array(),
            base3().rotate_yxz(p, y, r).to_array(),
        ];
        assert_eq!(got_m3.map(bits), *m3s, "m3 {p} {y} {r}");
    }
}

#[test]
fn axis_angle_rotations() {
    for (input, q, m4, m3) in data::AXIS_ANGLE {
        let [a, x, y, z] = input.map(f);
        assert_eq!(quat::axis_angle(a, x, y, z).map(f64::to_bits), *q, "quat {a} ({x} {y} {z})");
        assert_eq!(bits(base4().rotate(a, x, y, z).to_array()), *m4, "m4 {a} ({x} {y} {z})");
        assert_eq!(bits(base3().rotate(a, x, y, z).to_array()), *m3, "m3 {a} ({x} {y} {z})");
    }
}

#[test]
fn model_element_rotations() {
    for (input, rescale, expected) in data::MODEL_ROTATION {
        let [ox, oy, oz, x, y, z] = input.map(f);
        let got = model_rotation([ox, oy, oz], [x, y, z], *rescale);
        assert_eq!(bits(got.to_array()), *expected, "origin {ox} {oy} {oz} rot {x} {y} {z} rescale {rescale}");
    }
}

#[test]
fn m3_ops() {
    let m = m3_chain();
    assert_eq!(bits(m.to_array()), data::M3_CHAIN);
    assert_eq!(m.determinant().to_bits(), data::M3_DET);
    assert_eq!(bits(MatrixM3f::from_array(m.to_array()).invert().to_array()), data::M3_INVERTED);
    let mut multiplied = m;
    multiplied.multiply([0.5, -1.25, 3.0, 7.0, 0.1, -0.3, 2.5, 8.0, -9.75]);
    assert_eq!(bits(multiplied.to_array()), data::M3_MULTIPLIED);
}

#[test]
fn m4_ops() {
    let mut m = base4();
    m.rotate_yxz(-22.5, -45.0, 0.0);
    m.multiply([0.5, -1.25, 3.0, 7.0, 0.1, -0.3, 2.5, 8.0, -9.75, 1.0, 2.0, 3.0, 0.2, 0.4, 0.6, 1.1]);
    assert_eq!(bits(m.to_array()), data::M4_MULTIPLIED);
    m.multiply_to_m3(&m3_chain());
    assert_eq!(bits(m.to_array()), data::M4_MULTIPLIED_TO_M3);
}
