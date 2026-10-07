use super::*;
use crate::prbm::TileModel;

fn quad(m: &mut TileModel, material: u32, [x, y, z]: [f32; 3], light: u8) {
    let (a, b, c, d) = ([x, y, z], [x, y, z + 1.], [x + 1., y, z + 1.], [x + 1., y, z]);
    for tri in [[a, b, c], [a, c, d]] {
        tri.iter().for_each(|p| m.position.extend(p));
        m.color.extend([1., 0.5, 0.25]);
        m.sunlight.push(light);
        m.blocklight.push(15 - light);
        m.material.push(material);
    }
    m.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
    m.ao.extend([1., 0.75, 0.5, 1., 0.5, 0.25]);
}

fn sample() -> Vec<u8> {
    let mut m = TileModel::default();
    for i in 0..40 {
        let y = if i % 7 == 0 { 64.05 } else { 64. + (i % 3) as f32 };
        quad(&mut m, i / 10, [(i % 8) as f32, y, (i / 8) as f32 - 0.0], (i % 16) as u8);
    }
    // PRBM vertices 6q+1 are not shared within the quad
    m.position[4] = -0.0;
    m.position[21] = f32::NAN;
    m.uv[3] = 0.3;
    let mut out = Vec::new();
    m.write_prbm(&mut out).unwrap();
    out
}

fn round_trip(codec: &mut CompactCodec, input: &[u8]) -> Vec<u8> {
    let (mut blob, mut out) = (Vec::new(), Vec::new());
    codec.encode_into(input, &mut blob).unwrap();
    codec.decode_into(&blob, &mut out).unwrap();
    assert_eq!(out, input);
    blob
}

#[test]
fn quad_tiles_round_trip_in_quad_mode() {
    let mut codec = CompactCodec::default();
    let prbm = sample();
    let blob = round_trip(&mut codec, &prbm);
    assert_eq!(blob[4], MODE_QUADS);
    assert!(blob.len() < prbm.len() / 4, "{} vs {}", blob.len(), prbm.len());
    let mut empty = Vec::new();
    TileModel::default().write_prbm(&mut empty).unwrap();
    assert_eq!(round_trip(&mut codec, &empty)[4], MODE_QUADS);
}

#[test]
fn anything_else_round_trips_raw() {
    let mut codec = CompactCodec::default();
    let mut prbm = sample();
    // break the quad pattern (vertex 3 ≠ vertex 0) and a padding byte
    prbm[8 + 12 + 3 * 12] ^= 1;
    assert_eq!(round_trip(&mut codec, &prbm)[4], MODE_RAW);
    let mut padded = sample();
    padded[18] = 7;
    assert_eq!(round_trip(&mut codec, &padded)[4], MODE_RAW);
    assert_eq!(round_trip(&mut codec, b"not a tile")[4], MODE_RAW);
    assert_eq!(round_trip(&mut codec, b"")[4], MODE_RAW);
}

#[test]
fn per_quad_deviations_become_exceptions() {
    let mut codec = CompactCodec::default();
    let mut prbm = sample();
    let v = view::parse(&prbm).unwrap();
    let offset = |a: usize| v.attrs[a].as_ptr() as usize - prbm.as_ptr() as usize;
    let (normal, color, sun) = (offset(view::NORMAL), offset(view::COLOR), offset(view::SUNLIGHT));
    prbm[normal + 18 * 3 + 4] ^= 0x10;
    prbm[color + 18 * 5 + 17] ^= 0x01;
    prbm[sun + 6 * 9 + 5] ^= 0x02;
    assert_eq!(round_trip(&mut codec, &prbm)[4], MODE_QUADS);
}

#[test]
fn corrupt_blobs_fail_cleanly() {
    let mut codec = CompactCodec::default();
    let mut blob = Vec::new();
    codec.encode_into(&sample(), &mut blob).unwrap();
    let mut out = Vec::new();
    for len in 0..blob.len() {
        assert!(codec.decode_into(&blob[..len], &mut out).is_err(), "prefix {len}");
    }
    let mut flipped = blob.clone();
    flipped[4] = 9;
    assert!(codec.decode_into(&flipped, &mut out).is_err());
}
