use bm_math::{Axis, MatrixM4f, VectorM3f};
use serde_json::Value;

use super::ModelError;
use super::gson::{self, opt_bool, opt_f32, opt_strict_string, opt_vec};

const DEFAULT_ORIGIN: [f32; 3] = [8.0, 8.0, 8.0];

/// An element rotation (`Rotation.java`): `axis` + `angle`, or per-axis `x`/`y`/`z` degrees, around `origin`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rotation {
    pub origin: [f32; 3],
    /// Effective per-axis angles: a non-zero `angle` has replaced them with its single axis.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub axis: Axis,
    pub angle: f32,
    pub rescale: bool,
    matrix: MatrixM4f,
}

impl Default for Rotation {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Rotation {
    pub const ZERO: Self = Self {
        origin: DEFAULT_ORIGIN,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        axis: Axis::Y,
        angle: 0.0,
        rescale: false,
        matrix: MatrixM4f::IDENTITY,
    };

    pub fn from_axis(origin: [f32; 3], axis: Axis, angle: f32, rescale: bool) -> Self {
        Self { origin, axis, angle, rescale, ..Self::ZERO }.init()
    }

    pub fn from_xyz(origin: [f32; 3], [x, y, z]: [f32; 3], rescale: bool) -> Self {
        Self { origin, x, y, z, rescale, ..Self::ZERO }.init()
    }

    pub fn from_value(v: &Value) -> Result<Self, ModelError> {
        let obj = gson::object(v, "rotation")?;
        let axis = match opt_strict_string(obj, "axis")? {
            Some(name) => java_axis(&name)?,
            None => Axis::Y,
        };
        Ok(Self {
            origin: opt_vec(obj, "origin")?.unwrap_or(DEFAULT_ORIGIN),
            x: opt_f32(obj, "x", 0.0)?,
            y: opt_f32(obj, "y", 0.0)?,
            z: opt_f32(obj, "z", 0.0)?,
            axis,
            angle: opt_f32(obj, "angle", 0.0)?,
            rescale: opt_bool(obj, "rescale")?.unwrap_or(false),
            matrix: MatrixM4f::IDENTITY,
        }
        .init())
    }

    /// `T(-origin) · rotateYXZ(x, y, z) · T(origin)`, rescaled so the rotated unit axes span the block again.
    pub fn matrix(&self) -> &MatrixM4f {
        &self.matrix
    }

    fn init(mut self) -> Self {
        if self.angle != 0.0 {
            (self.x, self.y, self.z) = (0.0, 0.0, 0.0);
            match self.axis {
                Axis::X => self.x = self.angle,
                Axis::Y => self.y = self.angle,
                Axis::Z => self.z = self.angle,
            }
        }
        let [ox, oy, oz] = self.origin;
        let (x, y, z) = (self.x, self.y, self.z);
        let mut m = MatrixM4f::IDENTITY;
        if x != 0.0 || y != 0.0 || z != 0.0 {
            m.translate(-ox, -oy, -oz).rotate_yxz(x, y, z);
            if self.rescale {
                let mut v = VectorM3f::default();
                let sx = 1.0 / v.set(1.0, 0.0, 0.0).rotate_and_scale(&m).absolute().max();
                let sy = 1.0 / v.set(0.0, 1.0, 0.0).rotate_and_scale(&m).absolute().max();
                let sz = 1.0 / v.set(0.0, 0.0, 1.0).rotate_and_scale(&m).absolute().max();
                m.identity().translate(-ox, -oy, -oz).scale(sx, sy, sz).rotate_yxz(x, y, z);
            }
            m.translate(ox, oy, oz);
        }
        self.matrix = m;
        self
    }
}

/// `Axis.fromString`: `toUpperCase(Locale.ROOT)`, so ASCII case-insensitive.
fn java_axis(name: &str) -> Result<Axis, ModelError> {
    Ok(name.to_ascii_lowercase().parse()?)
}
