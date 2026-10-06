//! Vector ops against Java BlueMap (see `data/vector.rs`).

mod common;
#[allow(clippy::type_complexity)]
#[rustfmt::skip]
#[path = "data/vector.rs"]
mod data;

use bm_java::trig;
use bm_math::{VectorM2f, VectorM2i, VectorM3f};
use common::*;

const PTS: [[f32; 3]; 6] =
    [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.25, 0.75, 0.5], [16.0, 0.0, 3.0], [-0.5, 2.25, -7.125], [0.1, 0.2, 0.3]];

fn v3([x, y, z]: [f32; 3]) -> VectorM3f {
    VectorM3f::new(x, y, z)
}

fn b3(v: &VectorM3f) -> [u32; 3] {
    bits([v.x, v.y, v.z])
}

fn b2(v: &VectorM2f) -> [u32; 2] {
    bits([v.x, v.y])
}

#[test]
fn transforms() {
    let mats = [
        variant(0.0, 90.0, 0.0),
        variant(90.0, 270.0, 0.0),
        *base4().rotate_yxz(-22.5, -45.0, 0.0),
        model_rotation([8.0; 3], [0.0, 0.0, 22.5], true),
    ];
    let m3 = m3_chain();
    for (i, p, transformed, rotated, by_m3) in data::TRANSFORM {
        let p = p.map(f);
        assert_eq!(b3(v3(p).transform(&mats[*i])), *transformed, "transform {i} {p:?}");
        assert_eq!(b3(v3(p).rotate_and_scale(&mats[*i])), *rotated, "rotate_and_scale {i} {p:?}");
        assert_eq!(b3(v3(p).transform_m3(&m3)), *by_m3, "transform_m3 {p:?}");
    }
}

#[test]
fn vec3_ops() {
    for (i, (p, norm, cross, dot, len, len_sq, max, min, abs, mul)) in data::VEC3.iter().enumerate() {
        let i = i + 1;
        let (v, w) = (v3(PTS[i]), v3(PTS[(i % (PTS.len() - 1)) + 1]));
        assert_eq!(*p, bits(PTS[i]));
        let neg = v3([-v.x, v.y - 1.0, -v.z]);
        assert_eq!(b3({ v }.normalize()), *norm);
        assert_eq!(b3({ v }.cross(&w)), *cross);
        assert_eq!(v.dot(&w).to_bits(), *dot);
        assert_eq!(v.length().to_bits(), *len);
        assert_eq!(v.length_squared().to_bits(), *len_sq);
        assert_eq!((neg.max().to_bits(), neg.min().to_bits()), (*max, *min));
        assert_eq!(b3({ neg }.absolute()), *abs);
        assert_eq!(b3({ v }.mul(1.5)), *mul);
    }
}

#[test]
fn uv_lock_rotation() {
    for (r, uv, expected) in data::UV_LOCK {
        let r = f(*r);
        let (cx, cy) = (trig::cos(r as f64), trig::sin(r as f64));
        let mut v = VectorM2f::new(f(uv[0]), f(uv[1]));
        v.translate(-0.5, -0.5).rotate(cx, cy).translate(0.5, 0.5);
        assert_eq!(b2(&v), *expected, "radians {r} uv {uv:?}");
    }
}

#[test]
fn vec2_ops() {
    let m3 = m3_chain();
    for (v, transformed, norm, len, angle, deg) in data::VEC2 {
        let v = VectorM2f::new(f(v[0]), f(v[1]));
        let mut t = v;
        assert_eq!(b2(t.transform(&m3)), *transformed);
        let mut n = v;
        assert_eq!(b2(n.normalize()), *norm);
        assert_eq!(v.length().to_bits(), *len);
        assert_eq!(v.angle_to(0.0, -1.0).to_bits(), *angle, "{v:?}");
        // LiquidModelRenderer's flow angle
        assert_eq!((v.angle_to(0.0, -1.0) as f64 * trig::RAD_TO_DEG) as i32, *deg);
    }
}

#[test]
fn vec2i_ops() {
    for &(v, floor_div, div, norm, len, hash) in data::VEC2I {
        let v = VectorM2i::new(v[0], v[1]);
        let xy = |v: &mut VectorM2i| [v.x, v.y];
        assert_eq!(xy({ v }.floor_div(16, -3)), floor_div);
        assert_eq!(xy({ v }.div(16, -3)), div);
        assert_eq!(xy({ v }.normalize()), norm);
        assert_eq!((v.length(), v.java_hash()), (len, hash));
    }
}
