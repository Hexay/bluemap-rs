//! `Source.bake` for the generating sources: `UnstitchSource` and `PalettedPermutationsSource`.

use std::collections::HashMap;

use bm_math::Color;

use super::atlas::Source;
use super::image::{DecodedPng, RgbaImage};
use super::load::TexturePool;
use super::{AnimationMeta, Texture};
use crate::key::ResourcePath;

type Images = HashMap<ResourcePath, DecodedPng>;

pub(super) fn bake(source: &Source, pool: &mut TexturePool, images: &mut Images, used: &dyn Fn(&ResourcePath) -> bool) {
    match source {
        Source::Unstitch { resource: Some(resource), divisor_x, divisor_y, regions: Some(regions) } => {
            let Some((decoded, animation)) = input(resource, pool, images) else { return };
            let image = &decoded.image;
            let dx = if *divisor_x <= 0.0 { image.width as f64 } else { *divisor_x };
            let dy = if *divisor_y <= 0.0 { image.height as f64 } else { *divisor_y };
            let (fx, fy) = (image.width as f64 / dx, image.height as f64 / dy);
            for region in regions.iter().flatten() {
                let Some(sprite) = &region.sprite else { continue };
                if pool.contains_key(sprite) || !used(sprite) {
                    continue;
                }
                let (x, y) = ((region.x * fx) as i32, (region.y * fy) as i32);
                let (w, h) = ((region.width * fx) as i32, (region.height * fy) as i32);
                // getSubimage keeps the source's colour model
                if let Some(sub) = decoded.sub_image(x, y, w, h) {
                    put(sprite.clone(), sub, animation.clone(), pool, images);
                }
            }
        }
        Source::PalettedPermutations {
            textures: Some(textures),
            separator,
            palette_key: Some(palette_key),
            permutations: Some(permutations),
        } => {
            let Some((key_palette, _)) = input(palette_key, pool, images) else { return };
            let mut palettes = Vec::new();
            for (suffix, value_key) in permutations {
                if !pool.contains_key(value_key) {
                    continue;
                }
                let Some((values, _)) = input(value_key, pool, images) else { return };
                if let Some(palette) = PaletteMap::new(&key_palette.image, &values.image) {
                    palettes.push((suffix, palette));
                }
            }
            let separator = Source::java_str(separator);
            for resource in textures {
                if !pool.contains_key(resource) {
                    continue;
                }
                let Some((image, animation)) = input(resource, pool, images) else { return };
                for (suffix, palette) in &palettes {
                    let sprite =
                        ResourcePath::key(&format!("{}:{}{separator}{suffix}", resource.namespace(), resource.path()));
                    if pool.contains_key(&sprite) || !used(&sprite) {
                        continue;
                    }
                    let recoloured = DecodedPng::from_rgba(palette.apply_to(&image.image));
                    put(sprite, recoloured, animation.clone(), pool, images);
                }
            }
        }
        _ => {}
    }
}

/// A pool texture's image and animation; `None` when absent or its image can't be decoded.
fn input(key: &ResourcePath, pool: &TexturePool, images: &Images) -> Option<(DecodedPng, Option<AnimationMeta>)> {
    let texture = pool.get(key)?;
    let image = match images.get(key) {
        Some(image) => image.clone(),
        None => texture.decode().ok()?,
    };
    Some((image, texture.animation.clone()))
}

fn put(
    sprite: ResourcePath,
    image: DecodedPng,
    animation: Option<AnimationMeta>,
    pool: &mut TexturePool,
    images: &mut Images,
) {
    pool.insert(sprite.clone(), Texture::from_image(sprite.clone(), &image, animation));
    images.insert(sprite, image);
}

struct PaletteMap(HashMap<i32, i32>);

const OPAQUE: i32 = 0xFF00_0000u32 as i32;

impl PaletteMap {
    /// `None` when `values` is smaller than `keys` (upstream's `ArrayIndexOutOfBoundsException`).
    fn new(keys: &RgbaImage, values: &RgbaImage) -> Option<Self> {
        if values.width < keys.width || values.height < keys.height {
            return None;
        }
        let mut map = HashMap::new();
        let mut tmp = Color::default();
        for x in 0..keys.width {
            for y in 0..keys.height {
                let key = keys.read_pixel_int(x, y, &mut tmp);
                map.insert(key | OPAQUE, values.read_pixel_int(x, y, &mut tmp));
            }
        }
        Some(Self(map))
    }

    fn apply(&self, color: i32) -> i32 {
        let color = color | OPAQUE;
        self.0.get(&color).copied().unwrap_or(color)
    }

    /// Recolours `image`, multiplying the source and palette alphas.
    fn apply_to(&self, image: &RgbaImage) -> RgbaImage {
        let mut out = RgbaImage::new(image.width, image.height);
        let mut tmp = Color::default();
        for x in 0..image.width {
            for y in 0..image.height {
                let color = image.read_pixel_int(x, y, &mut tmp);
                let mut alpha = ((color >> 24) & 0xFF) as f32 / 255.0;
                let mapped = self.apply(color);
                alpha *= ((mapped >> 24) & 0xFF) as f32 / 255.0;
                out.set_argb(x, y, ((((alpha * 255.0) as i32) & 0xFF) << 24) | (mapped & 0xFF_FFFF));
            }
        }
        out
    }
}
