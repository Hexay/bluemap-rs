//! BlueMap's `core.util.math` package. Every operation keeps Java's evaluation order and its float/double
//! intermediates, so geometry and colours come out bit-identical to BlueMap's.

mod axis;
mod color;
mod matrix3;
mod matrix4;
mod noise;
pub mod quat;
mod vector2;
mod vector3;

pub use axis::{Axis, ParseAxisError};
pub use color::{Color, ParseColorError};
pub use matrix3::MatrixM3f;
pub use matrix4::MatrixM4f;
pub use noise::SimplexNoise;
pub use vector2::{VectorM2f, VectorM2i};
pub use vector3::VectorM3f;
