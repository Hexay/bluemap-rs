//! The rotation-relative lookups of `ResourceModelRenderer`: neighbours, AO and the uv-lock angle.

use bm_java::trig;
use bm_math::VectorM3f;
use bm_resources::model::Direction;

use super::Renderer;

impl Renderer<'_, '_, '_> {
    /// `makeRotationRelative`: the variant's rotation, if it has one.
    pub(super) fn make_relative(&self, v: &mut VectorM3f) {
        let variant = self.v.variant;
        if variant.transformed {
            v.rotate_and_scale(&variant.transform);
        }
    }

    /// `getRotationRelativeBlock`'s offset.
    pub(super) fn relative(&self, [x, y, z]: [i32; 3]) -> [i32; 3] {
        let mut v = VectorM3f::default();
        v.set_i([x, y, z]);
        self.make_relative(&mut v);
        [java_round(v.x), java_round(v.y), java_round(v.z)]
    }

    fn occluding(&self, offset: [i32; 3]) -> bool {
        let [dx, dy, dz] = self.relative(offset);
        self.block.neighbor(self.ctx, dx, dy, dz).1.props.occluding
    }

    /// `testAo`: one corner of a face, in element space.
    pub(super) fn ao(&self, vertex: [f32; 3], dir: Direction) -> f32 {
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
        let mut occluding = 0;
        if x * dx + y * dy > 0 && self.occluding([x, y, 0]) {
            occluding += 1;
        }
        if x * dx + z * dz > 0 && self.occluding([x, 0, z]) {
            occluding += 1;
        }
        if y * dy + z * dz > 0 && self.occluding([0, y, z]) {
            occluding += 1;
        }
        if x * dx + y * dy + z * dz > 0 && self.occluding([x, y, z]) {
            occluding += 1;
        }
        (1.0 - occluding.min(3) as f32 * 0.25).clamp(0.0, 1.0)
    }

    /// `uvLockRotation`: the angle between the rotated face's up and world up projected onto the face.
    pub(super) fn uv_lock_rotation(&self, dir: Direction) -> f32 {
        let mut normal = VectorM3f::default();
        normal.set_i(dir.to_vector());
        self.make_relative(&mut normal);
        let mut up = VectorM3f::default();
        up.set_i(dir.local_up().to_vector());
        self.make_relative(&mut up);

        let mut world_up = VectorM3f::new(0.0, 1.0, 0.0);
        let dot = world_up.dot(&normal);
        world_up = normal;
        world_up.mul(dot);
        world_up.set(0.0 - world_up.x, 1.0 - world_up.y, 0.0 - world_up.z);
        if world_up.length_squared() < 0.01 {
            let up_down = if normal.y > 0.0 { Direction::Up } else { Direction::Down };
            world_up.set_i(up_down.local_up().to_vector());
        } else {
            world_up.normalize();
        }

        let dot = up.dot(&world_up);
        up.cross(&world_up);
        trig::atan2(f64::from(up.dot(&normal)), f64::from(dot)) as f32
    }
}

/// `Math.round(float)`: half up; exact in double for every float that isn't already an integer.
fn java_round(v: f32) -> i32 {
    (f64::from(v) + 0.5).floor() as i32
}

/// `ResourceModelRenderer.hashToFloat`, in `[0, 1)`.
pub(super) fn hash_to_float(x: i32, z: i32, seed: i64) -> f32 {
    let hash = i64::from(x).wrapping_mul(73428767) ^ i64::from(z).wrapping_mul(4382893) ^ seed.wrapping_mul(457);
    (hash.wrapping_mul(hash.wrapping_add(456149)) & 0x00ff_ffff) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_java() {
        assert_eq!([java_round(-0.5), java_round(0.5), java_round(-1.5), java_round(0.49999997)], [0, 1, -1, 0]);
        assert_eq!(java_round(-4.371139e-8), 0);
        assert_eq!(java_round(f32::NAN), 0);
    }
}
