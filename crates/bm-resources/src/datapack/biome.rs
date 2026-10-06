//! `data/<ns>/worldgen/biome/*.json` as BlueMap reads it (`DatapackBiome.Data`, `Biome.DEFAULT`).

use bm_math::Color;
use serde_json::Value;

use super::{DataError, gson};
use crate::ResourcePath;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GrassColorModifier {
    #[default]
    None,
    DarkForest,
    Swamp,
}

impl GrassColorModifier {
    /// Registry lookup with the `minecraft` namespace by default; unknown keys fall back to `None`.
    pub fn from_key(key: &str) -> Self {
        match ResourcePath::key(key).as_str() {
            "minecraft:dark_forest" => Self::DarkForest,
            "minecraft:swamp" => Self::Swamp,
            _ => Self::None,
        }
    }
}

/// The colour-relevant part of a biome. Colours are premultiplied, ready for `Color::overlay`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biome {
    pub temperature: f32,
    pub downfall: f32,
    pub water_color: Color,
    /// Overlay over the foliage colormap; fully transparent (a no-op) when the biome sets none.
    pub foliage_color: Color,
    /// Parsed but unused by the calculators: upstream's `dry_foliage` overlays `foliage_color`.
    pub dry_foliage_color: Color,
    pub grass_color: Color,
    pub grass_color_modifier: GrassColorModifier,
}

const fn argb(c: u32) -> Color {
    Color {
        r: ((c >> 16) & 0xFF) as f32 / 255.0,
        g: ((c >> 8) & 0xFF) as f32 / 255.0,
        b: (c & 0xFF) as f32 / 255.0,
        a: ((c >> 24) & 0xFF) as f32 / 255.0,
        premultiplied: true,
    }
}

const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0, premultiplied: true };

impl Biome {
    /// `Biome.DEFAULT` (`bluemap:default`): unknown biomes and the field defaults of datapack biomes.
    pub const DEFAULT: Self = Self {
        temperature: 0.5,
        downfall: 0.5,
        water_color: argb(4159204 | 0xFF00_0000),
        foliage_color: TRANSPARENT,
        dry_foliage_color: TRANSPARENT,
        grass_color: TRANSPARENT,
        grass_color_modifier: GrassColorModifier::None,
    };

    /// Reads a biome file. Missing or `null` scalars keep their defaults; unknown fields are ignored.
    pub fn from_json(v: &Value) -> Result<Self, DataError> {
        let Value::Object(data) = v else {
            return Err(DataError::Invalid { field: "biome".into(), expected: "an object" });
        };
        let mut biome = Self::DEFAULT;
        for (name, x) in data {
            match (name.as_str(), x) {
                ("temperature" | "downfall", Value::Null) => {}
                ("temperature", x) => biome.temperature = gson::double(x, name)? as f32,
                ("downfall", x) => biome.downfall = gson::double(x, name)? as f32,
                ("effects", Value::Object(effects)) => biome.read_effects(effects)?,
                ("effects", _) => return Err(DataError::Invalid { field: name.clone(), expected: "an object" }),
                _ => {}
            }
        }
        Ok(biome)
    }

    fn read_effects(&mut self, effects: &serde_json::Map<String, Value>) -> Result<(), DataError> {
        for (name, x) in effects {
            match name.as_str() {
                "water_color" => {
                    self.water_color = gson::color(x, name)?;
                    self.water_color.a = 1.0;
                    self.water_color.premultiplied();
                }
                "foliage_color" => self.foliage_color = premultiplied(gson::color(x, name)?),
                "dry_foliage_color" => self.dry_foliage_color = premultiplied(gson::color(x, name)?),
                "grass_color" => self.grass_color = premultiplied(gson::color(x, name)?),
                "grass_color_modifier" => {
                    self.grass_color_modifier = GrassColorModifier::from_key(&gson::string(x, name)?)
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Upstream keeps overlays straight and throws in `overlay` when one is translucent; premultiplying instead is
/// identical for opaque colours and renders translucent ones.
fn premultiplied(mut c: Color) -> Color {
    c.premultiplied();
    c
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn defaults_and_effects() {
        assert_eq!(Biome::from_json(&json!({})).unwrap(), Biome::DEFAULT);
        let b = Biome::from_json(&json!({
            "temperature": 0.8, "downfall": null, "has_precipitation": true,
            "effects": {"water_color": "#617b6480", "foliage_color": 0x6a7039, "grass_color_modifier": "swamp"}
        }))
        .unwrap();
        assert_eq!((b.temperature, b.downfall), (0.8, 0.5));
        assert_eq!(
            b.water_color,
            Color {
                r: 0x61 as f32 / 255.0,
                g: 0x7b as f32 / 255.0,
                b: 0x64 as f32 / 255.0,
                a: 1.0,
                premultiplied: true
            }
        );
        assert_eq!(b.foliage_color, argb(0xFF6A_7039));
        assert_eq!(b.grass_color, TRANSPARENT);
        assert_eq!(b.grass_color_modifier, GrassColorModifier::Swamp);
    }

    #[test]
    fn translucent_overlays_are_premultiplied() {
        let b = Biome::from_json(&json!({"effects": {"grass_color": [1.0, 0.5, 0.0, 0.5]}})).unwrap();
        assert_eq!(b.grass_color, Color { r: 0.5, g: 0.25, b: 0.0, a: 0.5, premultiplied: true });
    }

    #[test]
    fn modifier_registry() {
        assert_eq!(GrassColorModifier::from_key("minecraft:dark_forest"), GrassColorModifier::DarkForest);
        assert_eq!(GrassColorModifier::from_key("Swamp"), GrassColorModifier::None);
        assert_eq!(GrassColorModifier::from_key("mod:swamp"), GrassColorModifier::None);
        let b = Biome::from_json(&json!({"effects": {"grass_color_modifier": 3}})).unwrap();
        assert_eq!(b.grass_color_modifier, GrassColorModifier::None);
    }

    #[test]
    fn failures_drop_the_biome() {
        for bad in [
            json!([]),
            json!({"effects": null}),
            json!({"effects": {"foliage_color": null}}),
            json!({"effects": {"grass_color_modifier": null}}),
            json!({"temperature": "warm"}),
        ] {
            assert!(Biome::from_json(&bad).is_err(), "{bad}");
        }
    }
}
