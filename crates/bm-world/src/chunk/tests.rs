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

#[test]
fn pre_1_18_is_reported_unsupported_for_now() {
    let f = Fixture::new();
    let mut w = Writer::new();
    w.int("DataVersion", 2586);
    assert!(matches!(f.parse(&w.finish()), Err(Error::UnsupportedVersion(2586))));
}
