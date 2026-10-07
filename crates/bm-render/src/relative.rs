//! `getRotationRelativeBlock` offsets, resolved once per variant instead of rotated and rounded per lookup.

use bm_math::VectorM3f;
use bm_resources::blockstate::Variant;

/// The variant-rotated offset of every offset in `-1..=1`³ (faces, cullfaces and AO only look that far).
#[derive(Clone, Copy, Debug)]
pub struct RelativeOffsets([[i32; 3]; 27]);

impl RelativeOffsets {
    pub fn new(variant: &Variant) -> Self {
        Self(std::array::from_fn(|i| {
            let o = [i as i32 / 9 - 1, i as i32 / 3 % 3 - 1, i as i32 % 3 - 1];
            if !variant.transformed {
                return o;
            }
            let mut v = VectorM3f::default();
            v.set_i(o);
            v.rotate_and_scale(&variant.transform);
            [java_round(v.x), java_round(v.y), java_round(v.z)]
        }))
    }

    /// `offset` must lie in `-1..=1`³.
    pub fn get(&self, [x, y, z]: [i32; 3]) -> [i32; 3] {
        self.0[((x + 1) * 9 + (y + 1) * 3 + (z + 1)) as usize]
    }
}

/// `Math.round(float)`: half up; exact in double for every float that isn't already an integer.
fn java_round(v: f32) -> i32 {
    // integer floor: `f64::floor` is a libm call without SSE4.1
    let d = f64::from(v) + 0.5;
    let t = d as i64;
    (t - i64::from((t as f64) > d)).clamp(i32::MIN.into(), i32::MAX.into()) as i32
}

#[cfg(test)]
mod tests {
    use bm_resources::ResourcePath;

    use super::*;

    #[test]
    fn rounding_matches_java() {
        assert_eq!([java_round(-0.5), java_round(0.5), java_round(-1.5), java_round(0.49999997)], [0, 1, -1, 0]);
        assert_eq!(java_round(-4.371139e-8), 0);
        assert_eq!(java_round(f32::NAN), 0);
        assert_eq!([java_round(-2.5), java_round(-2.6), java_round(1e10), java_round(-1e10)], [-2, -3, i32::MAX, i32::MIN]);
        for v in [-3.75f32, -1.0, -0.25, 0.0, 0.7, 2.5, 1e-9, -1e-9] {
            assert_eq!(java_round(v), (f64::from(v) + 0.5).floor() as i32, "{v}");
        }
    }

    #[test]
    fn identity_and_index_order() {
        let variant = |y| Variant::new(ResourcePath::key("m"), 0.0, y, 0.0, false, 1.0);
        let offsets = RelativeOffsets::new(&variant(0.0));
        for o in [[-1, 0, 1], [1, -1, 0], [0, 0, -1], [1, 1, 1]] {
            assert_eq!(offsets.get(o), o);
        }
        let turned = RelativeOffsets::new(&variant(90.0));
        assert_eq!(turned.get([0, 1, 0]), [0, 1, 0]);
        assert_ne!(turned.get([1, 0, 0]), [1, 0, 0]);
    }
}
