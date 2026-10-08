//! `BufferedImage.getRGB` for the images `PNGImageReader` builds: palette lookups, the sub-byte gray ramp,
//! `ComponentColorModel`'s linear-gray → sRGB LUTs for 8/16-bit gray, and its 16 → 8 bit scaling.

use super::gray_lut::{GRAY8, GRAY16_STARTS};
use super::{JavaImage, Model};

impl JavaImage {
    /// `getRGB` of every pixel as straight RGBA8, row-major.
    pub fn rgba8(&self) -> Vec<u8> {
        let s = &self.samples;
        match &self.model {
            Model::Indexed { rgb, alpha, .. } => s
                .iter()
                .flat_map(|&i| {
                    let [r, g, b] = rgb[i as usize];
                    [r, g, b, alpha.as_ref().map_or(255, |a| a[i as usize])]
                })
                .collect(),
            &Model::Gray { bits } => s.iter().flat_map(|&v| [gray(v, bits); 3].into_iter().chain([255])).collect(),
            &Model::GrayAlpha { bits } => s
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|&[v, a]| [gray(v, bits); 3].into_iter().chain([to8(a, bits)]))
                .collect(),
            &Model::Rgb { bits } => s
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[r, g, b]| [to8(r, bits), to8(g, bits), to8(b, bits), 255])
                .collect(),
            &Model::Rgba { bits } => s.iter().map(|&v| to8(v, bits)).collect(),
        }
    }
}

fn gray(v: u16, bits: u8) -> u8 {
    match bits {
        // IndexColorModel ramp i * 255 / (2^bits - 1)
        1 | 2 | 4 => (u32::from(v) * 255 / ((1 << bits) - 1)) as u8,
        8 => GRAY8[v as usize],
        _ => (GRAY16_STARTS.partition_point(|&start| start <= v) - 1) as u8,
    }
}

/// `extractComponent(..., 8)` of an sRGB or alpha sample.
fn to8(v: u16, bits: u8) -> u8 {
    if bits == 16 { (f32::from(v) / 65535.0 * 255.0 + 0.5) as u8 } else { v as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_luts_cover_the_range() {
        assert_eq!((gray(0, 8), gray(255, 8), gray(1, 8)), (0, 255, 13));
        assert_eq!((gray(0, 16), gray(9, 16), gray(10, 16), gray(65535, 16)), (0, 0, 1, 255));
        assert_eq!((gray(1, 2), gray(15, 4)), (85, 255));
        assert_eq!((to8(32767, 16), to8(32768, 16), to8(200, 8)), (127, 128, 200));
    }
}
