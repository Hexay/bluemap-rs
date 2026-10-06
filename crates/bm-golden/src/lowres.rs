//! Lowres tile PNGs: top half = column colours, bottom half = height/blocklight (docs/03-rendering.md §4).

use std::io::Cursor;

use anyhow::{Context, Result, bail};

pub struct LowresImage {
    /// Pixels per side of the colour half (tileSize + 1, the extra row/col duplicates the neighbour).
    pub width: usize,
    pub height: usize,
    rgba: Vec<u8>,
}

impl LowresImage {
    pub fn decode(png_bytes: &[u8]) -> Result<Self> {
        let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info()?;
        let mut rgba = vec![0; reader.output_buffer_size().context("png too large")?];
        let info = reader.next_frame(&mut rgba)?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            bail!("expected 8-bit RGBA lowres tile, got {:?}/{:?}", info.color_type, info.bit_depth);
        }
        let (width, full) = (info.width as usize, info.height as usize);
        rgba.truncate(width * full * 4);
        Ok(Self { width, height: full / 2, rgba })
    }

    fn px(&self, x: usize, row: usize) -> [u8; 4] {
        let i = (row * self.width + x) * 4;
        self.rgba[i..i + 4].try_into().unwrap()
    }

    pub fn color(&self, x: usize, z: usize) -> [u8; 4] {
        self.px(x, z)
    }

    /// Top visible block y (0 where the column is empty).
    pub fn block_height(&self, x: usize, z: usize) -> i16 {
        let [_, g, b, _] = self.px(x, self.height + z);
        i16::from_be_bytes([g, b])
    }

    /// Pixel coords (x, z) in the colour half with alpha > 0, i.e. columns that have rendered geometry.
    pub fn visible_pixels(&self) -> Vec<(i32, i32)> {
        (0..self.height)
            .flat_map(|z| (0..self.width).map(move |x| (x, z)))
            .filter(|&(x, z)| self.color(x, z)[3] > 0)
            .map(|(x, z)| (x as i32, z as i32))
            .collect()
    }
}
