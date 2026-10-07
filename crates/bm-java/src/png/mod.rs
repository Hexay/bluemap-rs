//! `javax.imageio` PNG round trip: the `BufferedImage` model `PNGImageReader` builds from a PNG, and the exact
//! bytes `ImageIO.write(image, "png", out)` (`PNGImageWriter`, default params and metadata) produces for it.
//! BlueMap embeds those bytes in `textures.json`, so they must match byte for byte.

mod read;
mod write;
mod zlib;

pub use read::{RawPng, ReadError};

/// The colour model of a `BufferedImage`, as far as `PNGImageWriter` distinguishes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Model {
    /// `IndexColorModel` with `2^bits` entries; `alpha` is `None` without a `tRNS` chunk.
    Indexed { bits: u8, rgb: Vec<[u8; 3]>, alpha: Option<Vec<u8>> },
    /// Bits 1, 2, 4, 8 or 16.
    Gray { bits: u8 },
    /// Bits 8 or 16.
    GrayAlpha { bits: u8 },
    /// Bits 8 or 16.
    Rgb { bits: u8 },
    /// Bits 8 or 16; `TYPE_INT_ARGB` and `TYPE_4BYTE_ABGR` are `Rgba { bits: 8 }`.
    Rgba { bits: u8 },
}

impl Model {
    pub fn bands(&self) -> usize {
        match self {
            Model::Indexed { .. } | Model::Gray { .. } => 1,
            Model::GrayAlpha { .. } => 2,
            Model::Rgb { .. } => 3,
            Model::Rgba { .. } => 4,
        }
    }

    pub fn bits(&self) -> u8 {
        match *self {
            Model::Indexed { bits, .. }
            | Model::Gray { bits }
            | Model::GrayAlpha { bits }
            | Model::Rgb { bits }
            | Model::Rgba { bits } => bits,
        }
    }
}

/// A `BufferedImage`: its model plus the raster's samples (`Raster.getPixels` order: row-major, band-interleaved;
/// palette indices for [`Model::Indexed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaImage {
    pub width: u32,
    pub height: u32,
    pub model: Model,
    pub samples: Vec<u16>,
}

impl JavaImage {
    /// A `TYPE_INT_ARGB` image from straight RGBA8 pixels.
    pub fn int_argb(width: u32, height: u32, rgba: &[u8]) -> Self {
        Self { width, height, model: Model::Rgba { bits: 8 }, samples: rgba.iter().map(|&s| s.into()).collect() }
    }

    /// `getSubimage`; the caller checks bounds.
    pub fn sub_image(&self, x: u32, y: u32, w: u32, h: u32) -> Self {
        let bands = self.model.bands();
        let mut samples = Vec::with_capacity(w as usize * h as usize * bands);
        for row in y..y + h {
            let start = (row as usize * self.width as usize + x as usize) * bands;
            samples.extend_from_slice(&self.samples[start..start + w as usize * bands]);
        }
        Self { width: w, height: h, model: self.model.clone(), samples }
    }

    /// `ImageIO.write(this, "png", out)`.
    pub fn write_png_into(&self, out: &mut Vec<u8>) {
        write::write_png(self, out);
    }
}

#[cfg(test)]
mod tests;
