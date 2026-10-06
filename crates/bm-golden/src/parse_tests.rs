use super::*;
use crate::test_prbm::Prbm;

fn err(p: &Prbm) -> String {
    format!("{:#}", parse(&p.bytes()).expect_err("malformed PRBM must not parse"))
}

#[test]
fn parses_writer_layout() {
    let tile = parse(&Prbm::one_triangle().bytes()).unwrap();
    assert_eq!(tile.face_count(), 1);
    assert_eq!(tile.groups, [Group { material: 5, start: 0, count: 3 }]);
    let f = tile.faces().next().unwrap();
    assert_eq!(f.material, 5);
    assert_eq!(f.pos[2], [1.0, 1.0, 1.0]);
    assert_eq!(f.uv[1], [1.0, 0.0]);
    assert_eq!(f.ao, [255, 191, 127]);
    assert_eq!((f.color, f.normal, f.blocklight, f.sunlight), ([255, 128, 0], [0, 127, 0], 14, 15));
}

#[test]
fn empty_tile() {
    let mut p = Prbm::one_triangle();
    p.vertices = 0;
    p.groups.clear();
    for a in &mut p.attrs {
        a.2.clear();
    }
    assert_eq!(parse(&p.bytes()).unwrap().face_count(), 0);
}

#[test]
fn attributes_are_found_by_name() {
    let mut p = Prbm::one_triangle();
    p.attrs.reverse();
    p.attrs.push(("extra", 0x07, vec![1, 2, 3]));
    let tile = parse(&p.bytes()).unwrap();
    assert_eq!(tile.position[1], [1.0, 1.0, 0.0]);
    assert_eq!(tile.sunlight, [15; 3]);
}

#[test]
fn multiple_groups_are_contiguous() {
    let mut p = Prbm::one_triangle();
    p.vertices = 6;
    for a in &mut p.attrs {
        a.2 = a.2.repeat(2);
    }
    p.groups = vec![[1, 0, 3], [4, 3, 3]];
    let tile = parse(&p.bytes()).unwrap();
    assert_eq!(tile.faces().map(|f| f.material).collect::<Vec<_>>(), [1, 4]);
}

#[test]
fn rejects_bad_header() {
    let mut p = Prbm::one_triangle();
    p.version = 2;
    assert!(err(&p).contains("version 2"));

    let mut p = Prbm::one_triangle();
    p.extra_flags = 0x80;
    assert!(err(&p).contains("indexed or big-endian"));

    let mut p = Prbm::one_triangle();
    p.vertices = 4;
    assert!(err(&p).contains("not a multiple of 3"));
}

#[test]
fn rejects_bad_attributes() {
    let mut p = Prbm::one_triangle();
    p.attrs.retain(|a| a.0 != "sunlight");
    assert!(err(&p).contains("missing attribute sunlight"));

    let mut p = Prbm::one_triangle();
    *p.attr_mut("normal") = ("normal", 0x53, [0, 127].repeat(3));
    assert!(err(&p).contains("cardinality 2 != 3"));

    let mut p = Prbm::one_triangle();
    p.attr_mut("color").1 = 0x63;
    assert!(err(&p).contains("expected u8"));

    let mut p = Prbm::one_triangle();
    p.attr_mut("position").1 = 0x22;
    assert!(err(&p).contains("unsupported encoding 2"));

    let mut p = Prbm::one_triangle();
    p.attr_mut("ao").1 = 0xc7;
    assert!(err(&p).contains("integer-typed"));
}

#[test]
fn rejects_bad_groups() {
    for groups in [
        vec![[5, 1, 3]],
        vec![[5, 0, 0]],
        vec![[-2, 0, 3]],
        vec![[5, 0, 2]],
        vec![[5, 0, 3], [6, 3, 3]],
        vec![[5, 0, 1], [6, 2, 2]],
    ] {
        let mut p = Prbm::one_triangle();
        p.groups = groups.clone();
        assert!(parse(&p.bytes()).is_err(), "{groups:?}");
    }
}

#[test]
fn rejects_trailing_bytes() {
    let mut p = Prbm::one_triangle();
    p.trailer = vec![0; 4];
    assert!(err(&p).contains("trailing bytes"));
}

#[test]
fn every_truncation_errors() {
    let b = Prbm::one_triangle().bytes();
    for n in 0..b.len() {
        assert!(parse(&b[..n]).is_err(), "prefix {n}");
    }
}

#[test]
fn single_byte_corruption_never_panics() {
    let b = Prbm::one_triangle().bytes();
    for i in 0..b.len() {
        for flip in [0x01, 0x80, 0xff] {
            let mut c = b.clone();
            c[i] ^= flip;
            let _ = parse(&c);
        }
    }
}

#[test]
fn huge_group_count_errors() {
    let mut p = Prbm::one_triangle();
    p.groups = vec![[5, 0, 3], [5, 3, i32::MAX]];
    assert!(parse(&p.bytes()).is_err());
}
