//! `assets/<ns>/blockColors.json` (`BlockColorsConfig.java`) and the per-block tint it resolves to.

use std::sync::Arc;

use bm_math::Color;
use bm_world::{BiomeId, BlockState};
use rustc_hash::FxHashSet;
use serde_json::Value;

use super::tint::{self, premultiplied_argb};
use super::{ColorMap, ColorMaps, ConfigError, LoadFailure, StateMapping, StateMatcher, namespace_files};
use crate::ResourcePath;
use crate::datapack::BiomeTable;
use crate::json;

const WHITE: u32 = 0xFFFF_FFFF;

/// A parsed config value.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorSource {
    /// `#hex` or a decimal ARGB int; also the white fallback for blank, malformed or unknown `@` values.
    Fixed(Color),
    Foliage,
    DryFoliage,
    Grass,
    Water,
    Redstone,
    /// Any other value: a colormap key such as `minecraft:colormap/foliage`.
    ColorMap(ResourcePath),
}

impl ColorSource {
    pub fn parse(value: &str) -> Self {
        let white = || Self::Fixed(premultiplied_argb(WHITE));
        let fixed = |s: &str| Color::default().parse(s).map(|c| Self::Fixed(*c)).ok();
        if value.trim().is_empty() {
            return white();
        }
        if let Some(calculator) = value.strip_prefix('@') {
            return match ResourcePath::key(calculator).as_str() {
                "minecraft:foliage" => Self::Foliage,
                "minecraft:dry_foliage" => Self::DryFoliage,
                "minecraft:grass" => Self::Grass,
                "minecraft:water" => Self::Water,
                "minecraft:redstone" => Self::Redstone,
                _ => white(),
            };
        }
        if value.starts_with('#') {
            return fixed(value).unwrap_or_else(white);
        }
        fixed(value).unwrap_or_else(|| Self::ColorMap(ResourcePath::key(value)))
    }
}

/// The loaded config entries, first definition of each exact state key wins.
#[derive(Clone, Debug, Default)]
pub struct BlockColorsConfig {
    entries: Vec<(StateMatcher, String)>,
    keys: FxHashSet<StateMatcher>,
}

impl BlockColorsConfig {
    /// Loads one file. Like upstream's streaming reader, entries before an error stay loaded.
    pub fn load_str(&mut self, src: &str) -> Result<(), ConfigError> {
        let (members, syntax_error) = json::parse_entries(src);
        for (key, value) in members {
            let matcher = StateMatcher::parse(&key)?;
            let value = match value {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                _ => return Err(ConfigError::Expected { key, expected: "a string" }),
            };
            if self.keys.insert(matcher.clone()) {
                self.entries.push((matcher, value));
            }
        }
        syntax_error.map_or(Ok(()), |e| Err(e.into()))
    }

    pub fn load_pack(&mut self, pack: &crate::Pack, failures: &mut Vec<LoadFailure>) {
        for (file, src) in namespace_files(pack, "blockColors.json") {
            if let Err(error) = self.load_str(&src) {
                failures.push(LoadFailure { file: format!("{}: {file}", pack.origin), error });
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolves every value against the loaded colormaps (a missing colormap becomes white).
    pub fn bake(&self, colormaps: &ColorMaps) -> BlockColors {
        let mut mappings = StateMapping::default();
        for (matcher, value) in &self.entries {
            let tint = match ColorSource::parse(value) {
                ColorSource::Fixed(mut c) => Tint::Fixed(*c.premultiplied()),
                ColorSource::Foliage => Tint::Foliage,
                ColorSource::DryFoliage => Tint::DryFoliage,
                ColorSource::Grass => Tint::Grass,
                ColorSource::Water => Tint::Water,
                ColorSource::Redstone => Tint::Redstone,
                ColorSource::ColorMap(key) => {
                    colormaps.get(&key).map_or(Tint::Fixed(premultiplied_argb(WHITE)), |m| Tint::ColorMap(m.clone()))
                }
            };
            mappings.push(matcher.clone(), tint);
        }
        let map = |name: &str| colormaps.get(&ResourcePath::key(name)).cloned();
        BlockColors {
            mappings,
            foliage: map("colormap/foliage"),
            dry_foliage: map("colormap/dry_foliage"),
            grass: map("colormap/grass"),
        }
    }
}

/// How a block is tinted.
#[derive(Clone, Debug)]
pub enum Tint {
    /// Premultiplied.
    Fixed(Color),
    /// The block's own biome looked up in a config-named colormap; not blended.
    ColorMap(Arc<ColorMap>),
    Foliage,
    DryFoliage,
    Grass,
    Water,
    Redstone,
}

static DEFAULT_TINT: Tint = Tint::Fixed(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0, premultiplied: true });

/// The baked block colour calculator (`BlockColorsConfig.createBlockColorCalculator`). Immutable and shareable.
#[derive(Clone, Debug)]
pub struct BlockColors {
    mappings: StateMapping<Tint>,
    foliage: Option<Arc<ColorMap>>,
    dry_foliage: Option<Arc<ColorMap>>,
    grass: Option<Arc<ColorMap>>,
}

impl BlockColors {
    /// The first mapping that fits `state`, else white. Worth caching per state id.
    pub fn tint(&self, state: &BlockState) -> &Tint {
        self.mappings.get(state).unwrap_or(&DEFAULT_TINT)
    }

    /// The colour of `state` at block (x, y, z). `biome_at` returns the biome at any block position; the biome
    /// tints sample it at the 75 positions around the block.
    pub fn color(
        &self,
        state: &BlockState,
        (x, y, z): (i32, i32, i32),
        biomes: &BiomeTable,
        biome_at: impl Fn(i32, i32, i32) -> BiomeId,
    ) -> Color {
        let biome = |x, y, z| biomes.get(biome_at(x, y, z));
        match self.tint(state) {
            Tint::Fixed(c) => *c,
            Tint::ColorMap(m) => m.biome_color(biome(x, y, z), premultiplied_argb(WHITE)),
            Tint::Foliage => tint::blend(x, y, z, |x, y, z| tint::foliage(biome(x, y, z), self.foliage.as_deref())),
            Tint::DryFoliage => {
                tint::blend(x, y, z, |x, y, z| tint::dry_foliage(biome(x, y, z), self.dry_foliage.as_deref()))
            }
            Tint::Grass => tint::blend(x, y, z, |x, y, z| tint::grass(biome(x, y, z), self.grass.as_deref(), x, z)),
            Tint::Water => tint::blend(x, y, z, |x, y, z| tint::water(biome(x, y, z))),
            Tint::Redstone => tint::redstone(tint::redstone_power(state)),
        }
    }
}

#[cfg(test)]
#[path = "block_colors_tests.rs"]
mod tests;
