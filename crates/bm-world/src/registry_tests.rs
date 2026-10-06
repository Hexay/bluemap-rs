use super::*;

#[test]
fn reserved_ids() {
    let r = BlockStates::default();
    assert!(r.get(StateId::AIR).is_air);
    assert_eq!(&*r.get(StateId::MISSING).key, "bluemap:missing[]");
}

#[test]
fn property_order_and_namespace_do_not_matter() {
    let r = BlockStates::default();
    let a = r.intern("oak_stairs", &mut [("half", "bottom"), ("facing", "east")]);
    let b = r.intern("minecraft:oak_stairs", &mut [("facing", "east"), ("half", "bottom")]);
    assert_eq!(a, b);
    let s = r.get(a);
    assert_eq!(&*s.key, "minecraft:oak_stairs[facing=east,half=bottom]");
    assert_eq!(s.property("half"), Some("bottom"));
    assert_ne!(a, r.intern("minecraft:oak_stairs", &mut [("facing", "west"), ("half", "bottom")]));
}

#[test]
fn flags() {
    let r = BlockStates::default();
    assert!(r.get(r.intern("cave_air", &mut [])).is_air);
    assert!(r.get(r.intern("water", &mut [("level", "0")])).is_water);
    let fence = r.get(r.intern("oak_fence", &mut [("waterlogged", "true")]));
    assert!(fence.waterlogged && !fence.is_water);
}

#[test]
fn state_strings_parse() {
    let r = BlockStates::default();
    let id = r.intern_str("minecraft:oak_door[half=lower,facing=north]").unwrap();
    assert_eq!(id, r.intern("oak_door", &mut [("facing", "north"), ("half", "lower")]));
    assert_eq!(r.intern_str("stone"), r.intern_str("minecraft:stone[]"));
    assert_eq!(r.intern_str("a[broken]"), None);
}

#[test]
fn defaults_first_wins_and_unknown_blocks_have_no_properties() {
    let r = BlockStates::default();
    let n = r
        .add_defaults_json(r#"{"minecraft:lever": "minecraft:lever[face=wall,facing=north,powered=false]"}"#)
        .unwrap();
    assert_eq!(n, 1);
    r.add_default(r.intern("lever", &mut [("face", "floor")]));
    assert_eq!(&*r.get(r.default_state("lever")).key, "minecraft:lever[face=wall,facing=north,powered=false]");
    assert_eq!(&*r.get(r.default_state("mod:widget")).key, "mod:widget[]");
}

#[test]
fn bundled_bluemap_defaults_load() {
    let json = include_str!("../../../assets/resourceExtensions/data/minecraft/defaultBlockstates.json");
    let r = BlockStates::default();
    assert!(r.add_defaults_json(json).unwrap() > 1000);
    let door = r.get(r.default_state("minecraft:acacia_door"));
    assert_eq!(&*door.key, "minecraft:acacia_door[facing=north,half=lower,hinge=left,open=false,powered=false]");
}

#[test]
fn biomes_intern() {
    let b = Biomes::default();
    let plains = b.intern("plains");
    assert_eq!(plains, b.intern("minecraft:plains"));
    assert_ne!(plains, b.intern("minecraft:desert"));
    assert_eq!(&*b.name(plains), "minecraft:plains");
}
