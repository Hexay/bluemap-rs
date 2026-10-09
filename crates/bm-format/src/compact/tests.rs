use super::face::hash_offset;
use super::*;
use crate::prbm::TileModel;

/// A quad with corners `c` (block-local), moved the way the mesher does: random offset first, then the block.
fn quad(m: &mut TileModel, material: u32, c: [[f32; 3]; 4], d: [f32; 2], block: [i32; 3], ao: [f32; 4]) {
    let p = c.map(|v| [(v[0] + d[0]) + block[0] as f32, v[1] + block[1] as f32, (v[2] + d[1]) + block[2] as f32]);
    for tri in [[0, 1, 2], [0, 2, 3]] {
        tri.iter().for_each(|&i| m.position.extend(p[i]));
        tri.iter().for_each(|&i| m.uv.extend([c[i][0], c[i][2]]));
        tri.iter().for_each(|&i| m.ao.push(ao[i]));
        m.color.extend([1., 0.5, 0.25]);
        m.sunlight.push((block[0] & 15) as u8);
        m.blocklight.push((block[2] & 15) as u8);
        m.material.push(material);
    }
}

const TOP: [[f32; 3]; 4] = [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]];
const SIDE: [[f32; 3]; 4] = [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]];
const CROSS: [[f32; 3]; 4] = [[0.05, 0., 0.05], [0.95, 0., 0.95], [0.95, 1., 0.95], [0.05, 1., 0.05]];
const ORIGIN: [i32; 2] = [-1566, 34];

/// Steps of terrain with side faces, a slab-like inner face, and offset plants on top.
fn terrain() -> Vec<u8> {
    let mut m = TileModel::default();
    for x in 0..12 {
        for z in 0..12 {
            let y = 60 + (x * z) % 3;
            quad(&mut m, 0, TOP, [0.; 2], [x, y, z], [1., 0.75, 0.5, 0.25]);
            quad(&mut m, 1, SIDE, [0.; 2], [x, y, z], [1.; 4]);
            if (x + z) % 4 == 0 {
                let d = hash_offset(ORIGIN[0] + x, ORIGIN[1] + z);
                quad(&mut m, 2, CROSS, d, [x, y + 1, z], [1.; 4]);
                quad(&mut m, 2, TOP.map(|v| [v[0], 0.5, v[2]]), [0.; 2], [x, y - 40, z], [1.; 4]);
            }
        }
    }
    let mut sorted = TileModel::default();
    let mut order: Vec<usize> = (0..m.material.len()).collect();
    order.sort_by_key(|&i| m.material[i]);
    for i in order {
        sorted.position.extend(&m.position[9 * i..9 * i + 9]);
        sorted.uv.extend(&m.uv[6 * i..6 * i + 6]);
        sorted.ao.extend(&m.ao[3 * i..3 * i + 3]);
        sorted.color.extend(&m.color[3 * i..3 * i + 3]);
        sorted.sunlight.push(m.sunlight[i]);
        sorted.blocklight.push(m.blocklight[i]);
        sorted.material.push(m.material[i]);
    }
    let mut out = Vec::new();
    sorted.write_prbm(&mut out).unwrap();
    out
}

fn round_trip(codec: &mut CompactCodec, input: &[u8], origin: Option<[i32; 2]>) -> Vec<u8> {
    let (mut blob, mut out) = (Vec::new(), Vec::new());
    codec.encode_into(input, origin, &mut blob).unwrap();
    assert!(is_compact(&blob));
    codec.decode_into(&blob, &mut out).unwrap();
    assert!(out == input, "round trip differs");
    blob
}

#[test]
fn terrain_round_trips_and_uses_hashed_shapes() {
    let mut codec = CompactCodec::default();
    let input = terrain();
    let with_origin = round_trip(&mut codec, &input, Some(ORIGIN));
    assert_eq!(with_origin[4], MODE_MODEL);
    let tables = &codec.enc.shapes.tables;
    assert!(tables.hashed[0].iter().any(|&h| h), "plants should share a hashed shape");
    assert!(tables.shapes[0].len() <= 4, "{} x shapes", tables.shapes[0].len());
    let mut faces = Vec::new();
    tables.faces(&mut faces).unwrap();
    let full: Vec<_> = faces.iter().map(|f| (f.full, f.normal)).collect();
    assert_eq!(full[..2], [(true, [0, 1, 0]), (true, [0, 0, 1])]);
    assert!(full[2..].iter().all(|f| !f.0));
    let without = round_trip(&mut codec, &input, None);
    let wrong = round_trip(&mut codec, &input, Some([7, 7]));
    assert!(with_origin.len() < without.len() && with_origin.len() < wrong.len());
}

#[test]
fn odd_values_become_exceptions() {
    let mut codec = CompactCodec::default();
    let mut m = TileModel::default();
    quad(&mut m, 0, TOP, [0.; 2], [3, 70, 5], [1.; 4]);
    quad(&mut m, 0, TOP, [0.; 2], [0, 0, 0], [1.; 4]);
    // z of the second quad's vertex 0, in both of its triangles
    m.position[18 + 2] = -0.0;
    m.position[18 + 9 + 2] = -0.0;
    let mut input = Vec::new();
    m.write_prbm(&mut input).unwrap();
    round_trip(&mut codec, &input, None);
}

#[test]
fn per_quad_deviations_become_exceptions() {
    let mut codec = CompactCodec::default();
    let mut prbm = terrain();
    let v = view::parse(&prbm).unwrap();
    let offset = |a: usize| v.attrs[a].as_ptr() as usize - prbm.as_ptr() as usize;
    let (normal, color, sun) = (offset(view::NORMAL), offset(view::COLOR), offset(view::SUNLIGHT));
    prbm[normal + 18 * 3 + 4] ^= 0x10;
    prbm[color + 18 * 5 + 17] ^= 0x01;
    prbm[sun + 6 * 9 + 5] ^= 0x02;
    assert_eq!(round_trip(&mut codec, &prbm, Some(ORIGIN))[4], MODE_MODEL);
}

#[test]
fn anything_else_round_trips_raw() {
    let mut codec = CompactCodec::default();
    let mut m = TileModel::default();
    quad(&mut m, 0, TOP, [0.; 2], [3, 70, 5], [1.; 4]);
    let mut clean = Vec::new();
    m.write_prbm(&mut clean).unwrap();
    assert_eq!(round_trip(&mut codec, &clean, None)[4], MODE_MODEL);

    m.position[4] = f32::NAN;
    let (mut nan, mut empty) = (Vec::new(), Vec::new());
    m.write_prbm(&mut nan).unwrap();
    TileModel::default().write_prbm(&mut empty).unwrap();
    // vertex 3 ≠ vertex 0, a padding byte, an ao level and a light value the mesher never writes
    let mut edits = Vec::new();
    let v = view::parse(&clean).unwrap();
    let offset = |a: usize| v.attrs[a].as_ptr() as usize - clean.as_ptr() as usize;
    for (at, flip) in
        [(offset(view::POSITION) + 3 * 12, 1), (18, 1), (offset(view::AO), 1), (offset(view::SUNLIGHT), 0x40)]
    {
        let mut edited = clean.clone();
        edited[at] ^= flip;
        edits.push(edited);
    }
    // incompressible: overflows the first output buffer
    let mut x = 1u64;
    let noise: Vec<u8> =
        (0..100_000).map(|_| (x = x.wrapping_mul(6364136223846793005).wrapping_add(1), (x >> 56) as u8).1).collect();
    let inputs = [&nan[..], &empty, b"not a tile", b"", &noise];
    for input in inputs.into_iter().chain(edits.iter().map(Vec::as_slice)) {
        assert_eq!(round_trip(&mut codec, input, Some(ORIGIN))[4], MODE_RAW);
    }
}

#[test]
fn corrupt_blobs_fail_without_panicking() {
    let mut codec = CompactCodec::default();
    let mut body = Vec::new();
    assert!(codec.body_into(&terrain(), Some(ORIGIN), &mut body));
    let mut out = Vec::new();
    for cut in (0..body.len()).step_by(7) {
        assert!(decode::quads(&body[..cut], &mut out, &mut codec.dec).is_err());
    }
    let mut x = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..3000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let mut bad = body.clone();
        bad[(x >> 8) as usize % body.len()] ^= 1 << (x & 7);
        let _ = decode::quads(&bad, &mut out, &mut codec.dec);
    }
    let blob = round_trip(&mut codec, &terrain(), Some(ORIGIN));
    for len in 0..blob.len() {
        assert!(codec.decode_into(&blob[..len], &mut out).is_err(), "prefix {len}");
    }
    for mode in [MODE_RAW, 9] {
        let mut flipped = blob.clone();
        flipped[4] = mode;
        assert!(codec.decode_into(&flipped, &mut out).is_err() || out != terrain());
    }
    let mut old = blob;
    old[..4].copy_from_slice(b"BMQ2");
    assert!(codec.decode_into(&old, &mut out).is_err());
}
