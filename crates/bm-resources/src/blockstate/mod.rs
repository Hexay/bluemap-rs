//! Blockstate files (`assets/<ns>/blockstates/<name>.json`): which models a world block state renders with
//! (docs/02-resources.md §2). Parsing and resolution port BlueMap's `resourcepack.blockstate` package,
//! including its non-vanilla fallbacks.

mod condition;
mod parse;
mod variant;

pub use condition::Condition;
pub use parse::ParseError;
pub use variant::{RendererType, Variant, VariantSet, hash_to_float};

use bm_world::BlockState;
use serde_json::Value;

/// One parsed blockstate file. A file may carry both `variants` and `multipart`; both then apply.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockStateDef {
    pub variants: Option<Variants>,
    pub multipart: Option<Multipart>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Variants {
    /// Keyed sets in file order; the default key and malformed keys are not in here.
    pub sets: Vec<VariantSet>,
    /// The `""`/`default`/`normal` set (the last one if repeated).
    pub default: Option<VariantSet>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Multipart {
    pub parts: Vec<VariantSet>,
}

impl BlockStateDef {
    /// Reads a blockstate from [`crate::json::parse`] output. On error BlueMap skips the whole file.
    pub fn from_json(v: &Value) -> Result<Self, ParseError> {
        parse::block_state(v)
    }

    /// The variant sets that apply to `state`: the `variants` match (if any), then every matching multipart part.
    pub fn resolve<'a>(&'a self, state: &'a BlockState) -> impl Iterator<Item = &'a VariantSet> + 'a {
        let variants = self.variants.as_ref().and_then(|v| v.resolve(state));
        let parts = self.multipart.iter().flat_map(move |m| m.resolve(state));
        variants.into_iter().chain(parts)
    }

    /// `BlockState.forEach(state, x, y, z, ..)`: the variants rendered for `state` at this position.
    pub fn variants_at<'a>(
        &'a self,
        state: &'a BlockState,
        x: i32,
        y: i32,
        z: i32,
    ) -> impl Iterator<Item = &'a Variant> + 'a {
        self.resolve(state).filter_map(move |set| set.pick(x, y, z))
    }

    /// Every variant in the file regardless of state (`BlockState.forEach(consumer)` order).
    pub fn all_variants(&self) -> impl Iterator<Item = &Variant> {
        let variants = self.variants.iter().flat_map(|v| v.sets.iter().chain(&v.default));
        let parts = self.multipart.iter().flat_map(|m| &m.parts);
        variants.chain(parts).flat_map(|set| &*set.variants)
    }
}

impl Variants {
    /// First matching set in file order, else the default, else the first set: unlike vanilla, a state matching
    /// nothing still renders `sets[0]`.
    pub fn resolve(&self, state: &BlockState) -> Option<&VariantSet> {
        self.sets.iter().find(|s| s.condition.matches(state)).or(self.default.as_ref()).or(self.sets.first())
    }
}

impl Multipart {
    pub fn resolve<'a>(&'a self, state: &'a BlockState) -> impl Iterator<Item = &'a VariantSet> + 'a {
        self.parts.iter().filter(move |p| p.condition.matches(state))
    }
}

#[cfg(test)]
#[rustfmt::skip]
mod ref_data;
#[cfg(test)]
mod tests;
