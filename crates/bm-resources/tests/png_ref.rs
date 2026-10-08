//! Every PNG colour type and bit depth (tRNS variants, all row filters, Adam7) through `decode_png` against
//! `ImageIO.read` + `getRGB` + `ImageIO.write` on JDK 25 (`tools/javaref/PngRef.java`).

use std::path::Path;

use bm_resources::texture::decode_png;

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes().chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}

#[test]
fn decodes_and_reencodes_like_imageio() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let reference = std::fs::read_to_string(data.join("png_ref.txt")).unwrap();
    let mut failures = Vec::new();
    let mut cases = 0;
    for line in reference.lines().filter(|l| !l.starts_with('#')) {
        let [name, argb, written] = line.split(' ').collect::<Vec<_>>()[..] else { panic!("bad line {line}") };
        cases += 1;
        let png = std::fs::read(data.join("png").join(format!("{name}.png"))).unwrap();
        let decoded = match decode_png(&png) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let expected: Vec<i32> = hex(argb).as_chunks::<4>().0.iter().map(|&b| i32::from_be_bytes(b)).collect();
        let image = &decoded.image;
        let actual: Vec<i32> =
            (0..image.height).flat_map(|y| (0..image.width).map(move |x| image.argb(x, y))).collect();
        if let Some(i) = (0..expected.len()).find(|&i| actual.get(i) != Some(&expected[i])) {
            failures.push(format!("{name}: pixel {i} is {:08x?}, Java {:08x}", actual.get(i), expected[i]));
        }
        if decoded.encode_png() != hex(written) {
            failures.push(format!("{name}: ImageIO.write bytes differ"));
        }
    }
    assert!(cases >= 40, "{cases} cases");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
