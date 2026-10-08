use super::*;

/// Test-only NBT writer.
#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn named(&mut self, ty: u8, name: &str) -> &mut Self {
        self.0.push(ty);
        self.str(name)
    }
    fn str(&mut self, s: &str) -> &mut Self {
        self.0.extend((s.len() as u16).to_be_bytes());
        self.0.extend(s.as_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn byte(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn end(&mut self) -> &mut Self {
        self.byte(tag::END)
    }
}

/// `{DataVersion: 4440, Status: "minecraft:full", sections: [{Y: -4b, data: [L; 1, -1]}, {Y: 0b}],
///   Heightmaps: {}, small: 7s}`
fn chunk() -> Vec<u8> {
    let mut w = W::default();
    w.named(tag::COMPOUND, "");
    w.named(tag::INT, "DataVersion").i32(4440);
    w.named(tag::STRING, "Status").str("minecraft:full");
    w.named(tag::LIST, "sections").byte(tag::COMPOUND).i32(2);
    w.named(tag::BYTE, "Y").byte(-4i8 as u8);
    w.named(tag::LONG_ARRAY, "data").i32(2);
    w.0.extend(1u64.to_be_bytes());
    w.0.extend(u64::MAX.to_be_bytes());
    w.end();
    w.named(tag::BYTE, "Y").byte(0).end();
    w.named(tag::COMPOUND, "Heightmaps").end();
    w.named(tag::SHORT, "small").0.extend(7i16.to_be_bytes());
    w.end();
    w.0
}

#[test]
fn reads_nested_fields_lazily() {
    let buf = chunk();
    let root = read_root(&buf).unwrap();
    assert_eq!(root.i64("DataVersion"), Some(4440));
    assert_eq!(root.str("Status"), Some("minecraft:full"));
    assert_eq!(root.i64("small"), Some(7));
    assert!(root.compound("Heightmaps").unwrap().entries().next().is_none());

    let sections: Vec<_> = root.list("sections").unwrap().compounds().collect();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].i64("Y"), Some(-4));
    let data = sections[0].get("data").unwrap().as_long_array().unwrap();
    assert_eq!(data.iter().collect::<Vec<_>>(), [1, u64::MAX]);
    assert!(sections[1].get("data").is_none());
}

#[test]
fn entries_visit_every_field_in_order() {
    let buf = chunk();
    let names: Vec<_> =
        read_root(&buf).unwrap().entries().map(|(n, _)| String::from_utf8(n.to_vec()).unwrap()).collect();
    assert_eq!(names, ["DataVersion", "Status", "sections", "Heightmaps", "small"]);
}

#[test]
fn every_truncation_is_an_error() {
    let buf = chunk();
    for cut in 0..buf.len() {
        assert!(read_root(&buf[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn malformed_documents_are_rejected() {
    assert_eq!(read_root(&[tag::LIST, 0, 0]).unwrap_err(), Error::RootNotCompound);
    let mut w = W::default();
    w.named(tag::COMPOUND, "").named(13, "x").end();
    assert_eq!(read_root(&w.0).unwrap_err(), Error::BadTag(13));

    let mut w = W::default();
    w.named(tag::COMPOUND, "").named(tag::INT_ARRAY, "neg").i32(-1).end();
    assert_eq!(read_root(&w.0).unwrap_err(), Error::Truncated);
}

#[test]
fn nesting_is_capped() {
    let mut w = W::default();
    w.named(tag::COMPOUND, "");
    for _ in 0..600 {
        w.named(tag::COMPOUND, "c");
    }
    for _ in 0..=600 {
        w.end();
    }
    assert_eq!(read_root(&w.0).unwrap_err(), Error::TooDeep);
}

#[test]
fn end_typed_lists_are_empty_whatever_their_length() {
    let mut w = W::default();
    w.named(tag::COMPOUND, "").named(tag::LIST, "l").byte(tag::END).i32(5).end();
    let root = read_root(&w.0).unwrap();
    assert!(root.list("l").unwrap().is_empty());
}

#[test]
fn compounds_of_a_non_compound_list_is_empty() {
    let mut w = W::default();
    w.named(tag::COMPOUND, "").named(tag::LIST, "l").byte(tag::INT).i32(2).i32(1).i32(2).end();
    let root = read_root(&w.0).unwrap();
    let list = root.list("l").unwrap();
    assert_eq!(list.compounds().count(), 0);
    assert_eq!(list.iter().filter_map(|t| t.as_i64()).collect::<Vec<_>>(), [1, 2]);
}
