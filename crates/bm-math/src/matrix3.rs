use crate::quat;

/// Row-major 3x3 float matrix (2D affine transforms of UVs). Transforms pre-multiply: `translate` after `scale`
/// applies the translation last.
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatrixM3f {
    pub m00: f32, pub m01: f32, pub m02: f32,
    pub m10: f32, pub m11: f32, pub m12: f32,
    pub m20: f32, pub m21: f32, pub m22: f32,
}

impl Default for MatrixM3f {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl MatrixM3f {
    pub const IDENTITY: Self = Self::from_array([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub const fn from_array(m: [f32; 9]) -> Self {
        let [m00, m01, m02, m10, m11, m12, m20, m21, m22] = m;
        Self { m00, m01, m02, m10, m11, m12, m20, m21, m22 }
    }

    pub fn to_array(&self) -> [f32; 9] {
        [self.m00, self.m01, self.m02, self.m10, self.m11, self.m12, self.m20, self.m21, self.m22]
    }

    pub fn set(&mut self, m: [f32; 9]) -> &mut Self {
        *self = Self::from_array(m);
        self
    }

    pub fn invert(&mut self) -> &mut Self {
        let det = self.determinant();
        let Self { m00, m01, m02, m10, m11, m12, m20, m21, m22 } = *self;
        self.set([
            (m11 * m22 - m21 * m12) / det,
            -(m01 * m22 - m21 * m02) / det,
            (m01 * m12 - m02 * m11) / det,
            -(m10 * m22 - m20 * m12) / det,
            (m00 * m22 - m20 * m02) / det,
            -(m00 * m12 - m10 * m02) / det,
            (m10 * m21 - m20 * m11) / det,
            -(m00 * m21 - m20 * m01) / det,
            (m00 * m11 - m01 * m10) / det,
        ])
    }

    pub fn identity(&mut self) -> &mut Self {
        self.set(Self::IDENTITY.to_array())
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.multiply_to([x, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, z])
    }

    pub fn translate(&mut self, x: f32, y: f32) -> &mut Self {
        self.multiply_to([1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0, 1.0])
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
        self.multiply_to(quat::rotation_matrix(q))
    }

    /// `self = self * m`.
    pub fn multiply(&mut self, m: [f32; 9]) -> &mut Self {
        let [m00, m01, m02, m10, m11, m12, m20, m21, m22] = m;
        let t = *self;
        self.set([
            t.m00 * m00 + t.m01 * m10 + t.m02 * m20,
            t.m00 * m01 + t.m01 * m11 + t.m02 * m21,
            t.m00 * m02 + t.m01 * m12 + t.m02 * m22,
            t.m10 * m00 + t.m11 * m10 + t.m12 * m20,
            t.m10 * m01 + t.m11 * m11 + t.m12 * m21,
            t.m10 * m02 + t.m11 * m12 + t.m12 * m22,
            t.m20 * m00 + t.m21 * m10 + t.m22 * m20,
            t.m20 * m01 + t.m21 * m11 + t.m22 * m21,
            t.m20 * m02 + t.m21 * m12 + t.m22 * m22,
        ])
    }

    /// `self = m * self`.
    pub fn multiply_to(&mut self, m: [f32; 9]) -> &mut Self {
        let [m00, m01, m02, m10, m11, m12, m20, m21, m22] = m;
        let t = *self;
        self.set([
            m00 * t.m00 + m01 * t.m10 + m02 * t.m20,
            m00 * t.m01 + m01 * t.m11 + m02 * t.m21,
            m00 * t.m02 + m01 * t.m12 + m02 * t.m22,
            m10 * t.m00 + m11 * t.m10 + m12 * t.m20,
            m10 * t.m01 + m11 * t.m11 + m12 * t.m21,
            m10 * t.m02 + m11 * t.m12 + m12 * t.m22,
            m20 * t.m00 + m21 * t.m10 + m22 * t.m20,
            m20 * t.m01 + m21 * t.m11 + m22 * t.m21,
            m20 * t.m02 + m21 * t.m12 + m22 * t.m22,
        ])
    }

    pub fn determinant(&self) -> f32 {
        let Self { m00, m01, m02, m10, m11, m12, m20, m21, m22 } = *self;
        m00 * (m11 * m22 - m12 * m21) - m01 * (m10 * m22 - m12 * m20) + m02 * (m10 * m21 - m11 * m20)
    }
}
