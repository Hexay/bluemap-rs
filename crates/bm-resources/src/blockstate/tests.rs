use std::sync::Arc;

use bm_world::{BlockState, BlockStates};

use super::condition::java_split;
use super::ref_data::{MATRICES, PICKS, WEIGHTS};
use super::*;
use crate::{ResourcePath, json};

fn state(s: &str) -> Arc<BlockState> {
    let registry = BlockStates::default();
    let id = registry.intern_str(s).unwrap();
    registry.get(id)
}

fn def(src: &str) -> BlockStateDef {
    BlockStateDef::from_json(&json::parse(src).unwrap()).unwrap()
}

fn def_err(src: &str) -> ParseError {
    BlockStateDef::from_json(&json::parse(src).unwrap()).unwrap_err()
}

fn models<'a>(d: &'a BlockStateDef, s: &'a BlockState) -> Vec<&'a str> {
    d.variants_at(s, 0, 0, 0).map(|v| v.model.as_str()).collect()
}

#[test]
fn java_split_drops_trailing_empties_only() {
    assert_eq!(java_split("", ','), [""]);
    assert_eq!(java_split("a", ','), ["a"]);
    assert_eq!(java_split("a,,b,,", ','), ["a", "", "b"]);
    assert_eq!(java_split(",a", ','), ["", "a"]);
    assert!(java_split(",,", ',').is_empty());
}

#[test]
fn variant_keys() {
    use Condition as C;
    let p = |k: &str, v: &str| C::Property(k.into(), v.into());
    for all in ["", "default", "normal", ","] {
        assert_eq!(C::parse_variant_key(all), C::All, "{all:?}");
    }
    assert_eq!(C::parse_variant_key("Facing=North"), p("facing", "north"));
    assert_eq!(C::parse_variant_key("a=b,c=d=e"), C::And([p("a", "b"), p("c", "d=e")].into()));
    assert_eq!(C::parse_variant_key("a=b|c"), p("a", "b|c"));
    assert_eq!(C::parse_variant_key("garbage"), C::None);
    assert_eq!(C::parse_variant_key("a=b,garbage"), p("a", "b"));
    assert_eq!(C::parse_variant_key("a="), p("a", ""));
}

#[test]
fn variants_resolve_in_file_order_with_fallbacks() {
    let d = def(r#"{"variants": {
        "__comment": "x",
        "z=1": {"model": "Z"},
        "a=1": {"model": "A"},
        "bogus": {"model": "dropped"},
        "": [{"model": "D1"}],
        "normal": {"model": "D2"}
    }}"#);
    let v = d.variants.as_ref().unwrap();
    assert_eq!(v.sets.len(), 2);
    assert_eq!(models(&d, &state("x[a=1,z=1]")), ["minecraft:z"]);
    assert_eq!(models(&d, &state("x[a=1]")), ["minecraft:a"]);
    assert_eq!(models(&d, &state("x[a=2]")), ["minecraft:d2"]);

    let no_default = def(r#"{"variants": {"a=1": {"model": "first"}, "a=2": {"model": "second"}}}"#);
    assert_eq!(models(&no_default, &state("x[a=3]")), ["minecraft:first"]);
    assert_eq!(models(&no_default, &state("x")), ["minecraft:first"]);
}

#[test]
fn multipart_conditions() {
    let d = def(r#"{"multipart": [
        {"apply": {"model": "always"}},
        {"when": {"north": "true", "east": true}, "apply": {"model": "and"}},
        {"when": {"facing": "North|SOUTH"}, "apply": {"model": "set"}},
        {"when": {"OR": [{"a": "1"}, {"AND": [{"b": "1"}, {"c": "1|2"}]}]}, "apply": {"model": "or"}},
        {"when": {"a": "1"}},
        {"when": {"a": "1"}, "apply": null},
        {"when": {"__comment": "x", "a": 1}, "apply": [{"model": "num"}]}
    ]}"#);
    assert_eq!(d.multipart.as_ref().unwrap().parts.len(), 5);
    assert_eq!(models(&d, &state("x")), ["minecraft:always"]);
    assert_eq!(models(&d, &state("x[east=true,north=true]")), ["minecraft:always", "minecraft:and"]);
    assert_eq!(models(&d, &state("x[east=false,north=true]")), ["minecraft:always"]);
    assert_eq!(models(&d, &state("x[facing=south]")), ["minecraft:always", "minecraft:set"]);
    assert_eq!(models(&d, &state("x[a=1]")), ["minecraft:always", "minecraft:or", "minecraft:num"]);
    assert_eq!(models(&d, &state("x[b=1,c=2]")), ["minecraft:always", "minecraft:or"]);
    assert_eq!(models(&d, &state("x[b=1]")), ["minecraft:always"]);
    assert_eq!(d.resolve(&state("x[a=1]")).count(), 3);
}

#[test]
fn multipart_value_splitting() {
    let d = def(
        r#"{"multipart": [{"when": {"a": "1|"}, "apply": {"model": "m"}}, {"when": {"a": ""}, "apply": {"model": "e"}}]}"#,
    );
    let parts = &d.multipart.as_ref().unwrap().parts;
    assert_eq!(parts[0].condition, Condition::Property("a".into(), "1".into()));
    assert_eq!(parts[1].condition, Condition::Property("a".into(), "".into()));
}

#[test]
fn whole_file_failures() {
    use ParseError::*;
    assert_eq!(def_err(r#"{"multipart": [{"when": {}, "apply": {"model": "m"}}]}"#), Empty("when"));
    assert_eq!(def_err(r#"{"multipart": [{"when": {"__comment": "c"}, "apply": {}}]}"#), Empty("when"));
    assert_eq!(def_err(r#"{"multipart": [{"when": {"a": "1"}}, {"when": {}}]}"#), Empty("when"));
    assert_eq!(def_err(r#"{"multipart": [{"when": {"OR": []}, "apply": {}}]}"#), Empty("OR"));
    assert_eq!(def_err(r#"{"multipart": [{"when": {"a": "|"}, "apply": {}}]}"#), Empty("when value"));
    assert!(matches!(def_err(r#"{"multipart": [{"when": {"a": null}, "apply": {}}]}"#), Type { .. }));
    assert!(matches!(def_err(r#"{"multipart": [null]}"#), Type { .. }));
    assert!(matches!(def_err(r#"{"variants": {"": null}}"#), Type { .. }));
    assert!(matches!(def_err(r#"{"variants": {"bogus": [{"model": "a"}, null]}}"#), Type { .. }));
    assert!(matches!(def_err(r#"{"variants": {"": {"renderer": null}}}"#), Type { .. }));
    assert!(matches!(def_err(r#"{"variants": {"": {"x": "ninety"}}}"#), Number { .. }));
    assert!(matches!(def_err(r#"{"variants": {"": {"uvlock": 1}}}"#), Type { .. }));
    assert!(matches!(def_err("[]"), Type { .. }));
}

#[test]
fn empty_and_null_sections() {
    let d = def(r#"{"variants": null, "multipart": null}"#);
    assert_eq!(d, BlockStateDef::default());
    assert_eq!(d.resolve(&state("x")).count(), 0);
    let empty_set = def(r#"{"variants": {"": []}}"#);
    assert_eq!(empty_set.resolve(&state("x")).count(), 1);
    assert_eq!(empty_set.variants_at(&state("x"), 0, 0, 0).count(), 0);
}

#[test]
fn variant_fields_and_coercions() {
    let d = def(r#"{"variants": {"": [
        {},
        {"model": "MyMod:Block/Thing", "x": 90, "y": "270", "z": 45.5, "uvlock": "TRUE", "weight": "2.5", "renderer": "liquid"},
        {"model": null, "x": null, "uvlock": false, "weight": 3, "renderer": "Mod:Custom"}
    ]}}"#);
    let set = d.variants.as_ref().unwrap().default.as_ref().unwrap();
    let [a, b, c] = &*set.variants else { panic!() };
    assert_eq!((a.model.as_str(), a.renderer.as_str()), ("bluemap:block/missing", "bluemap:default"));
    assert_eq!((a.x, a.y, a.z, a.uvlock, a.weight, a.transformed), (0.0, 0.0, 0.0, false, 1.0, false));
    assert_eq!(b.model.as_str(), "mymod:block/thing");
    assert_eq!((b.x, b.y, b.z, b.uvlock, b.weight, b.transformed), (90.0, 270.0, 45.5, true, 2.5, true));
    assert_eq!(RendererType::from_key(&b.renderer), Some(RendererType::Liquid));
    assert_eq!((c.model.as_str(), c.x, c.weight), ("bluemap:block/missing", 0.0, 3.0));
    assert_eq!(c.renderer.as_str(), "Mod:Custom");
    assert_eq!(RendererType::from_key(&c.renderer), None);
    assert_eq!(set.total_weight(), 6.5);
    assert_eq!(ResourcePath::key(RendererType::Missing.key()), ResourcePath::key_in("missing", "bluemap"));
}

#[test]
fn all_variants_order() {
    let d = def(
        r#"{"variants": {"": {"model": "d"}, "a=1": {"model": "a"}}, "multipart": [{"apply": [{"model": "m1"}, {"model": "m2"}]}]}"#,
    );
    let all: Vec<_> = d.all_variants().map(|v| v.model.as_str()).collect();
    assert_eq!(all, ["minecraft:a", "minecraft:d", "minecraft:m1", "minecraft:m2"]);
    assert_eq!(models(&d, &state("x[a=1]")), ["minecraft:a", "minecraft:m1"]);
}

#[test]
fn hash_and_weighted_pick_match_java() {
    for &(x, y, z, bits, picks) in PICKS {
        assert_eq!(hash_to_float(x, y, z).to_bits(), bits, "hash at {x} {y} {z}");
        for (weights, &expected) in WEIGHTS.iter().zip(&picks) {
            let variants =
                weights.iter().map(|&w| Variant::new(ResourcePath::key("m"), 0.0, 0.0, 0.0, false, w)).collect();
            let set = VariantSet::new(Condition::All, variants);
            let got = set.pick(x, y, z).map(|v| set.variants.iter().position(|o| std::ptr::eq(o, v)).unwrap() as i8);
            assert_eq!(got.unwrap_or(-1), expected, "{weights:?} at {x} {y} {z}");
        }
    }
}

#[test]
fn transform_matrices_match_java() {
    for &(x, y, z, bits) in MATRICES {
        let v = Variant::new(ResourcePath::key("m"), x, y, z, false, 1.0);
        assert_eq!(v.transform.to_array().map(f32::to_bits), bits, "rotation {x} {y} {z}");
    }
}
