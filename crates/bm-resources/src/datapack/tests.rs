use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;

use super::*;

/// An in-memory zip pack with `files` as `(path, contents)`.
pub(crate) fn zip_pack(files: &[(&str, &[u8])]) -> Pack {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, SimpleFileOptions::default()).unwrap();
        w.write_all(body).unwrap();
    }
    Pack::zip(w.finish().unwrap().into_inner().into(), "test.zip".into()).unwrap()
}

#[test]
fn first_pack_wins_and_failures_fall_through() {
    let high = zip_pack(&[
        ("data/minecraft/worldgen/biome/plains.json", br#"{"temperature": 0.8}"#),
        ("data/minecraft/worldgen/biome/swamp.json", br#"{"effects": {"water_color": null}}"#),
        ("data/mod/worldgen/biome/sub/x.json", b"{}"),
        ("data/minecraft/dimension_type/the_nether.json", br#"{"height": 128, "has_ceiling": true}"#),
        ("data/minecraft/worldgen/biome/notes.txt", b"x"),
    ]);
    let low = zip_pack(&[
        ("data/minecraft/worldgen/biome/plains.json", br#"{"temperature": 0.1}"#),
        ("data/minecraft/worldgen/biome/swamp.json", br#"{"downfall": 0.9}"#),
    ]);
    let (dp, failures) = DataPack::load(&[high, low]);
    assert_eq!(failures.len(), 1);
    assert!(failures[0].file.ends_with("swamp.json"), "{failures:?}");
    assert_eq!(dp.biome("plains").unwrap().temperature, 0.8);
    assert_eq!(dp.biome("minecraft:swamp").unwrap().downfall, 0.9);
    assert!(dp.biome("mod:sub/x").is_some());
    assert_eq!(dp.biome_count(), 3);
    assert_eq!(dp.dimension_type("the_nether").unwrap().height, 128);
    assert_eq!(dp.dimension_type("overworld"), Some(&DimensionType::OVERWORLD));
    assert_eq!(dp.dimension_type_count(), 4);
}

#[test]
fn biome_table_indexes_by_registry_id() {
    let pack = zip_pack(&[("data/minecraft/worldgen/biome/desert.json", br#"{"temperature": 2.0, "downfall": 0}"#)]);
    let (dp, _) = DataPack::load(&[pack]);
    let registry = Biomes::default();
    let early = registry.intern("minecraft:unknown_early");
    let table = dp.biome_table(&registry);
    assert_eq!(table.get(registry.intern("desert")).temperature, 2.0);
    assert_eq!(table.get(early), &Biome::DEFAULT);
    assert_eq!(table.get(BiomeId::DEFAULT), &Biome::DEFAULT);
    assert_eq!(table.get(registry.intern("mod:late")), &Biome::DEFAULT);
}
