//! `render-mask` (`CombinedMaskSerializer` + `config/mask/*MaskConfig.java`): validated while loading, as in Java.

use serde::Deserialize;
use serde::de::{Deserializer, Error};

use crate::de::{fields, from_value};
use crate::key::Key;
use crate::value::Value;

/// One entry of a map's `render-mask` list. Entries apply in order; `subtract` removes instead of adds.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderMask {
    pub shape: MaskShape,
    pub subtract: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaskShape {
    /// Inclusive block bounds `[x, y, z]`; unset bounds are `i32::MIN`/`i32::MAX`.
    Box { min: [i32; 3], max: [i32; 3] },
    /// `circle` (equal radii) or `ellipse`, centre `[x, z]`, radii `[x, z]`.
    Ellipse { center: [f64; 2], radius: [f64; 2], min_y: i32, max_y: i32 },
    /// Points `[x, z]`, at least 3.
    Polygon { points: Vec<[f64; 2]>, min_y: i32, max_y: i32 },
    /// `size <= 0` means the inner masks unblurred.
    Blur { size: i32, masks: Vec<RenderMask> },
}

#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct Base {
    #[serde(rename = "type")]
    kind: String,
    subtract: bool,
}

impl Default for Base {
    fn default() -> Self {
        Self { kind: "bluemap:box".into(), subtract: false }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct BoxMask {
    min_x: i32,
    min_y: i32,
    min_z: i32,
    max_x: i32,
    max_y: i32,
    max_z: i32,
}

impl Default for BoxMask {
    fn default() -> Self {
        let (lo, hi) = (i32::MIN, i32::MAX);
        Self { min_x: lo, min_y: lo, min_z: lo, max_x: hi, max_y: hi, max_z: hi }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct EllipseMask {
    center_x: f64,
    center_z: f64,
    radius: f64,
    radius_x: f64,
    radius_z: f64,
    min_y: i32,
    max_y: i32,
}

impl Default for EllipseMask {
    fn default() -> Self {
        let big = f64::MAX;
        Self {
            center_x: 0.0,
            center_z: 0.0,
            radius: big,
            radius_x: big,
            radius_z: big,
            min_y: i32::MIN,
            max_y: i32::MAX,
        }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct PolygonMask {
    min_y: i32,
    max_y: i32,
    #[serde(deserialize_with = "fields::vec2d_list")]
    shape: Option<Vec<[f64; 2]>>,
}

impl Default for PolygonMask {
    fn default() -> Self {
        Self { min_y: i32::MIN, max_y: i32::MAX, shape: None }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct BlurMask {
    size: i32,
    #[serde(deserialize_with = "deserialize")]
    masks: Vec<RenderMask>,
}

impl Default for BlurMask {
    fn default() -> Self {
        Self { size: 5, masks: Vec::new() }
    }
}

const DEGENERATE_Y: &str = "results in a degenerate mask: \"min-y\" must not be greater than \"max-y\"";

fn mask(v: &Value) -> Result<RenderMask, String> {
    let nested = |e: crate::de::DeError| e.to_string();
    let base: Base = from_value(v).map_err(nested)?;
    let kind = Key::parse_with_default(&base.kind, Key::BLUEMAP);
    let shape = match (kind.namespace(), kind.value()) {
        (Key::BLUEMAP, "box") => {
            let m: BoxMask = from_value(v).map_err(nested)?;
            if m.min_x > m.max_x || m.min_y > m.max_y || m.min_z > m.max_z {
                return Err("the box-mask results in a degenerate mask: all \"min-\" values must be smaller than their \"max-\" counterparts".into());
            }
            MaskShape::Box { min: [m.min_x, m.min_y, m.min_z], max: [m.max_x, m.max_y, m.max_z] }
        }
        (Key::BLUEMAP, kind @ ("circle" | "ellipse")) => {
            let m: EllipseMask = from_value(v).map_err(nested)?;
            let radius = if kind == "circle" { [m.radius, m.radius] } else { [m.radius_x, m.radius_z] };
            if m.min_y > m.max_y {
                return Err(format!("the {kind}-mask {DEGENERATE_Y}"));
            }
            if radius[0] <= 0.0 || radius[1] <= 0.0 {
                return Err(format!("the {kind}-mask results in a degenerate mask: the radius must be greater than 0"));
            }
            MaskShape::Ellipse { center: [m.center_x, m.center_z], radius, min_y: m.min_y, max_y: m.max_y }
        }
        (Key::BLUEMAP, "polygon") => {
            let m: PolygonMask = from_value(v).map_err(nested)?;
            if m.min_y > m.max_y {
                return Err(format!("the polygon-mask {DEGENERATE_Y}"));
            }
            match m.shape {
                Some(points) if points.len() >= 3 => MaskShape::Polygon { points, min_y: m.min_y, max_y: m.max_y },
                _ => return Err("the polygon-mask needs at least 3 points in \"shape\"".into()),
            }
        }
        (Key::BLUEMAP, "blur") => {
            let m: BlurMask = from_value(v).map_err(nested)?;
            MaskShape::Blur { size: m.size, masks: m.masks }
        }
        _ => {
            return Err(format!(
                "no mask-type found for key: {} (expected box, circle, ellipse, polygon or blur)",
                base.kind
            ));
        }
    };
    Ok(RenderMask { shape, subtract: base.subtract })
}

/// A non-list `render-mask` is ignored, like `node.childrenList()` on a non-list node.
pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RenderMask>, D::Error> {
    let Value::List(items) = Value::deserialize(d)? else { return Ok(Vec::new()) };
    items.iter().enumerate().map(|(i, v)| mask(v).map_err(|e| D::Error::custom(format!("[{i}] {e}")))).collect()
}
