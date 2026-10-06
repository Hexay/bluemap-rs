//! `java.lang.Math` functions whose semantics differ from Rust's `std` equivalents.

const DEGREES_TO_RADIANS: f64 = 0.017453292519943295;

/// `Math.toRadians` since JDK 9 (one multiply; JDK 8 divided by 180 then multiplied by π).
pub fn to_radians(deg: f64) -> f64 {
    deg * DEGREES_TO_RADIANS
}

/// `Math.max(float, float)`: NaN wins and `+0.0 > -0.0`, unlike `f32::max`.
pub fn max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Math.min(float, float)`: NaN wins and `-0.0 < +0.0`, unlike `f32::min`.
pub fn min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a <= b { a } else { b }
}

/// `Math.floorDiv(int, int)`: rounds toward negative infinity (`div_euclid` differs for negative divisors).
pub fn floor_div(x: i32, y: i32) -> i32 {
    let q = x.wrapping_div(y);
    if (x ^ y) < 0 && q.wrapping_mul(y) != x { q - 1 } else { q }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_semantics() {
        assert_eq!(max_f32(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(max_f32(0.0, -0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(min_f32(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(min_f32(-0.0, 0.0).to_bits(), (-0.0f32).to_bits());
        assert!(max_f32(1.0, f32::NAN).is_nan());
        assert!(min_f32(f32::NAN, 1.0).is_nan());
        assert_eq!(floor_div(-7, 2), -4);
        assert_eq!(floor_div(7, -2), -4);
        assert_eq!(floor_div(-7, -2), 3);
        assert_eq!(floor_div(-8, 2), -4);
        assert_eq!(floor_div(i32::MIN, -1), i32::MIN);
    }
}
