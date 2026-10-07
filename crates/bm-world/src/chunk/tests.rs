use bm_nbt::Writer;

use super::*;

fn pack(values: &[u32], bits: u32) -> Vec<u64> {
    let per_long = (64 / bits) as usize;
    values.chunks(per_long).map(|c| c.iter().enumerate().fold(0, |l, (i, &v)| l | (v as u64) << (i as u32 * bits))).collect()
}

struct Fixture {
    states: BlockStates,
    biomes: Biomes,
}

impl Fixture {
    fn new() -> Self {
        let states = BlockStates::default();
        states.add_defaults_json(r#"{"minecraft:lever": "minecraft:lever[face=wall,facing=north,powered=false]"}"#).unwrap();
        Self { states, biomes: Biomes::default() }
    }

    fn parse(&self, nbt: &[u8]) -> Result<Chunk> {
        Chunk::parse(nbt, &ChunkContext { states: &self.states, biomes: &self.biomes, dimension: &DimensionType::OVERWORLD })
    }
}

/// Sections y=-4 (stone/dirt checkerboard by x, two biomes, partial sky light) and y=-2 (light only); y=-3 is a gap.
fn modern_chunk(status: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.int("DataVersion", 4440).string("Status", status).long("InhabitedTime", 77);
    w.begin_compound("Heightmaps");
    w.long_array("WORLD_SURFACE", &pack(&[70; 256], 9));
    w.long_array("OCEAN_FLOOR", &[1, 2]);
    w.end_compound();
    w.begin_compound_list("sections", 2);
    {
        w.byte("Y", -4);
        w.begin_compound("block_states");
        w.begin_compound_list("palette", 2);
        w.string("Name", "minecraft:stone").end_compound();
        w.string("Name", "minecraft:oak_log").begin_compound("Properties").string("axis", "x").end_compound().end_compound();
        let indices: Vec<u32> = (0..4096).map(|i| i % 2).collect();
        w.long_array("data", &pack(&indices, 4));
        w.end_compound();
        w.begin_compound("biomes").string_list("palette", &["plains", "minecraft:desert"]);
        let cells: Vec<u32> = (0..64).map(|i| u32::from(i >= 32)).collect();
        w.long_array("data", &pack(&cells, 1)).end_compound();
        let mut sky = [0xFFu8; 2048];
        sky[0] = 0x3F;
        w.byte_array("SkyLight", &sky);
        w.end_compound();
    }
    {
        w.byte("Y", -2);
        w.byte_array("BlockLight", &[0x77; 2048]);
        w.end_compound();
    }
    w.begin_compound_list("block_entities", 1);
    w.string("id", "minecraft:chest").int("x", 3).int("y", -60).int("z", 7).end_compound();
    w.finish()
}

#[test]
fn blocks_biomes_and_metadata() {
    let f = Fixture::new();
    let c = f.parse(&modern_chunk("minecraft:full")).unwrap();
    assert_eq!((c.data_version, c.generated, c.has_light, c.inhabited_time), (4440, true, true, 77));
    let log = f.states.intern("oak_log", &mut [("axis", "x")]);
    let stone = f.states.intern("stone", &mut []);
    assert_eq!((c.block(0, -64, 0), c.block(1, -64, 0), c.block(17, -49, 5)), (stone, log, log));
    assert_eq!(c.block(0, -40, 0), StateId::AIR, "gap section");
    assert_eq!(c.block(0, 200, 0), StateId::AIR, "above");
    assert_eq!((c.min_y(), c.max_y()), (-64, -17));

    assert_eq!(&*f.biomes.name(c.biome(0, -64, 0)), "minecraft:plains");
    assert_eq!(&*f.biomes.name(c.biome(0, -56, 0)), "minecraft:desert");
    assert_eq!(c.biome(0, 100, 0), BiomeId::DEFAULT);

    assert_eq!(c.world_surface_y(5, 5), Some(70 - 64));
    assert_eq!(c.ocean_floor_y(0, 0), None, "truncated heightmap");
    assert_eq!(c.block_entities, [BlockEntity { id: "minecraft:chest".into(), x: 3, y: -60, z: 7 }]);
}

#[test]
fn column_reads_match_block_and_light() {
    let f = Fixture::new();
    for status in ["minecraft:full", "minecraft:features"] {
        let c = f.parse(&modern_chunk(status)).unwrap();
        for (x, z) in [(0, 0), (1, 0), (17, 5), (-3, 9)] {
            let (mut states, mut light) = (Vec::new(), Vec::new());
            c.column_into(x, z, -100, 300, &mut states, &mut light);
            for (i, y) in (-100..=300).enumerate() {
                let (sky, block) = c.light(x, y, z);
                assert_eq!((states[i], light[i]), (c.block(x, y, z), [sky, block]), "{status} {x},{y},{z}");
            }
        }
    }
}

#[test]
fn light_follows_bluemap_fallbacks() {
    let f = Fixture::new();
    let c = f.parse(&modern_chunk("minecraft:full")).unwrap();
    assert_eq!(c.light(0, -64, 0), (15, 0), "low nibble of byte 0");
    assert_eq!(c.light(1, -64, 0), (3, 0), "high nibble of byte 0");
    assert_eq!(c.light(5, -60, 5), (15, 0));
    assert_eq!(c.light(0, -40, 0), (15, 0), "gap section: open sky");
    assert_eq!(c.light(0, -30, 0), (0, 7), "block light only");
    assert_eq!(c.light(0, -100, 0), (0, 0), "below the lowest section");
    assert_eq!(c.light(0, 300, 0), (15, 0), "above the highest section");

    let unlit = f.parse(&modern_chunk("minecraft:features")).unwrap();
    assert!(unlit.generated && !unlit.has_light);
    assert_eq!(unlit.light(0, -100, 0), (15, 0));
    assert!(!f.parse(&modern_chunk("empty")).unwrap().generated);
}

#[test]
fn palette_shorthands_of_26_3() {
    let f = Fixture::new();
    let mut w = Writer::new();
    w.int("DataVersion", 5023).begin_compound_list("sections", 2);
    w.byte("Y", 0).begin_compound("block_states").string_list("palette", &["lever"]).end_compound().end_compound();
    w.byte("Y", 1).begin_compound("block_states").begin_compound_list("palette", 3);
    w.string("", "minecraft:lever").end_compound();
    w.string("id", "minecraft:lever").begin_compound("properties").string("face", "floor").end_compound().end_compound();
    w.int("broken", 1).end_compound();
    w.long_array("data", &pack(&(0..4096).map(|i| i % 3).collect::<Vec<_>>(), 4)).end_compound().end_compound();
    let c = f.parse(&w.finish()).unwrap();

    let default = f.states.intern("lever", &mut [("face", "wall"), ("facing", "north"), ("powered", "false")]);
    assert_eq!(c.block(0, 0, 0), default, "bare string");
    assert_eq!(c.block(0, 16, 0), default, "{{\"\": name}}");
    assert_eq!(c.block(1, 16, 0), f.states.intern("lever", &mut [("face", "floor")]), "{{id, properties}}");
    assert_eq!(c.block(2, 16, 0), StateId::MISSING, "entry without a name");
}

#[test]
fn out_of_range_palette_index_is_missing() {
    let f = Fixture::new();
    let mut w = Writer::new();
    w.int("DataVersion", 3000).begin_compound_list("sections", 1).byte("Y", 0);
    w.begin_compound("block_states").string_list("palette", &["stone", "dirt"]);
    w.long_array("data", &pack(&[15; 4096], 4)).end_compound().end_compound();
    assert_eq!(f.parse(&w.finish()).unwrap().block(0, 0, 0), StateId::MISSING);
}

/// Values packed back to back across long boundaries (1.13–1.15).
fn pack_spanning(values: &[u32], bits: u32) -> Vec<u64> {
    let mut longs = vec![0u64; (values.len() * bits as usize).div_ceil(64)];
    for (i, &v) in values.iter().enumerate() {
        let (bit, v) = (i * bits as usize, v as u64);
        longs[bit / 64] |= v << (bit % 64);
        if bit % 64 + bits as usize > 64 {
            longs[bit / 64 + 1] |= v >> (64 - bit % 64);
        }
    }
    longs
}

/// A `Level` chunk with one section at y=0 holding 20 states (5 bits), so index arrays exercise both layouts.
fn legacy_chunk(data_version: i32, status: &str, padded: bool, biomes: &[i32]) -> Vec<u8> {
    let indices: Vec<u32> = (0..4096).map(|i| i % 20).collect();
    let mut w = Writer::new();
    w.int("DataVersion", data_version).begin_compound("Level").string("Status", status);
    w.int_array("Biomes", biomes);
    w.begin_compound("Heightmaps");
    let heights = vec![65; 256];
    w.long_array("WORLD_SURFACE", &if padded { pack(&heights, 9) } else { pack_spanning(&heights, 9) });
    w.end_compound();
    w.begin_compound_list("Sections", 1).byte("Y", 0).begin_compound_list("Palette", 20);
    for i in 0..20 {
        w.string("Name", &format!("minecraft:block_{i}")).end_compound();
    }
    w.long_array("BlockStates", &if padded { pack(&indices, 5) } else { pack_spanning(&indices, 5) });
    w.byte_array("SkyLight", &[0xAA; 2048]).end_compound();
    w.begin_compound_list("TileEntities", 1).string("id", "minecraft:sign").int("x", 1).int("y", 2).int("z", 3);
    w.end_compound().end_compound();
    w.finish()
}

#[test]
fn chunk_1_16_padded_with_repeating_biome_cells() {
    let f = Fixture::new();
    let biomes: Vec<i32> = (0..1024).map(|i| if i < 16 { 2 } else { 1 }).collect();
    let c = f.parse(&legacy_chunk(2586, "full", true, &biomes)).unwrap();
    assert!(c.has_light);
    for i in [0, 12, 19, 4095] {
        let state = f.states.get(c.block(i & 15, i >> 8, (i >> 4) & 15));
        assert_eq!(*state.name, *format!("minecraft:block_{}", i % 20));
    }
    assert_eq!(&*f.biomes.name(c.biome(0, 0, 0)), "minecraft:desert");
    assert_eq!(&*f.biomes.name(c.biome(0, 4, 0)), "minecraft:plains");
    assert_eq!(&*f.biomes.name(c.biome(0, 16, 0)), "minecraft:desert", "BlueMap repeats the lowest 16 blocks");
    assert_eq!(c.world_surface_y(3, 3), Some(65), "no min-y offset before 1.18");
    assert_eq!(c.light(0, 0, 0), (10, 0));
    assert_eq!(c.block_entities[0].id.as_ref(), "minecraft:sign");
}

#[test]
fn chunk_1_13_spanning_with_column_biomes() {
    let f = Fixture::new();
    let biomes: Vec<i32> = (0..256).map(|i| if i == 17 { 6 } else { 999 }).collect();
    let c = f.parse(&legacy_chunk(1631, "postprocessed", false, &biomes)).unwrap();
    assert!(c.has_light, "postprocessed counts as lit before 1.16");
    for i in [0, 12, 13, 19, 4095] {
        let state = f.states.get(c.block(i & 15, i >> 8, (i >> 4) & 15));
        assert_eq!(*state.name, *format!("minecraft:block_{}", i % 20), "index {i}");
    }
    assert_eq!(&*f.biomes.name(c.biome(1, 100, 1)), "minecraft:swamp");
    assert_eq!(c.biome(0, 0, 0), BiomeId::DEFAULT, "unknown legacy id");
    assert_eq!(c.world_surface_y(15, 15), Some(65));

    let c = f.parse(&legacy_chunk(2586, "postprocessed", true, &[])).unwrap();
    assert!(!c.has_light, "1.16+ needs full");
    assert_eq!(c.biome(0, 0, 0), BiomeId::DEFAULT, "no biome array");
}

#[test]
fn pre_flattening_chunks_are_unsupported() {
    let f = Fixture::new();
    let mut w = Writer::new();
    w.int("DataVersion", 1343);
    assert!(matches!(f.parse(&w.finish()), Err(Error::UnsupportedVersion(1343))));
    let mut w = Writer::new();
    w.int("DataVersion", 2586);
    assert!(matches!(f.parse(&w.finish()), Err(Error::Corrupt(_))), "legacy chunk without Level");
}
