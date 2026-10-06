//! PNG images as `ImageIO.read` + `BufferedImage.getRGB` see them, and `BufferedImageUtil`'s analysis
//! (`core/.../util/BufferedImageUtil.java`).

use std::io::Cursor;

use bm_math::Color;

use super::Error;

/// Straight-alpha RGBA8, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A decoded PNG plus whether it carries colour-management chunks a browser would apply.
#[derive(Clone, Debug)]
pub struct DecodedPng {
    pub image: RgbaImage,
    /// `gAMA`, `cHRM`, `iCCP` or `sRGB` present: Java's re-encode drops them, so the original bytes would
    /// render differently in the webapp.
    pub color_managed: bool,
}

/// Decodes any PNG colour type and bit depth to RGBA8 (palette, gray and `tRNS` expanded).
pub fn decode_png(bytes: &[u8]) -> Result<DecodedPng, Error> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size().ok_or(Error::Decode(png::DecodingError::LimitsExceeded))?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf)?;
    buf.truncate(frame.buffer_size());
    let info = reader.info();
    let color_managed =
        info.gama_chunk.is_some() || info.chrm_chunk.is_some() || info.icc_profile.is_some() || info.srgb.is_some();
    let sixteen = frame.bit_depth == png::BitDepth::Sixteen;
    let samples: Vec<u8> = if sixteen {
        buf.as_chunks::<2>().0.iter().map(|s| sixteen_to_eight(u16::from_be_bytes(*s))).collect()
    } else {
        buf
    };
    let pixels = match frame.color_type {
        png::ColorType::Rgba => samples,
        png::ColorType::Rgb => samples.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => samples.as_chunks::<2>().0.iter().flat_map(|&[v, a]| [v, v, v, a]).collect(),
        png::ColorType::Grayscale => samples.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(Error::UnexpandedPalette),
    };
    Ok(DecodedPng { image: RgbaImage { width: frame.width, height: frame.height, pixels }, color_managed })
}

/// `ComponentColorModel` scaling a 16-bit sample to 8 bits. 16-bit gray goes through Java's linear-gray
/// colour space upstream (JDK bug 5051418); that conversion is not replicated.
fn sixteen_to_eight(v: u16) -> u8 {
    (v as f32 * (255.0f32 / 65535.0f32) + 0.5) as u8
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

    pub fn encode_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.pixels)?;
        Ok(out)
    }
}
