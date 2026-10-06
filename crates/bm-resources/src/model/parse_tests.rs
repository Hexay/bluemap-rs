use bm_math::{Axis, MatrixM4f};
use serde_json::json;

use super::java_ref::ROTATIONS;
use super::*;

fn model(src: &str) -> Model {
    Model::parse(src).unwrap().unwrap()
}

fn element(v: serde_json::Value) -> Element {
    Element::from_value(&v).unwrap()
}

fn bits(m: &MatrixM4f) -> [u32; 16] {
    m.to_array().map(f32::to_bits)
}

#[test]
fn texture_variable_forms() {
    let path = |s| TextureVariable::Path(ResourcePath::key(s));
    let reference = |s: &str| TextureVariable::Reference(s.to_owned());
    assert_eq!(TextureVariable::parse("#all").unwrap(), reference("all"));
    // quirk: neither ':' nor '/' makes it a reference
    assert_eq!(TextureVariable::parse("particle").unwrap(), reference("particle"));
    assert_eq!(TextureVariable::parse("Block/Stone").unwrap(), path("minecraft:block/stone"));
    assert_eq!(TextureVariable::parse("Mod:Stone").unwrap(), path("mod:stone"));
    assert!(matches!(TextureVariable::parse(""), Err(ModelError::EmptyTexture)));
    let v = |v| TextureVariable::from_value(&v);
    assert_eq!(v(json!({"sprite": "block/dirt", "force_translucent": true})).unwrap(), path("minecraft:block/dirt"));
    assert_eq!(v(json!({"sprite": "#side"})).unwrap(), reference("side"));
    assert!(matches!(v(json!({"other": 1})), Err(ModelError::NoSprite)));
    assert!(v(json!(5)).is_err());
    assert_eq!(v(json!(null)).unwrap(), TextureVariable::Null);
}

#[test]
fn model_fields_and_defaults() {
    let m = model(
        r##"{ "parent": "Block/Cube_All", "ambientocclusion": "false", "display": {"gui": {}},
            textures: {all: "block/stone", 'side': "#all", "particle": {"sprite": "#side"}},
            "elements": [ null, { "faces": { "Top": {}, "bottom": {"texture": "#all", "cullface": "TOP"} } } ] }"##,
    );
    assert_eq!(m.parent, Some(ResourcePath::key("minecraft:block/cube_all")));
    assert_eq!(m.ambientocclusion, Some(false));
    assert!(!m.ambient_occlusion());
    assert_eq!(m.textures.len(), 3);
    let elements = m.elements.as_ref().unwrap();
    assert_eq!(elements.len(), 1, "null slots are dropped");
    let e = &elements[0];
    assert_eq!((e.from, e.to, e.shade, e.light_emission), ([0.0; 3], [16.0; 3], true, 0));
    assert_eq!(e.rotation, Rotation::ZERO);
    let up = e.face(Direction::Up).unwrap();
    assert_eq!(up.texture, TextureVariable::Path(missing_texture()));
    assert_eq!((up.cullface, up.rotation, up.tintindex), (None, 0, -1));
    assert_eq!(e.face(Direction::Down).unwrap().cullface, Some(Direction::Up));
    assert!(e.face(Direction::North).is_none());

    let empty = model("{}");
    assert_eq!(empty, Model::default());
    assert!(empty.ambient_occlusion());
    assert_eq!(Model::parse("  \n").unwrap(), None);
    assert_eq!(Model::parse("null").unwrap(), None);
    assert_eq!(model(r#"{"elements": []}"#).elements, Some(vec![]));
    assert_eq!(model(r#"{"elements": null, "textures": null, "parent": null}"#), Model::default());
}

#[test]
fn gson_coercions() {
    let e = element(json!({"from": ["1", 2.5, 3], "to": [4, 5, "6.5"], "shade": "TRUE", "light_emission": "15",
        "faces": {"north": {"rotation": 90.0, "tintindex": "0", "uv": [0, 0, 16, 16]}}}));
    assert_eq!((e.from, e.to, e.shade, e.light_emission), ([1.0, 2.5, 3.0], [4.0, 5.0, 6.5], true, 15));
    let n = e.face(Direction::North).unwrap();
    assert_eq!((n.rotation, n.tintindex, n.uv), (90, 0, [0.0, 0.0, 16.0, 16.0]));
    assert!(!element(json!({"shade": "yes"})).shade);
    assert!(element(json!({"shade": null})).shade, "null keeps a primitive's default");

    let bad = |v| Element::from_value(&v).is_err();
    assert!(bad(json!({"from": [1, 2]})));
    assert!(bad(json!({"from": null})), "vector adapters are not null-safe");
    assert!(bad(json!({"faces": {"north": {"rotation": 1.5}}})));
    assert!(bad(json!({"faces": {"north": {"cullface": null}}})));
    assert!(bad(json!({"faces": {"sideways": {}}})));
    assert!(bad(json!({"faces": {"up": {}, "top": {}}})), "Gson rejects a duplicate map key");
    assert!(bad(json!({"faces": {"up": null}})), "Element.init dereferences every face");
    assert!(bad(json!({"faces": null})));
    assert!(bad(json!({"shade": 1})));
    assert!(Model::parse(r#"{"elements": {}}"#).is_err());
    assert!(Model::parse(r#"{"parent": true}"#).is_err());
}

#[test]
fn default_uvs_follow_element_bounds() {
    let e = element(json!({"from": [1, 2, 3], "to": [10, 12, 14],
        "faces": {"up": {}, "down": {}, "north": {}, "south": {}, "east": {}, "west": {}}}));
    let uv = |d| e.face(d).unwrap().uv;
    assert_eq!(uv(Direction::Up), [1.0, 3.0, 10.0, 14.0]);
    assert_eq!(uv(Direction::Down), [1.0, 2.0, 10.0, 13.0]);
    assert_eq!(uv(Direction::North), [6.0, 4.0, 15.0, 14.0]);
    assert_eq!(uv(Direction::South), [1.0, 4.0, 10.0, 14.0]);
    assert_eq!(uv(Direction::East), [2.0, 4.0, 13.0, 14.0]);
    assert_eq!(uv(Direction::West), [3.0, 4.0, 14.0, 14.0]);
}

#[test]
fn full_cube_needs_exact_bounds_and_six_faces() {
    let all = json!({"up": {}, "down": {}, "north": {}, "south": {}, "east": {}, "west": {}});
    assert!(element(json!({"faces": all})).is_full_cube());
    assert!(!element(json!({"faces": {"up": {}}})).is_full_cube());
    assert!(!element(json!({"from": [0, 0, 0.01], "faces": all})).is_full_cube());
    assert!(!element(json!({"from": [0, 0, -0.0], "faces": all})).is_full_cube(), "Float.compare: -0 != 0");
}

#[test]
fn rotation_matrices_match_java() {
    for r in ROTATIONS {
        let rot = Rotation::from_xyz(r.origin, r.xyz, r.rescale);
        assert_eq!(bits(rot.matrix()), r.matrix, "{:?} {:?} rescale {}", r.origin, r.xyz, r.rescale);
        let e = Element::new([0.0; 3], [16.0; 3], rot, Default::default());
        let baked = super::baked::bake_model(
            &Model { elements: Some(vec![e]), ..Model::default() },
            &|_| None,
            &mut Vec::new(),
        );
        assert_eq!(bits(&baked.elements[0].transform), r.transform);
    }
}

#[test]
fn axis_angle_overrides_per_axis_angles() {
    let r = Rotation::from_value(&json!({"origin": [8, 4.5, 3.25], "axis": "Z", "angle": -22.5, "x": 30, "y": 10}))
        .unwrap();
    assert_eq!((r.x, r.y, r.z), (0.0, 0.0, -22.5));
    assert_eq!(bits(r.matrix()), ROTATIONS[4].matrix);
    let r = Rotation::from_value(&json!({"x": 22.5, "y": 45})).unwrap();
    assert_eq!(bits(r.matrix()), ROTATIONS[6].matrix, "origin defaults to 8,8,8");
    let r = Rotation::from_value(&json!({"axis": "y", "angle": 45, "rescale": true})).unwrap();
    assert_eq!(bits(r.matrix()), ROTATIONS[2].matrix);
    assert_eq!(Rotation::from_axis([8.0; 3], Axis::Y, 0.0, true).matrix(), &MatrixM4f::IDENTITY);
    assert!(Rotation::from_value(&json!({"axis": "w", "angle": 45})).is_err());
    assert!(Rotation::from_value(&json!({"axis": null})).is_err());
    let e = element(json!({"rotation": null}));
    assert_eq!(e.rotation, Rotation::ZERO);
}

#[test]
fn directions() {
    assert_eq!(Direction::parse("BOTTOM").unwrap(), Direction::Down);
    assert_eq!(Direction::parse("Top").unwrap(), Direction::Up);
    assert_eq!(Direction::parse("eAsT").unwrap(), Direction::East);
    assert!(Direction::parse("").is_err());
    for d in Direction::ALL {
        let [x, y, z] = d.to_vector();
        assert_eq!(d.opposite().to_vector(), [-x, -y, -z]);
    }
}
