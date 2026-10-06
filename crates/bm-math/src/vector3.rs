use bm_java::math::{max_f32, min_f32};

use crate::{MatrixM3f, MatrixM4f};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VectorM3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl VectorM3f {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn set(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        *self = Self { x, y, z };
        self
    }

    pub fn set_i(&mut self, [x, y, z]: [i32; 3]) -> &mut Self {
        self.set(x as f32, y as f32, z as f32)
    }

    pub fn mul(&mut self, a: f32) -> &mut Self {
        self.x *= a;
        self.y *= a;
        self.z *= a;
        self
    }

    pub fn cross(&mut self, v: &VectorM3f) -> &mut Self {
        let Self { x, y, z } = *self;
        self.set(y * v.z - z * v.y, z * v.x - x * v.z, x * v.y - y * v.x)
    }

    pub fn transform_m3(&mut self, t: &MatrixM3f) -> &mut Self {
        let Self { x, y, z } = *self;
        self.set(
            t.m00 * x + t.m01 * y + t.m02 * z,
            t.m10 * x + t.m11 * y + t.m12 * z,
            t.m20 * x + t.m21 * y + t.m22 * z,
        )
    }

    pub fn transform(&mut self, t: &MatrixM4f) -> &mut Self {
        let Self { x, y, z } = *self;
        self.set(
            t.m00 * x + t.m01 * y + t.m02 * z + t.m03,
            t.m10 * x + t.m11 * y + t.m12 * z + t.m13,
            t.m20 * x + t.m21 * y + t.m22 * z + t.m23,
        )
    }

    /// `transform` without the translation column (for directions and normals).
    pub fn rotate_and_scale(&mut self, t: &MatrixM4f) -> &mut Self {
        let Self { x, y, z } = *self;
        self.set(
            t.m00 * x + t.m01 * y + t.m02 * z,
            t.m10 * x + t.m11 * y + t.m12 * z,
            t.m20 * x + t.m21 * y + t.m22 * z,
        )
    }

    pub fn normalize(&mut self) -> &mut Self {
        let length = self.length();
        self.x /= length;
        self.y /= length;
        self.z /= length;
        self
    }

    pub fn absolute(&mut self) -> &mut Self {
        self.x = self.x.abs();
        self.y = self.y.abs();
        self.z = self.z.abs();
        self
    }

    pub fn length(&self) -> f32 {
        self.length_squared().sqrt() as f32
    }

    /// Summed in float, then widened (Java's signature says double, the arithmetic is float).
    pub fn length_squared(&self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z) as f64
    }

    pub fn dot(&self, v: &VectorM3f) -> f32 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }

    pub fn max(&self) -> f32 {
        max_f32(self.x, max_f32(self.y, self.z))
    }

    pub fn min(&self) -> f32 {
        min_f32(self.x, min_f32(self.y, self.z))
    }
}
