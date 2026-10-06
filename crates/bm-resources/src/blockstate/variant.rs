//! `Variant` (one model reference with its rotation) and `VariantSet` (weighted alternatives under a condition).

use bm_math::MatrixM4f;

use super::Condition;
use crate::ResourcePath;

/// BlueMap's `BlockRendererType` registry (`bluemap:` namespace).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RendererType {
    Default,
    Liquid,
    Missing,
}

impl RendererType {
    /// `None` for keys outside the registry: BlueMap warns once per key and renders with [`Default`](Self::Default).
    pub fn from_key(key: &ResourcePath) -> Option<Self> {
        match key.as_str() {
            "bluemap:default" => Some(Self::Default),
            "bluemap:liquid" => Some(Self::Liquid),
            "bluemap:missing" => Some(Self::Missing),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Default => "bluemap:default",
            Self::Liquid => "bluemap:liquid",
            Self::Missing => "bluemap:missing",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    /// Registry key as written (`Key.parse(s, "bluemap")`, case kept); resolve with [`RendererType::from_key`].
    pub renderer: ResourcePath,
    pub model: ResourcePath,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub uvlock: bool,
    pub weight: f64,
    /// Any of `x`, `y`, `z` is non-zero.
    pub transformed: bool,
    /// `T(-.5) · rotateYXZ(-x, -y, -z) · T(.5)`.
    pub transform: MatrixM4f,
}

impl Variant {
    pub const DEFAULT_RENDERER: &str = "bluemap:default";
    pub const MISSING_MODEL: &str = "bluemap:block/missing";

    pub fn new(model: ResourcePath, x: f32, y: f32, z: f32, uvlock: bool, weight: f64) -> Self {
        Self::with_renderer(ResourcePath::key(Self::DEFAULT_RENDERER), model, x, y, z, uvlock, weight)
    }

    pub(super) fn with_renderer(
        renderer: ResourcePath,
        model: ResourcePath,
        x: f32,
        y: f32,
        z: f32,
        uvlock: bool,
        weight: f64,
    ) -> Self {
        let mut transform = MatrixM4f::default();
        transform.translate(-0.5, -0.5, -0.5).rotate_yxz(-x, -y, -z).translate(0.5, 0.5, 0.5);
        Self { renderer, model, x, y, z, uvlock, weight, transformed: x != 0.0 || y != 0.0 || z != 0.0, transform }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantSet {
    pub condition: Condition,
    pub variants: Box<[Variant]>,
    total_weight: f64,
}

impl VariantSet {
    pub fn new(condition: Condition, variants: Box<[Variant]>) -> Self {
        let total_weight = variants.iter().map(|v| v.weight).sum();
        Self { condition, variants, total_weight }
    }

    pub fn total_weight(&self) -> f64 {
        self.total_weight
    }

    /// The variant BlueMap renders at this block position: a deterministic weighted pick. `None` only for an
    /// empty set or degenerate (negative/infinite/NaN) weights.
    pub fn pick(&self, x: i32, y: i32, z: i32) -> Option<&Variant> {
        // with one finite non-negative weight the loop below always takes it on the first step
        if let [only] = &*self.variants
            && only.weight >= 0.0
            && only.weight.is_finite()
        {
            return Some(only);
        }
        let mut selection = f64::from(hash_to_float(x, y, z)) * self.total_weight;
        for variant in &self.variants {
            selection -= variant.weight;
            if selection <= 0.0 {
                return Some(variant);
            }
        }
        None
    }
}

/// `VariantSet.hashToFloat`, with Java's wrapping `long` arithmetic; in `[0, 1)`.
pub fn hash_to_float(x: i32, y: i32, z: i32) -> f32 {
    let hash =
        i64::from(x).wrapping_mul(73438747) ^ i64::from(y).wrapping_mul(9357269) ^ i64::from(z).wrapping_mul(4335792);
    (hash.wrapping_mul(hash.wrapping_add(456149)) & 0x00ff_ffff) as f32 / 16_777_216.0
}
