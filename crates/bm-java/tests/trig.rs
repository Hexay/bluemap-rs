//! flow-math `TrigMath` and `Math.toRadians` against values printed by Java (see `data/trig.rs`).

#[rustfmt::skip]
#[path = "data/trig.rs"]
mod data;

use bm_java::{math, trig};

#[test]
fn sin_table_matches_java() {
    assert_eq!(trig::table_fingerprint(), data::TABLE_FINGERPRINT);
}

#[test]
fn to_radians() {
    for &(deg, rad) in data::TO_RADIANS {
        assert_eq!(math::to_radians(f64::from_bits(deg)).to_bits(), rad, "{}", f64::from_bits(deg));
    }
}

#[test]
fn sin_cos() {
    for &(angle, sin, cos) in data::SIN_COS {
        let a = f64::from_bits(angle);
        assert_eq!((trig::sin(a).to_bits(), trig::cos(a).to_bits()), (sin, cos), "angle {a}");
    }
}

#[test]
fn atan2() {
    for &(y, x, out) in data::ATAN2 {
        let (y, x) = (f64::from_bits(y), f64::from_bits(x));
        assert_eq!(trig::atan2(y, x).to_bits(), out, "atan2({y}, {x})");
    }
}

#[test]
fn arc_functions() {
    for &(v, asin, acos, atan) in data::ARC {
        let v = f64::from_bits(v);
        assert_eq!(trig::asin(v).to_bits(), asin, "asin({v})");
        assert_eq!(trig::acos(v).to_bits(), acos, "acos({v})");
        assert_eq!(trig::atan(v).to_bits(), atan, "atan({v})");
    }
}
