use bm_world::BlockStates;

use super::*;
use crate::datapack::tests::zip_pack;

const T: Tristate = Tristate::True;
const F: Tristate = Tristate::False;
const U: Tristate = Tristate::Undefined;

fn model(culling: bool, occluding: bool) -> ModelProperties {
    ModelProperties { culling, occluding }
}

#[test]
fn tristates() {
    assert!(T.get_or(false) && !F.get_or(true) && U.get_or(true));
    assert_eq!((T.or(F), U.or(F), U.or(U)), (T, F, U));
    let base = BlockProperties { culling: T, occluding: F, random_offset: T, ..Default::default() };
    let over = BlockProperties { occluding: T, always_waterlogged: F, ..Default::default() };
    let merged = base.overridden_by(&over);
    assert_eq!(
        merged,
        BlockProperties { culling: T, occluding: T, always_waterlogged: F, random_offset: T, culling_identical: U }
    );
    assert!(!merged.is_always_waterlogged() && !merged.is_culling_identical() && merged.is_random_offset());
}

#[test]
fn load_order_and_first_fit() {
    let mut cfg = BlockPropertiesConfig::default();
    cfg.load_str(
        "{ 'glass': { occluding: false, cullingIdentical: true },
           'glass[x=1]': { culling: true },
           'kelp': { alwaysWaterlogged: true, comment: 'x', randomOffset: false } }",
    )
    .unwrap();
    assert!(cfg.load_str(r#"{"a": {"culling": true}, "b": {"culling": "true"}, "c": {}}"#).is_err());
    assert!(cfg.load_str(r#"{"d": 5}"#).is_err());
    assert_eq!(cfg.len(), 4);

    let states = BlockStates::default();
    let get = |s: &str| cfg.get(&states.get(states.intern_str(s).unwrap()));
    assert_eq!(get("glass[x=1]"), BlockProperties { occluding: F, culling_identical: T, ..Default::default() });
    assert_eq!(get("kelp[age=3]"), BlockProperties { always_waterlogged: T, random_offset: F, ..Default::default() });
    assert_eq!(get("a").culling, T);
    assert_eq!(get("b"), BlockProperties::default());
}

#[test]
fn resolve_fills_undefined_from_the_first_model() {
    let mut cfg = BlockPropertiesConfig::default();
    let pack = zip_pack(&[(
        "assets/minecraft/blockProperties.json",
        br#"{"glass": {"occluding": false}, "mushroom": {"culling": true, "occluding": true}}"#,
    )]);
    let mut failures = Vec::new();
    cfg.load_pack(&pack, &mut failures);
    assert!(failures.is_empty());

    let states = BlockStates::default();
    let state = |s: &str| states.get(states.intern_str(s).unwrap());
    let glass = cfg.resolve(&state("glass"), || [model(true, true), model(false, false)]);
    assert_eq!((glass.culling, glass.occluding), (T, F));
    let stone = cfg.resolve(&state("stone"), || [model(false, true), model(true, false)]);
    assert_eq!((stone.culling, stone.occluding), (F, T));
    let none = cfg.resolve(&state("air"), Vec::new);
    assert_eq!(none, BlockProperties::default());
    let mushroom = cfg.resolve(&state("mushroom"), || -> Vec<ModelProperties> { panic!("models not needed") });
    assert!(mushroom.is_culling() && mushroom.is_occluding());
}
