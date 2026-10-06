//! Lowres tiles (`LowresTile.java`): a PNG twice as tall as wide. Top half: straight ARGB colour per column; bottom
//! half: `height & 0xFFFF | blockLight << 16 | 0xFF000000`. One extra row and column duplicate the neighbour tile
//! for seamless edges. Pixels must match BlueMap's; the PNG bytes may differ.

use std::io::Cursor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowresTile {
    /// Tile size + 1.
    width: usize,
    depth: usize,
    /// ARGB, `width × depth × 2`.
    pixels: Vec<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum LowresError {
    #[error("lowres PNG: {0}")]
    Decode(#[from] png::DecodingError),
    #[error("lowres PNG: {0}")]
    Encode(#[from] png::EncodingError),
    #[error("lowres tile is {0}×{1}, expected {2}×{3}")]
    Size(usize, usize, usize, usize),
}

impl LowresTile {
    /// An empty (transparent) tile for a `tile_size` grid.
    pub fn new(tile_size: [usize; 2]) -> Self {
        let (width, depth) = (tile_size[0] + 1, tile_size[1] + 1);
        Self { width, depth, pixels: vec![0; width * depth * 2] }
    }

    pub fn set(&mut self, x: usize, z: usize, argb: u32, height: i32, block_light: u8) {
        self.pixels[z * self.width + x] = argb;
        self.pixels[(self.depth + z) * self.width + x] = (height as u32 & 0xFFFF) | (block_light as u32) << 16 | 0xFF00_0000;
    }

    pub fn color(&self, x: usize, z: usize) -> u32 {
        self.pixels[z * self.width + x]
    }

    /// 16-bit height, sign-extended only above 0x8000 as BlueMap does (0x8000 reads as +32768).
    pub fn height(&self, x: usize, z: usize) -> i32 {
        let h = (self.pixels[(self.depth + z) * self.width + x] & 0xFFFF) as i32;
        if h > 0x8000 { h | !0xFFFF } else { h }
    }

    pub fn block_light(&self, x: usize, z: usize) -> u8 {
        (self.pixels[(self.depth + z) * self.width + x] >> 16) as u8
    }

    /// 8-bit RGBA PNG into `out` (replacing its contents).
    pub fn encode_png(&self, out: &mut Vec<u8>) -> Result<(), LowresError> {
        out.clear();
        let mut encoder = png::Encoder::new(&mut *out, self.width as u32, (self.depth * 2) as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // the default level writes PNGs up to a third bigger than Java's ImageIO
        encoder.set_compression(png::Compression::High);
        let mut writer = encoder.write_header()?;
        let rgba: Vec<u8> = self.pixels.iter().flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]).collect();
        writer.write_image_data(&rgba)?;
        writer.finish()?;
        Ok(())
    }

    /// Reads a tile written by BlueMap or [`LowresTile::encode_png`]; any PNG colour type is expanded to RGBA.
    pub fn decode_png(bytes: &[u8], tile_size: [usize; 2]) -> Result<Self, LowresError> {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf)?;
        let (w, h) = (info.width as usize, info.height as usize);
        let mut tile = Self::new(tile_size);
        if (w, h) != (tile.width, tile.depth * 2) {
            return Err(LowresError::Size(w, h, tile.width, tile.depth * 2));
        }
        let channels = info.color_type.samples();
        for (i, px) in buf[..w * h * channels].chunks_exact(channels).enumerate() {
            let (r, g, b, a) = match px {
                [v] => (*v, *v, *v, 255),
                [v, a] => (*v, *v, *v, *a),
                [r, g, b] => (*r, *g, *b, 255),
                [r, g, b, a] => (*r, *g, *b, *a),
                _ => unreachable!("PNG samples are 1-4"),
            };
            tile.pixels[i] = u32::from_be_bytes([a, r, g, b]);
        }
        Ok(tile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_encoding_and_height_sign() {
        let mut t = LowresTile::new([4, 4]);
        t.set(0, 0, 0x80FF_0000, -64, 7);
        t.set(4, 4, 0xFF00_00FF, 319, 0);
        t.set(1, 1, 0, 0x8000, 15);
        assert_eq!((t.color(0, 0), t.height(0, 0), t.block_light(0, 0)), (0x80FF_0000, -64, 7));
        assert_eq!((t.height(4, 4), t.block_light(4, 4)), (319, 0));
        assert_eq!(t.height(1, 1), 0x8000, "BlueMap only sign-extends above 0x8000");
    }

    #[test]
    fn png_round_trip_keeps_every_pixel() {
        let mut t = LowresTile::new([500, 500]);
        for i in 0..501 {
            t.set(i, (i * 7) % 501, 0x7F00_0000 | (i as u32 * 31), i as i32 - 64, (i % 16) as u8);
        }
        let mut png = Vec::new();
        t.encode_png(&mut png).unwrap();
        assert_eq!(LowresTile::decode_png(&png, [500, 500]).unwrap(), t);
        assert!(matches!(LowresTile::decode_png(&png, [4, 4]), Err(LowresError::Size(501, 1002, 5, 10))));
    }
}
