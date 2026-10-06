use crate::{MatrixM3f, quat};

/// Row-major 4x4 float matrix (3D affine transforms). Transforms pre-multiply: each call applies after the
/// previous ones, so `translate(-.5).rotate_yxz(..).translate(.5)` rotates around the block centre.
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatrixM4f {
    pub m00: f32, pub m01: f32, pub m02: f32, pub m03: f32,
    pub m10: f32, pub m11: f32, pub m12: f32, pub m13: f32,
    pub m20: f32, pub m21: f32, pub m22: f32, pub m23: f32,
    pub m30: f32, pub m31: f32, pub m32: f32, pub m33: f32,
}

impl Default for MatrixM4f {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl MatrixM4f {
    pub const IDENTITY: Self = Self::from_array([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    pub const fn from_array(m: [f32; 16]) -> Self {
        let [m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23, m30, m31, m32, m33] = m;
        Self { m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23, m30, m31, m32, m33 }
    }

    pub fn to_array(&self) -> [f32; 16] {
        let Self { m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23, m30, m31, m32, m33 } = *self;
        [m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23, m30, m31, m32, m33]
    }

    pub fn set(&mut self, m: [f32; 16]) -> &mut Self {
        *self = Self::from_array(m);
        self
    }

    pub fn identity(&mut self) -> &mut Self {
        self.set(Self::IDENTITY.to_array())
    }

    pub fn translate(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.multiply_to([1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, y, 0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0])
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.multiply_to([x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0])
    }

    pub fn rotate(&mut self, angle: f32, axis_x: f32, axis_y: f32, axis_z: f32) -> &mut Self {
        self.rotate_by_quaternion(quat::to_f32(quat::axis_angle(angle, axis_x, axis_y, axis_z)))
    }

    pub fn rotate_xyz(&mut self, pitch: f32, yaw: f32, roll: f32) -> &mut Self {
        self.rotate_by_quaternion(quat::to_f32(quat::euler_xyz(pitch, yaw, roll)))
    }

    pub fn rotate_zyx(&mut self, pitch: f32, yaw: f32, roll: f32) -> &mut Self {
        self.rotate_by_quaternion(quat::to_f32(quat::euler_zyx(pitch, yaw, roll)))
    }

    pub fn rotate_yxz(&mut self, pitch: f32, yaw: f32, roll: f32) -> &mut Self {
        self.rotate_by_quaternion(quat::to_f32(quat::euler_yxz(pitch, yaw, roll)))
    }

    pub fn rotate_by_quaternion(&mut self, q: [f32; 4]) -> &mut Self {
        let [r00, r01, r02, r10, r11, r12, r20, r21, r22] = quat::rotation_matrix(q);
        self.multiply_to([r00, r01, r02, 0.0, r10, r11, r12, 0.0, r20, r21, r22, 0.0, 0.0, 0.0, 0.0, 1.0])
    }

    /// `self = self * m`.
    pub fn multiply(&mut self, m: [f32; 16]) -> &mut Self {
        let [m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23, m30, m31, m32, m33] = m;
        let t = *self;
        self.set([
            t.m00 * m00 + t.m01 * m10 + t.m02 * m20 + t.m03 * m30,
            t.m00 * m01 + t.m01 * m11 + t.m02 * m21 + t.m03 * m31,
            t.m00 * m02 + t.m01 * m12 + t.m02 * m22 + t.m03 * m32,
            t.m00 * m03 + t.m01 * m13 + t.m02 * m23 + t.m03 * m33,
            t.m10 * m00 + t.m11 * m10 + t.m12 * m20 + t.m13 * m30,
            t.m10 * m01 + t.m11 * m11 + t.m12 * m21 + t.m13 * m31,
            t.m10 * m02 + t.m11 * m12 + t.m12 * m22 + t.m13 * m32,
            t.m10 * m03 + t.m11 * m13 + t.m12 * m23 + t.m13 * m33,
            t.m20 * m00 + t.m21 * m10 + t.m22 * m20 + t.m23 * m30,
            t.m20 * m01 + t.m21 * m11 + t.m22 * m21 + t.m23 * m31,
            t.m20 * m02 + t.m21 * m12 + t.m22 * m22 + t.m23 * m32,
            t.m20 * m03 + t.m21 * m13 + t.m22 * m23 + t.m23 * m33,
            t.m30 * m00 + t.m31 * m10 + t.m32 * m20 + t.m33 * m30,
            t.m30 * m01 + t.m31 * m11 + t.m32 * m21 + t.m33 * m31,
            t.m30 * m02 + t.m31 * m12 + t.m32 * m22 + t.m33 * m32,
            t.m30 * m03 + t.m31 * m13 + t.m32 * m23 + t.m33 * m33,
        ])
    }

    /// `self = m * self`.
    pub fn multiply_to(&mut self, m: [f32; 16]) -> &mut Self {
        let [m00, m01, m02, m03, m10, m11, m12, m13, m20, m21, m22, m23, m30, m31, m32, m33] = m;
        let t = *self;
        self.set([
            m00 * t.m00 + m01 * t.m10 + m02 * t.m20 + m03 * t.m30,
            m00 * t.m01 + m01 * t.m11 + m02 * t.m21 + m03 * t.m31,
            m00 * t.m02 + m01 * t.m12 + m02 * t.m22 + m03 * t.m32,
            m00 * t.m03 + m01 * t.m13 + m02 * t.m23 + m03 * t.m33,
            m10 * t.m00 + m11 * t.m10 + m12 * t.m20 + m13 * t.m30,
            m10 * t.m01 + m11 * t.m11 + m12 * t.m21 + m13 * t.m31,
            m10 * t.m02 + m11 * t.m12 + m12 * t.m22 + m13 * t.m32,
            m10 * t.m03 + m11 * t.m13 + m12 * t.m23 + m13 * t.m33,
            m20 * t.m00 + m21 * t.m10 + m22 * t.m20 + m23 * t.m30,
            m20 * t.m01 + m21 * t.m11 + m22 * t.m21 + m23 * t.m31,
            m20 * t.m02 + m21 * t.m12 + m22 * t.m22 + m23 * t.m32,
            m20 * t.m03 + m21 * t.m13 + m22 * t.m23 + m23 * t.m33,
            m30 * t.m00 + m31 * t.m10 + m32 * t.m20 + m33 * t.m30,
            m30 * t.m01 + m31 * t.m11 + m32 * t.m21 + m33 * t.m31,
            m30 * t.m02 + m31 * t.m12 + m32 * t.m22 + m33 * t.m32,
            m30 * t.m03 + m31 * t.m13 + m32 * t.m23 + m33 * t.m33,
        ])
    }

    /// `self = m * self` with `m` as the upper-left 3x3; the bottom row is kept, translation is not added.
    pub fn multiply_to_m3(&mut self, m: &MatrixM3f) -> &mut Self {
        let t = *self;
        self.set([
            m.m00 * t.m00 + m.m01 * t.m10 + m.m02 * t.m20,
            m.m00 * t.m01 + m.m01 * t.m11 + m.m02 * t.m21,
            m.m00 * t.m02 + m.m01 * t.m12 + m.m02 * t.m22,
            m.m00 * t.m03 + m.m01 * t.m13 + m.m02 * t.m23,
            m.m10 * t.m00 + m.m11 * t.m10 + m.m12 * t.m20,
            m.m10 * t.m01 + m.m11 * t.m11 + m.m12 * t.m21,
            m.m10 * t.m02 + m.m11 * t.m12 + m.m12 * t.m22,
            m.m10 * t.m03 + m.m11 * t.m13 + m.m12 * t.m23,
            m.m20 * t.m00 + m.m21 * t.m10 + m.m22 * t.m20,
            m.m20 * t.m01 + m.m21 * t.m11 + m.m22 * t.m21,
            m.m20 * t.m02 + m.m21 * t.m12 + m.m22 * t.m22,
            m.m20 * t.m03 + m.m21 * t.m13 + m.m22 * t.m23,
            t.m30,
            t.m31,
            t.m32,
            t.m33,
        ])
    }
}
