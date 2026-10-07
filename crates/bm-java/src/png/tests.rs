use super::*;

/// (type, payload) of every chunk, CRCs checked.
fn chunks(png: &[u8]) -> Vec<(String, Vec<u8>)> {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let mut out = Vec::new();
    let mut i = 8;
    while i < png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let (kind, data) = (&png[i + 4..i + 8], &png[i + 8..i + 8 + len]);
        let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
        assert_eq!(crc, zlib::crc32(&[kind, data]));
        out.push((String::from_utf8(kind.to_vec()).unwrap(), data.to_vec()));
        i += 12 + len;
    }
    out
}

fn inflate(zlib: &[u8], size: usize) -> Vec<u8> {
    let mut out = vec![0u8; size];
    let mut len = size as libz_sys::uLong;
    // SAFETY: buffers valid for the given lengths
    let rc = unsafe { libz_sys::uncompress(out.as_mut_ptr(), &mut len, zlib.as_ptr(), zlib.len() as _) };
    assert_eq!(rc, libz_sys::Z_OK);
    out.truncate(len as usize);
    out
}

fn write(image: &JavaImage) -> Vec<(String, Vec<u8>)> {
    let mut png = Vec::new();
    image.write_png_into(&mut png);
    chunks(&png)
}

fn kinds(c: &[(String, Vec<u8>)]) -> Vec<&str> {
    c.iter().map(|(k, _)| k.as_str()).collect()
}

fn indexed(bits: u8, rgb: Vec<[u8; 3]>, alpha: Option<Vec<u8>>, w: u32, samples: Vec<u16>) -> JavaImage {
    let height = samples.len() as u32 / w;
    JavaImage { width: w, height, model: Model::Indexed { bits, rgb, alpha }, samples }
}

#[test]
fn int_argb_writes_unfiltered_rgba_at_level_4() {
    let c = write(&JavaImage::int_argb(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]));
    assert_eq!(kinds(&c), ["IHDR", "IDAT", "IEND"]);
    assert_eq!(c[0].1, [0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    assert_eq!(&c[1].1[..2], [0x78, 0x5E]);
    assert_eq!(inflate(&c[1].1, 64), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn translucent_palette_moves_to_front_and_filters_rows() {
    let rgb = vec![[10, 0, 0], [20, 0, 0], [30, 0, 0], [40, 0, 0]];
    let image = indexed(2, rgb, Some(vec![255, 0, 255, 128]), 5, vec![0, 1, 2, 3, 0, 0, 1, 2, 3, 0]);
    let c = write(&image);
    assert_eq!(kinds(&c), ["IHDR", "PLTE", "tRNS", "IDAT", "IEND"]);
    assert_eq!(c[0].1[8..10], [2, 3]);
    assert_eq!(c[1].1, [20, 0, 0, 40, 0, 0, 10, 0, 0, 30, 0, 0]);
    assert_eq!(c[2].1, [0, 128]);
    // remapped indices 2 0 3 1 2 → 0b10_00_11_01, 0b10_000000; Up beats None on the identical second row
    assert_eq!(inflate(&c[3].1, 64), [1, 0b1000_1101, 0b1000_0000u8.wrapping_sub(0b1000_1101), 2, 0, 0]);
}

#[test]
fn gray_ramp_palettes_become_gray() {
    let ramp = vec![[0; 3], [255; 3]];
    let c = write(&indexed(1, ramp, None, 3, vec![1, 0, 1]));
    assert_eq!(kinds(&c), ["IHDR", "IDAT", "IEND"]);
    assert_eq!(c[0].1[8..10], [1, 0]);
    assert_eq!(inflate(&c[1].1, 64), [0, 0b1010_0000]);

    let ramp8: Vec<[u8; 3]> = (0..=255).map(|i| [i; 3]).collect();
    let mut alpha = vec![255; 256];
    alpha[7] = 3;
    let c = write(&indexed(8, ramp8, Some(alpha), 2, vec![7, 9]));
    assert_eq!(c[0].1[8..10], [8, 4]);
    assert_eq!(inflate(&c[1].1, 64), [0, 7, 3, 9, 255]);
}

#[test]
fn reader_pads_palettes_like_java() {
    let raw = |bits, plte: &'static [u8], trns: Option<&'static [u8]>| RawPng {
        width: 1,
        height: 1,
        bit_depth: bits,
        color_type: 3,
        data: &[0],
        palette: Some(plte),
        trns,
    };
    let Model::Indexed { rgb, alpha, .. } = JavaImage::read(&raw(8, &[1, 1, 1, 2, 2, 2, 3, 3, 3], None)).unwrap().model
    else {
        panic!()
    };
    assert_eq!((rgb.len(), rgb[2], rgb[3], rgb[255], alpha), (256, [3; 3], [0; 3], [0; 3], None));
    let four = &[1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4];
    let Model::Indexed { rgb, alpha, .. } = JavaImage::read(&raw(4, four, Some(&[0]))).unwrap().model else { panic!() };
    assert_eq!((rgb.len(), rgb[15]), (16, [4; 3]));
    assert_eq!(alpha.unwrap()[..3], [0, 255, 255]);
}

#[test]
fn reader_turns_gray_trns_into_gray_alpha_comparing_scaled_samples() {
    let raw = RawPng {
        width: 3,
        height: 1,
        bit_depth: 4,
        color_type: 0,
        data: &[0x0F, 0x10],
        palette: None,
        trns: Some(&[0, 15]),
    };
    let image = JavaImage::read(&raw).unwrap();
    assert_eq!(image.model, Model::GrayAlpha { bits: 8 });
    assert_eq!(image.samples, [0, 255, 255, 255, 17, 255]);
}

#[test]
fn sub_image_keeps_model() {
    let image = indexed(8, vec![[0; 3]; 256], None, 3, (0..9).collect());
    let sub = image.sub_image(1, 1, 2, 2);
    assert_eq!((sub.samples, sub.model), (vec![4, 5, 7, 8], image.model));
}
