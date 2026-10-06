//! The colour calculators of `BlockColorCalculatorType` as pure functions of one sample position's biome. The
//! biome types (`foliage`, `dry_foliage`, `grass`, `water`) are then averaged with [`blend`].

use std::sync::LazyLock;

use bm_java::JavaRandom;
use bm_math::{Color, SimplexNoise};
use bm_world::BlockState;

use super::ColorMap;
use crate::datapack::{Biome, GrassColorModifier};

/// Fallbacks when the colormap is missing, ARGB.
pub const FOLIAGE_DEFAULT: u32 = 0xFF48_B518;
pub const DRY_FOLIAGE_DEFAULT: u32 = 0xFF8F_5F33;
pub const GRASS_DEFAULT: u32 = 0xFF52_952F;

static SWAMP_GRASS_NOISE: LazyLock<SimplexNoise> = LazyLock::new(|| SimplexNoise::new(&mut JavaRandom::new(2345)));

pub(crate) fn premultiplied_argb(argb: u32) -> Color {
    *Color::default().set_int_premultiplied(argb as i32, true)
}

fn colormap_color(map: Option<&ColorMap>, default: u32, biome: &Biome) -> Color {
    let default = premultiplied_argb(default);
    map.map_or(default, |m| m.biome_color(biome, default))
}

pub fn foliage(biome: &Biome, colormap: Option<&ColorMap>) -> Color {
    *colormap_color(colormap, FOLIAGE_DEFAULT, biome).overlay(&biome.foliage_color)
}

/// Upstream bug, kept because it shows: the dry colormap is overlaid with the biome's *foliage* colour.
pub fn dry_foliage(biome: &Biome, colormap: Option<&ColorMap>) -> Color {
    *colormap_color(colormap, DRY_FOLIAGE_DEFAULT, biome).overlay(&biome.foliage_color)
}

pub fn grass(biome: &Biome, colormap: Option<&ColorMap>, x: i32, z: i32) -> Color {
    let mut c = colormap_color(colormap, GRASS_DEFAULT, biome);
    c.overlay(&biome.grass_color);
    grass_modifier(biome.grass_color_modifier, &mut c, x, z);
    c
}

/// `GrassColorModifier.apply` at block (x, z).
pub fn grass_modifier(modifier: GrassColorModifier, color: &mut Color, x: i32, z: i32) {
    match modifier {
        GrassColorModifier::None => {}
        GrassColorModifier::DarkForest => {
            let c = ((color.get_int() & 0xFE_FEFE) + 0x28_340A) >> 1;
            color.set_int_premultiplied(c | 0xFF00_0000u32 as i32, true);
        }
        GrassColorModifier::Swamp => {
            let f = SWAMP_GRASS_NOISE.get_value(x as f64 * 0.0225, z as f64 * 0.0225);
            let c: u32 = if f < -0.1 { 0xFF4C_763C } else { 0xFF6A_7039 };
            color.set_int_premultiplied(c as i32, true);
        }
    }
}

pub fn water(biome: &Biome) -> Color {
    biome.water_color
}

/// `BlockState.getRedstonePower`: the `power` property clamped to 0..=15, 0 when absent, 15 when unparseable.
pub fn redstone_power(state: &BlockState) -> i32 {
    state.property("power").map_or(0, |p| p.parse::<i32>().map_or(15, |v| v.clamp(0, 15)))
}

pub fn redstone(power: i32) -> Color {
    Color { r: (power as f32 + 5.0) / 20.0, g: 0.0, b: 0.0, a: 1.0, premultiplied: true }
}

/// `BlendedBlockColorCalculator` with its default ±2 horizontal, ±1 vertical box: the premultiplied sum of
/// `sample(x, y, z)` over the 75 neighbours, visited y-outer, then x, then z, then flattened.
pub fn blend(x: i32, y: i32, z: i32, mut sample: impl FnMut(i32, i32, i32) -> Color) -> Color {
    let mut target = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0, premultiplied: true };
    for dy in -1..=1 {
        for dx in -2..=2 {
            for dz in -2..=2 {
                target.add(&sample(x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz)));
            }
        }
    }
    *target.flatten()
}

#[cfg(test)]
mod tests {
    use bm_world::BlockStates;

    use super::*;

    #[test]
    fn defaults_without_colormaps() {
        let b = Biome::DEFAULT;
        assert_eq!(foliage(&b, None), premultiplied_argb(FOLIAGE_DEFAULT));
        assert_eq!(grass(&b, None, 0, 0), premultiplied_argb(GRASS_DEFAULT));
        assert_eq!(water(&b), premultiplied_argb(0xFF3F_76E4));
    }

    #[test]
    fn dry_foliage_uses_the_foliage_overlay() {
        let b = Biome {
            foliage_color: premultiplied_argb(0xFF6A_7039),
            dry_foliage_color: premultiplied_argb(0xFF7B_5334),
            ..Biome::DEFAULT
        };
        assert_eq!(dry_foliage(&b, None), premultiplied_argb(0xFF6A_7039));
    }

    #[test]
    fn modifiers() {
        let mut c = premultiplied_argb(0xFF52_952F);
        let expected = ((c.get_int() as u32 & 0xFE_FEFE) + 0x28_340A) >> 1 | 0xFF00_0000;
        grass_modifier(GrassColorModifier::DarkForest, &mut c, 0, 0);
        assert_eq!(c, premultiplied_argb(expected));
        let swamp = |x, z| {
            let mut c = Color::default();
            grass_modifier(GrassColorModifier::Swamp, &mut c, x, z);
            c
        };
        let colors: std::collections::HashSet<i32> = (0..64).map(|i| swamp(i * 13, i * 7).get_int()).collect();
        assert_eq!(colors.len(), 2);
    }

    #[test]
    fn redstone_power_from_state() {
        let states = BlockStates::default();
        let p = |s: &str| redstone_power(&states.get(states.intern_str(s).unwrap()));
        assert_eq!(
            (p("redstone_wire[power=7]"), p("redstone_wire"), p("x[power=99]"), p("x[power=-2]"), p("x[power=a]")),
            (7, 0, 15, 0, 15)
        );
        assert_eq!(redstone(15).r, 1.0);
        assert_eq!(redstone(0).r, 0.25);
    }

    #[test]
    fn blend_of_uniform_samples_is_that_sample() {
        let c = premultiplied_argb(0xFF20_4060);
        let mut visits = Vec::new();
        let out = blend(10, 64, -3, |x, y, z| {
            visits.push((x, y, z));
            c
        });
        assert_eq!(visits.len(), 75);
        assert_eq!((visits[0], visits[1], visits[5], visits[25]), ((8, 63, -5), (8, 63, -4), (9, 63, -5), (8, 64, -5)));
        assert!((out.r - c.r).abs() < 1e-6 && out.a == 1.0);
    }
}
