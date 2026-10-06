//! `render-mask` config entries (`common/.../config/mask/**`) and the `CombinedMask` they build.
//! Field defaults mirror the Java config classes; parsing the HOCON into these structs happens elsewhere.

use super::{BlurMask, BoxMask, CombinedMask, EllipseMask, Mask, PolygonMask};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MaskConfigError {
    #[error("No mask-type found for key: {0}!")]
    UnknownType(String),
    #[error(
        "The box-mask configuration results in a degenerate mask.\nMake sure that all \"min-\" values are actually SMALLER than their \"max-\" counterparts."
    )]
    DegenerateBox,
    /// Java reports an inverted y range as a circle-mask problem for ellipses too.
    #[error(
        "The {0}-mask configuration results in a degenerate mask.\nMake sure that the \"min-y\" value is actually SMALLER than the \"max-y\" counterpart."
    )]
    DegenerateY(&'static str),
    #[error(
        "The circle-mask configuration results in a degenerate mask.\nMake sure that the \"radius\" value is greater than 0."
    )]
    CircleRadius,
    #[error(
        "The ellipse-mask configuration results in a degenerate mask.\nMake sure that the radius values are greater than 0."
    )]
    EllipseRadius,
    #[error("The polygon-mask configuration needs at least 3 points for a valid shape.")]
    PolygonPoints,
}

/// One `render-mask` list entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaskConfig {
    pub subtract: bool,
    pub shape: MaskShape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MaskShape {
    Box { min: [i32; 3], max: [i32; 3] },
    Circle { center_x: f64, center_z: f64, radius: f64, min_y: i32, max_y: i32 },
    Ellipse { center_x: f64, center_z: f64, radius_x: f64, radius_z: f64, min_y: i32, max_y: i32 },
    Polygon { min_y: i32, max_y: i32, shape: Vec<[f64; 2]> },
    Blur { size: i32, masks: Vec<MaskConfig> },
}

impl Default for MaskShape {
    /// `type` defaults to `box`, whose bounds default to the whole int range.
    fn default() -> Self {
        Self::Box { min: [i32::MIN; 3], max: [i32::MAX; 3] }
    }
}

impl MaskShape {
    /// The shape for a `type` key with every field at its Java default; keys resolve like `Key.parse(key, "bluemap")`.
    pub fn default_of(type_key: &str) -> Result<Self, MaskConfigError> {
        let value = match type_key.find(':') {
            Some(i) if i > 0 && &type_key[..i] == "bluemap" => &type_key[i + 1..],
            Some(i) if i > 0 => return Err(MaskConfigError::UnknownType(type_key.to_owned())),
            _ => type_key,
        };
        let (min_y, max_y) = (i32::MIN, i32::MAX);
        Ok(match value {
            "box" => Self::Box { min: [i32::MIN; 3], max: [i32::MAX; 3] },
            "circle" => Self::Circle { center_x: 0.0, center_z: 0.0, radius: f64::MAX, min_y, max_y },
            "ellipse" => {
                Self::Ellipse { center_x: 0.0, center_z: 0.0, radius_x: f64::MAX, radius_z: f64::MAX, min_y, max_y }
            }
            "polygon" => Self::Polygon { min_y, max_y, shape: Vec::new() },
            "blur" => Self::Blur { size: 5, masks: Vec::new() },
            _ => return Err(MaskConfigError::UnknownType(type_key.to_owned())),
        })
    }

    /// `MaskConfig.createMask`.
    pub fn create_mask(&self) -> Result<Mask, MaskConfigError> {
        Ok(match self {
            Self::Box { min, max } => {
                if (0..3).any(|i| min[i] > max[i]) {
                    return Err(MaskConfigError::DegenerateBox);
                }
                Mask::Box(BoxMask { min: *min, max: *max })
            }
            &Self::Circle { center_x, center_z, radius, min_y, max_y } => {
                check_y(min_y, max_y, "circle")?;
                if radius <= 0.0 {
                    return Err(MaskConfigError::CircleRadius);
                }
                Mask::Ellipse(EllipseMask::circle([center_x, center_z], radius, min_y, max_y))
            }
            &Self::Ellipse { center_x, center_z, radius_x, radius_z, min_y, max_y } => {
                check_y(min_y, max_y, "circle")?;
                if radius_x <= 0.0 || radius_z <= 0.0 {
                    return Err(MaskConfigError::EllipseRadius);
                }
                Mask::Ellipse(EllipseMask::new([center_x, center_z], radius_x, radius_z, min_y, max_y))
            }
            Self::Polygon { min_y, max_y, shape } => {
                check_y(*min_y, *max_y, "polygon")?;
                if shape.len() < 3 {
                    return Err(MaskConfigError::PolygonPoints);
                }
                Mask::Polygon(PolygonMask::new(shape.as_slice(), *min_y, *max_y))
            }
            Self::Blur { size, masks } => {
                let masks = build_render_mask(masks)?;
                if *size > 0 { Mask::Blur(BlurMask { masks, size: *size }) } else { Mask::Combined(masks) }
            }
        })
    }
}

fn check_y(min_y: i32, max_y: i32, kind: &'static str) -> Result<(), MaskConfigError> {
    if min_y > max_y { Err(MaskConfigError::DegenerateY(kind)) } else { Ok(()) }
}

/// `CombinedMaskSerializer.deserialize`: each entry adds its mask, `subtract` entries with value `false`.
pub fn build_render_mask(configs: &[MaskConfig]) -> Result<CombinedMask, MaskConfigError> {
    let mut combined = CombinedMask::default();
    for config in configs {
        combined.add(config.shape.create_mask()?, !config.subtract);
    }
    Ok(combined)
}
