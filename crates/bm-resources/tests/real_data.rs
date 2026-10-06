//! Loads the bundled configs and the real 26.3 client jar's datapack and colormaps.
//! Run: `cargo test -p bm-resources --test real_data -- --ignored --nocapture`.

use std::path::{Path, PathBuf};

use bm_resources::Pack;
use bm_resources::color::{BlockColorsConfig, BlockPropertiesConfig, ColorMaps, ModelProperties};
use bm_resources::datapack::{DataPack, GrassColorModifier};
use bm_world::{Biomes, BlockStates};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// `work/` is git-ignored, so worktrees find it in an ancestor checkout.
fn client_jar() -> PathBuf {
    let rel = "work/bluemap/vanilla/data/minecraft-client-26.3.jar";
    manifest().ancestors().map(|a| a.join(rel)).find(|p| p.exists()).expect("minecraft-client-26.3.jar")
}

#[test]
#[ignore = "needs work/bluemap/vanilla/data/minecraft-client-26.3.jar"]
fn bundled_configs_and_vanilla_datapack() {
    let ext = Pack::open(&manifest().join("../../assets/resourceExtensions")).unwrap();
    let jar = Pack::open(&client_jar()).unwrap();
    let packs = [ext, jar];

    let (mut colors_cfg, mut props_cfg, mut colormaps) =
        (BlockColorsConfig::default(), BlockPropertiesConfig::default(), ColorMaps::default());
    let mut failures = Vec::new();
    for pack in &packs {
        colors_cfg.load_pack(pack, &mut failures);
        props_cfg.load_pack(pack, &mut failures);
        colormaps.load_pack(pack, &mut failures);
    }
    let (dp, data_failures) = DataPack::load(&packs);
    eprintln!(
        "blockColors {} entries, blockProperties {} entries, colormaps {}, biomes {}, dimension types {}",
        colors_cfg.len(),
        props_cfg.len(),
        colormaps.len(),
        dp.biome_count(),
        dp.dimension_type_count()
    );
    eprintln!("failures: {failures:?} {data_failures:?}");
    assert!(failures.is_empty() && data_failures.is_empty());
    assert_eq!(colormaps.len(), 3);
    assert_eq!(dp.dimension_type_count(), 4);

    let swamp = dp.biome("minecraft:swamp").unwrap();
    assert_eq!(swamp.grass_color_modifier, GrassColorModifier::Swamp);
    assert_eq!(dp.biome("dark_forest").unwrap().grass_color_modifier, GrassColorModifier::DarkForest);
    let nether = dp.dimension_type("the_nether").unwrap();
    eprintln!("the_nether: {nether:?}");

    let states = BlockStates::default();
    let registry = Biomes::default();
    let table = dp.biome_table(&registry);
    let colors = colors_cfg.bake(&colormaps);
    let state = |s: &str| states.get(states.intern_str(s).unwrap());
    for (block, biome) in [
        ("grass_block", "plains"),
        ("oak_leaves", "plains"),
        ("leaf_litter", "plains"),
        ("water", "plains"),
        ("grass_block", "swamp"),
        ("grass_block", "dark_forest"),
        ("oak_leaves", "badlands"),
        ("water", "swamp"),
        ("birch_leaves", "plains"),
        ("redstone_wire[power=10]", "plains"),
        ("melon_stem[age=7]", "plains"),
        ("stone", "plains"),
    ] {
        let id = registry.intern(biome);
        let c = colors.color(&state(block), (100, 64, 100), &table, |_, _, _| id);
        eprintln!("{block:28} {biome:12} #{:08X}", c.get_int() as u32);
    }

    let props = |s: &str| props_cfg.resolve(&state(s), || [ModelProperties { culling: true, occluding: true }]);
    assert!(props("seagrass").is_always_waterlogged());
    assert!(props("glass").is_culling_identical() && !props("glass").is_occluding() && props("glass").is_culling());
    assert!(props("poppy").is_random_offset());
}
