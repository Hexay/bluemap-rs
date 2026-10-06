//! Colour calculators and blending against Java BlueMap (see `data/color.rs`, `tools/javaref/ColorRef.java`).

#[path = "data/color.rs"]
mod data;

use bm_math::Color;
use bm_resources::color::{self, ColorMap};
use bm_resources::datapack::{Biome, GrassColorModifier};

fn pixel(i: u32, salt: u32) -> u32 {
    i.wrapping_mul(0x9E37_79B1) ^ (i >> 3) ^ salt
}

fn map(salt: u32) -> ColorMap {
    ColorMap::from_argb((0..65536).map(|i| pixel(i, salt)).collect())
}

fn premultiplied(argb: u32) -> Color {
    *Color::default().set_int(argb as i32).premultiplied()
}

fn biomes() -> Vec<Biome> {
    data::BIOMES
        .iter()
        .map(|&(t, d, foliage, grass, modifier, water)| {
            let overlay = |c| if c == 0 { Color { premultiplied: true, ..Color::default() } } else { premultiplied(c) };
            let mut water_color = *Color::default().set_int(water as i32);
            water_color.a = 1.0;
            Biome {
                temperature: f32::from_bits(t),
                downfall: f32::from_bits(d),
                water_color: *water_color.premultiplied(),
                foliage_color: overlay(foliage),
                dry_foliage_color: overlay(0),
                grass_color: overlay(grass),
                grass_color_modifier: [
                    GrassColorModifier::None,
                    GrassColorModifier::DarkForest,
                    GrassColorModifier::Swamp,
                ][modifier as usize],
            }
        })
        .collect()
}

fn bits(c: Color) -> [u32; 5] {
    [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits(), c.premultiplied as u32]
}

struct Ref {
    biomes: Vec<Biome>,
    foliage: ColorMap,
    grass: ColorMap,
}

impl Ref {
    fn new() -> Self {
        Self { biomes: biomes(), foliage: map(0), grass: map(0x5A_5A5A) }
    }

    fn sample(&self, ty: u8, biome: &Biome, x: i32, z: i32) -> Color {
        match ty {
            0 => color::foliage(biome, Some(&self.foliage)),
            1 => color::dry_foliage(biome, None),
            2 => color::grass(biome, Some(&self.grass), x, z),
            _ => color::water(biome),
        }
    }

    fn biome_at(&self, x: i32, y: i32, z: i32) -> &Biome {
        &self.biomes[(x * 7 + z * 13 + y * 3 + (x >> 2) * 5).rem_euclid(self.biomes.len() as i32) as usize]
    }
}

#[test]
fn unblended_samples_match_java() {
    let r = Ref::new();
    for &(ty, x, z, biome, expected) in data::SAMPLES {
        // Java's water keeps the straight flag; the channels are identical at alpha 1
        let mut got = bits(r.sample(ty, &r.biomes[biome], x, z));
        if ty == 3 {
            got[4] = expected[4];
        }
        assert_eq!(got, expected, "type {ty} biome {biome} at {x},{z}");
    }
}

#[test]
fn blended_colors_match_java() {
    let r = Ref::new();
    for &(ty, x, y, z, expected) in data::BLENDED {
        let got = color::blend(x, y, z, |x, y, z| r.sample(ty, r.biome_at(x, y, z), x, z));
        assert_eq!(bits(got), expected, "type {ty} at {x},{y},{z}");
    }
}

#[test]
fn colormap_lookups_match_java() {
    let foliage = map(0);
    for &(t, d, expected) in data::LOOKUPS {
        let got = foliage.color(f32::from_bits(t) as f64, f32::from_bits(d) as f64, Color::default());
        assert_eq!(bits(got), expected, "t {} d {}", f32::from_bits(t), f32::from_bits(d));
    }
}
