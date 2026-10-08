//! PNG images as `ImageIO.read` + `BufferedImage.getRGB` see them, and `BufferedImageUtil`'s analysis
//! (`core/.../util/BufferedImageUtil.java`).

use std::borrow::Cow;
use std::io::Cursor;

use bm_java::png::{JavaImage, Model, RawPng};
use bm_math::Color;

use super::Error;

/// Straight-alpha RGBA8, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A `BufferedImage`: the model `ImageIO` re-encodes (`java`) and its `getRGB` pixels (`image`).
#[derive(Clone, Debug)]
pub struct DecodedPng {
    pub java: JavaImage,
    pub image: RgbaImage,
}

/// `ImageIO.read`: any PNG colour type and bit depth.
pub fn decode_png(bytes: &[u8]) -> Result<DecodedPng, Error> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size().ok_or(Error::Decode(png::DecodingError::LimitsExceeded))?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf)?;
    let info = reader.info();
    // png keeps only the low byte of each gray/RGB tRNS sample below 16 bits; RawPng wants the chunk payload
    let trns: Option<Vec<u8>> = match (frame.color_type, frame.bit_depth, info.trns.as_deref()) {
        (png::ColorType::Grayscale | png::ColorType::Rgb, d, Some(t)) if d != png::BitDepth::Sixteen => {
            Some(t.iter().flat_map(|&b| [0, b]).collect())
        }
        (_, _, t) => t.map(<[u8]>::to_vec),
    };
    let raw = RawPng {
        width: frame.width,
        height: frame.height,
        bit_depth: frame.bit_depth as u8,
        color_type: frame.color_type as u8,
        data: &buf[..frame.buffer_size()],
        palette: info.palette.as_deref(),
        trns: trns.as_deref(),
    };
    Ok(DecodedPng::new(JavaImage::read(&raw)?))
}

impl DecodedPng {
    pub fn new(java: JavaImage) -> Self {
        let image = RgbaImage { width: java.width, height: java.height, pixels: java.rgba8() };
        Self { image, java }
    }

    /// `BufferedImageUtil.readPixel` of every pixel: `getRGB`, except 8-bit gray (`TYPE_BYTE_GRAY`, or gray+alpha
    /// as `TYPE_CUSTOM`), whose raster samples upstream reads directly to dodge JDK bug 5051418's linear-gray LUT.
    pub fn read_pixels(&self) -> Cow<'_, RgbaImage> {
        let rgba = |v: u16, a: u16| [v as u8, v as u8, v as u8, a as u8];
        let s = &self.java.samples;
        let pixels: Vec<u8> = match self.java.model {
            Model::Gray { bits: 8 } => s.iter().flat_map(|&v| rgba(v, 255)).collect(),
            Model::GrayAlpha { bits: 8 } => s.as_chunks::<2>().0.iter().flat_map(|&[v, a]| rgba(v, a)).collect(),
            _ => return Cow::Borrowed(&self.image),
        };
        Cow::Owned(RgbaImage { width: self.image.width, height: self.image.height, pixels })
    }

    /// A `TYPE_INT_ARGB` image.
    pub fn from_rgba(image: RgbaImage) -> Self {
        Self { java: JavaImage::int_argb(image.width, image.height, &image.pixels), image }
    }

    /// `BufferedImage.getSubimage`; `None` where it throws `RasterFormatException`.
    pub fn sub_image(&self, x: i32, y: i32, w: i32, h: i32) -> Option<Self> {
        let image = self.image.sub_image(x, y, w, h)?;
        Some(Self { java: self.java.sub_image(x as u32, y as u32, w as u32, h as u32), image })
    }

    /// `ImageIO.write(image, "png", out)`.
    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.java.write_png_into(&mut out);
        out
    }
}

impl RgbaImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; width as usize * height as usize * 4] }
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }

    /// `getRGB`: straight ARGB.
    pub fn argb(&self, x: u32, y: u32) -> i32 {
        let o = self.offset(x, y);
        let p = &self.pixels[o..o + 4];
        i32::from_be_bytes([p[3], p[0], p[1], p[2]])
    }

    pub fn set_argb(&mut self, x: u32, y: u32, argb: i32) {
        let o = self.offset(x, y);
        let [a, r, g, b] = argb.to_be_bytes();
        self.pixels[o..o + 4].copy_from_slice(&[r, g, b, a]);
    }

    /// `readPixel(...).getInt()`: the pixel through a straight `Color` and back.
    pub(crate) fn read_pixel_int(&self, x: u32, y: u32, tmp: &mut Color) -> i32 {
        tmp.set_int_premultiplied(self.argb(x, y), false).get_int()
    }

    /// Any pixel with `0 < alpha < 1`. Upstream returns false early for images without (or with 1-bit)
    /// alpha, which can't have such pixels anyway.
    pub fn half_transparent(&self) -> bool {
        self.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[3] < 255)
    }

    /// Mean of the premultiplied pixels, summed in f32 column by column (x outer, y inner) as upstream does;
    /// the order matters for the float result. Returned premultiplied.
    pub fn average_color(&self) -> Color {
        let mut average = Color::default();
        let mut color = Color::default();
        let mut count = 0;
        for x in 0..self.width {
            for y in 0..self.height {
                color.set_int_premultiplied(self.argb(x, y), false);
                count += 1;
                average.add(color.premultiplied());
            }
        }
        *average.div(count)
    }

    /// `BufferedImage.getSubimage`; `None` where it throws `RasterFormatException`.
    pub fn sub_image(&self, x: i32, y: i32, w: i32, h: i32) -> Option<RgbaImage> {
        let fits = |start: i32, len: i32, max: u32| {
            start >= 0 && len > 0 && start.checked_add(len).is_some_and(|end| end as i64 <= max as i64)
        };
        if !fits(x, w, self.width) || !fits(y, h, self.height) {
            return None;
        }
        let mut out = RgbaImage::new(w as u32, h as u32);
        let row = w as usize * 4;
        for dy in 0..h as u32 {
            let src = self.offset(x as u32, y as u32 + dy);
            let dst = out.offset(0, dy);
            out.pixels[dst..dst + row].copy_from_slice(&self.pixels[src..src + row]);
        }
        Some(out)
    }
}
