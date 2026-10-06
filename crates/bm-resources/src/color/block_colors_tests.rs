use bm_world::BlockStates;

use super::*;
use crate::datapack::Biome;
use crate::datapack::tests::zip_pack;

fn fixed(argb: u32) -> ColorSource {
    ColorSource::Fixed(*Color::default().set_int(argb as i32))
}

#[test]
fn value_syntax() {
    assert_eq!(ColorSource::parse("@foliage"), ColorSource::Foliage);
    assert_eq!(ColorSource::parse("@minecraft:dry_foliage"), ColorSource::DryFoliage);
    assert_eq!(ColorSource::parse("@grass"), ColorSource::Grass);
    assert_eq!(ColorSource::parse("@water"), ColorSource::Water);
    assert_eq!(ColorSource::parse("@redstone"), ColorSource::Redstone);
    assert_eq!(ColorSource::parse("@Water"), ColorSource::Fixed(premultiplied_argb(WHITE)));
    assert_eq!(ColorSource::parse("#f9fffe"), fixed(0xFFF9_FFFE));
    assert_eq!(ColorSource::parse("#11223344"), fixed(0x4411_2233));
    assert_eq!(ColorSource::parse("#abc"), fixed(0xFFAA_BBCC));
    assert_eq!(ColorSource::parse("#xyz"), ColorSource::Fixed(premultiplied_argb(WHITE)));
    assert_eq!(ColorSource::parse("8431445"), fixed(0xFF80_A755));
    assert_eq!(ColorSource::parse("  "), ColorSource::Fixed(premultiplied_argb(WHITE)));
    assert_eq!(ColorSource::parse("colormap/Foo"), ColorSource::ColorMap(ResourcePath::key("colormap/Foo")));
}

#[test]
fn first_definition_of_a_key_wins_and_errors_keep_earlier_entries() {
    let mut cfg = BlockColorsConfig::default();
    cfg.load_str(r##"{"stone[b=1,a=2]": "#ff0000", "minecraft:stone[a=2,b=1]": "#00ff00", "dirt": 8431445}"##).unwrap();
    assert_eq!(cfg.len(), 2);
    let err = cfg.load_str(r##"{"stone": "#0000ff", "sand": true, "gravel": "#000000"}"##).unwrap_err();
    assert!(matches!(err, ConfigError::Expected { .. }));
    assert_eq!(cfg.len(), 3);
    assert!(cfg.load_str(r##"{"glass": "#000000", "bad[x]": "#000000", "ice": "#000000"}"##).is_err());
    assert!(cfg.load_str(r##"{"clay": "#000000" "##).is_err());
    assert_eq!(cfg.len(), 5);

    let states = BlockStates::default();
    let colors = cfg.bake(&ColorMaps::default());
    let tint = |s: &str| match colors.tint(&states.get(states.intern_str(s).unwrap())) {
        Tint::Fixed(c) => c.get_int() as u32,
        t => panic!("{t:?}"),
    };
    assert_eq!(tint("stone[a=2,b=1,c=3]"), 0xFFFF_0000);
    assert_eq!(tint("stone[a=1]"), 0xFF00_00FF);
    assert_eq!(tint("clay"), 0xFF00_0000);
    assert_eq!(tint("gravel"), 0xFFFF_FFFF);
}

#[test]
fn packs_load_per_namespace_and_bake_colormaps() {
    let pack = zip_pack(&[
        (
            "assets/minecraft/blockColors.json",
            br##"{"grass_block": "@grass", "vine": "minecraft:colormap/custom", "x": "colormap/none"}"##,
        ),
        ("assets/mod/blockColors.json", br##"{"grass_block": "#000000", "mod:leaf": "@foliage"}"##),
    ]);
    let mut cfg = BlockColorsConfig::default();
    let mut failures = Vec::new();
    cfg.load_pack(&pack, &mut failures);
    assert!(failures.is_empty());
    assert_eq!(cfg.len(), 4);

    let mut maps = ColorMaps::default();
    maps.insert(ResourcePath::key("colormap/custom"), ColorMap::from_argb(vec![0xFF12_3456; 65536].into()));
    let colors = cfg.bake(&maps);
    let states = BlockStates::default();
    let state = |s: &str| states.get(states.intern_str(s).unwrap());
    assert!(matches!(colors.tint(&state("grass_block[snowy=false]")), Tint::Grass));
    assert!(matches!(colors.tint(&state("mod:leaf")), Tint::Foliage));
    assert!(matches!(colors.tint(&state("vine")), Tint::ColorMap(_)));
    assert!(matches!(colors.tint(&state("x")), Tint::Fixed(c) if c.r == 1.0));
    assert!(matches!(colors.tint(&state("stone")), Tint::Fixed(c) if c.r == 1.0 && c.premultiplied));

    let biomes = BiomeTable::default();
    let at = |_, _, _| BiomeId::DEFAULT;
    let vine = colors.color(&state("vine"), (0, 0, 0), &biomes, at);
    assert_eq!(vine.get_int(), premultiplied_argb(0xFF12_3456).get_int());
    let grass = colors.color(&state("grass_block"), (5, 64, 5), &biomes, at);
    let expected = tint::blend(5, 64, 5, |x, _, z| tint::grass(&Biome::DEFAULT, None, x, z));
    assert_eq!(grass, expected);
    let wire = colors.color(&state("redstone_wire[power=15]"), (0, 0, 0), &biomes, at);
    assert_eq!(wire.get_int() as u32, 0xFFFF_FFFF, "unmapped block is white");
}
