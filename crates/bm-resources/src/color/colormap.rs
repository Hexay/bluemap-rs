//! `assets/<ns>/textures/colormap/**.png` (`ColorMap.java`): a 256×256 temperature/downfall lookup.

use std::io::Cursor;
use std::sync::Arc;

use bm_math::Color;
use rustc_hash::FxHashMap;

use super::{ConfigError, LoadFailure};
use crate::datapack::Biome;
use crate::{Pack, ResourcePath};

const SIZE: usize = 256;

#[derive(Clone)]
pub struct ColorMap {
    /// ARGB, row-major.
    argb: Box<[u32]>,
}

impl std::fmt::Debug for ColorMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ColorMap")
    }
}

impl ColorMap {
    /// `argb` holds 256×256 pixels, row-major.
    ///
    /// # Panics
    /// When `argb` has a different length.
    pub fn from_argb(argb: Box<[u32]>) -> Self {
        assert_eq!(argb.len(), SIZE * SIZE, "colormap must be 256x256");
        Self { argb }
    }

    /// Decodes a PNG and keeps its top-left 256×256 pixels; smaller images fail, as `BufferedImage.getRGB` does.
    /// Not replicated: Java's colour-space conversion of grey and 16-bit images (8-bit RGB(A) is exact).
    pub fn decode(png_bytes: &[u8]) -> Result<Self, ConfigError> {
        let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info()?;
        let (w, h) = (reader.info().width, reader.info().height);
        if w < SIZE as u32 || h < SIZE as u32 {
            return Err(ConfigError::ColormapTooSmall(w, h));
        }
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf)?;
        let channels = info.color_type.samples();
        let mut argb = vec![0u32; SIZE * SIZE].into_boxed_slice();
        for (y, row) in buf.chunks(info.line_size).take(SIZE).enumerate() {
            for (x, px) in row.chunks(channels).take(SIZE).enumerate() {
                let (r, g, b) = if channels >= 3 { (px[0], px[1], px[2]) } else { (px[0], px[0], px[0]) };
                let a = if channels % 2 == 0 { px[channels - 1] } else { 0xFF };
                argb[y * SIZE + x] = u32::from_be_bytes([a, r, g, b]);
            }
        }
        Ok(Self { argb })
    }

    /// `ColorMap.getColor`: clamps both inputs to `0..=1`, scales downfall by temperature, and returns the opaque
    /// pixel premultiplied.
    pub fn color(&self, temperature: f64, downfall: f64, default: Color) -> Color {
        let temperature = clamp(temperature);
        let downfall = clamp(downfall) * temperature;
        let x = ((1.0 - temperature) * 255.0) as i32;
        let y = ((1.0 - downfall) * 255.0) as i32;
        let index = (y << 8 | x) as usize;
        match self.argb.get(index) {
            Some(&c) => *Color::default().set_int_premultiplied((c | 0xFF00_0000) as i32, true),
            None => default,
        }
    }

    pub fn biome_color(&self, biome: &Biome, default: Color) -> Color {
        self.color(biome.temperature as f64, biome.downfall as f64, default)
    }
}

/// flow-math `GenericMath.clamp`: NaN passes through (and then truncates to pixel 0).
fn clamp(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Every pack's colormaps keyed like `minecraft:colormap/foliage`, first pack wins.
#[derive(Clone, Debug, Default)]
pub struct ColorMaps {
    maps: FxHashMap<ResourcePath, Arc<ColorMap>>,
}

impl ColorMaps {
    pub fn load_pack(&mut self, pack: &Pack, failures: &mut Vec<LoadFailure>) {
        for ns in pack.list("assets") {
            for file in pack.walk(&format!("assets/{ns}/textures/colormap")) {
                let Some(key) = file.strip_suffix(".png").and(ResourcePath::from_file(&file, 1, 3)) else { continue };
                if self.maps.contains_key(&key) {
                    continue;
                }
                let Some(bytes) = pack.read(&file) else { continue };
                match ColorMap::decode(&bytes) {
                    Ok(map) => {
                        self.maps.insert(key, Arc::new(map));
                    }
                    Err(error) => failures.push(LoadFailure { file: format!("{}: {file}", pack.origin), error }),
                }
            }
        }
    }

    pub fn insert(&mut self, key: ResourcePath, map: ColorMap) {
        self.maps.insert(key, Arc::new(map));
    }

    pub fn get(&self, key: &ResourcePath) -> Option<&Arc<ColorMap>> {
        self.maps.get(key)
    }

    pub fn len(&self) -> usize {
        self.maps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient() -> ColorMap {
        ColorMap::from_argb((0..SIZE * SIZE).map(|i| (i as u32) << 4 | 0x0100_0000).collect())
    }

    fn encode(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn lookup_clamps_and_scales_downfall() {
        let map = gradient();
        let px = |t: f64, d: f64| map.color(t, d, Color::default());
        let at =
            |x: u32, y: u32| *Color::default().set_int_premultiplied(((y << 8 | x) << 4 | 0xFF00_0000) as i32, true);
        assert_eq!(px(1.0, 1.0), at(0, 0));
        assert_eq!(px(0.0, 1.0), at(255, 255));
        assert_eq!(px(2.0, -1.0), at(0, 255));
        assert_eq!(px(0.5, 0.5), at(127, 191));
        assert_eq!(px(f64::NAN, 0.5), at(0, 0));
        let c = map.color(0.8, 0.4, Color::default());
        assert!(c.premultiplied && c.a == 1.0);
    }

    #[test]
    fn decodes_rgb_and_rejects_small_images() {
        let mut rgb = vec![0u8; 300 * 260 * 3];
        rgb[(259 * 300 + 299) * 3] = 1;
        rgb[(255 * 300 + 255) * 3..][..3].copy_from_slice(&[0x12, 0x34, 0x56]);
        let map = ColorMap::decode(&encode(300, 260, png::ColorType::Rgb, &rgb)).unwrap();
        assert_eq!(map.argb[SIZE * SIZE - 1], 0xFF12_3456);
        assert_eq!(map.argb[0], 0xFF00_0000);
        let small = encode(16, 16, png::ColorType::Grayscale, &[7; 256]);
        assert!(matches!(ColorMap::decode(&small), Err(ConfigError::ColormapTooSmall(16, 16))));
    }
}
