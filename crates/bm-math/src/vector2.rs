use bm_java::math::floor_div;
use bm_java::trig;

use crate::MatrixM3f;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VectorM2f {
    pub x: f32,
    pub y: f32,
}

impl VectorM2f {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn set(&mut self, x: f32, y: f32) -> &mut Self {
        *self = Self { x, y };
        self
    }

    pub fn translate(&mut self, x: f32, y: f32) -> &mut Self {
        self.x += x;
        self.y += y;
        self
    }

    /// Rotates by the angle whose cosine is `sx` and sine is `sy` (expected normalised).
    pub fn rotate(&mut self, sx: f32, sy: f32) -> &mut Self {
        let Self { x, y } = *self;
        self.set(x * sx - y * sy, y * sx + x * sy)
    }

    pub fn transform(&mut self, t: &MatrixM3f) -> &mut Self {
        let Self { x, y } = *self;
        self.set(t.m00 * x + t.m01 * y + t.m02, t.m10 * x + t.m11 * y + t.m12)
    }

    pub fn normalize(&mut self) -> &mut Self {
        let length = self.length();
        self.x /= length;
        self.y /= length;
        self
    }

    pub fn length(&self) -> f32 {
        (self.length_squared() as f64).sqrt() as f32
    }

    pub fn length_squared(&self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    /// Unsigned angle in radians, via flow-math's `acos` with a double denominator.
    pub fn angle_to(&self, x: f32, y: f32) -> f32 {
        let num = self.x * x + self.y * y;
        let den = self.length() as f64 * ((x * x + y * y) as f64).sqrt();
        trig::acos(num as f64 / den) as f32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VectorM2i {
    pub x: i32,
    pub y: i32,
}

impl VectorM2i {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub fn set(&mut self, x: i32, y: i32) -> &mut Self {
        *self = Self { x, y };
        self
    }

    /// Integer-truncated; panics on a zero vector like Java's `ArithmeticException`.
    pub fn normalize(&mut self) -> &mut Self {
        let length = self.length();
        self.x = self.x.wrapping_div(length);
        self.y = self.y.wrapping_div(length);
        self
    }

    pub fn add(&mut self, x: i32, y: i32) -> &mut Self {
        self.x = self.x.wrapping_add(x);
        self.y = self.y.wrapping_add(y);
        self
    }

    pub fn div(&mut self, x: i32, y: i32) -> &mut Self {
        self.x = self.x.wrapping_div(x);
        self.y = self.y.wrapping_div(y);
        self
    }

    pub fn floor_div(&mut self, x: i32, y: i32) -> &mut Self {
        self.x = floor_div(self.x, x);
        self.y = floor_div(self.y, y);
        self
    }

    pub fn length(&self) -> i32 {
        (self.length_squared() as f64).sqrt() as i32
    }

    pub fn length_squared(&self) -> i32 {
        self.x.wrapping_mul(self.x).wrapping_add(self.y.wrapping_mul(self.y))
    }

    /// Java `hashCode`, for anything that must iterate a `HashMap` in BlueMap's order.
    pub fn java_hash(&self) -> i32 {
        self.x ^ self.y.wrapping_add(34985735)
    }
}
