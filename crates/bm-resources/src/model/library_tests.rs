use std::io::{Cursor, Write};
use std::sync::Arc;

use zip::write::SimpleFileOptions;

use super::*;
use crate::Pack;

fn key(s: &str) -> ResourcePath {
    ResourcePath::key(s)
}

fn library(models: &[(&str, &str)]) -> ModelLibrary {
    let mut lib = ModelLibrary::new();
    for (k, src) in models {
        assert!(lib.insert(key(k), Model::parse(src).unwrap().unwrap()));
    }
    lib
}

fn zip_pack(files: &[(&str, &str)]) -> Pack {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, SimpleFileOptions::default()).unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    Pack::zip(w.finish().unwrap().into_inner().into(), Arc::from("test.zip")).unwrap()
}

const CUBE: &str = r##"{"elements": [{"faces": {
    "down": {"texture": "#down", "cullface": "down"}, "up": {"texture": "#up", "cullface": "up", "tintindex": 2},
    "north": {"texture": "#north"}, "south": {"texture": "#south"}, "west": {"texture": "#west"}, "east": {"texture": "#east"}}}]}"##;
const CUBE_ALL: &str = r##"{"parent": "block/cube", "textures": {"particle": "#all", "down": "#all", "up": "#all",
    "north": "#all", "south": "#all", "west": "#all", "east": "#all"}}"##;

#[test]
fn parent_merge_copies_absent_textures_ao_and_elements() {
    let mut lib = library(&[
        ("minecraft:block/cube", CUBE),
        ("minecraft:block/cube_all", CUBE_ALL),
        (
            "minecraft:block/stone",
            r#"{"parent": "Block/Cube_All", "textures": {"all": "block/stone", "up": "block/stone_top"}}"#,
        ),
        ("minecraft:block/flat", r#"{"parent": "block/stone", "ambientocclusion": false, "elements": []}"#),
        ("minecraft:block/flat_child", r#"{"parent": "block/flat"}"#),
        ("minecraft:item/thing", r#"{"parent": "builtin/generated"}"#),
    ]);
    let missing = lib.merge_parents();
    assert_eq!(missing, vec![(key("minecraft:item/thing"), key("minecraft:builtin/generated"))]);
    let stone = lib.get(&key("minecraft:block/stone")).unwrap();
    assert_eq!(stone.parent, None);
    assert_eq!(stone.textures["up"], TextureVariable::Path(key("minecraft:block/stone_top")), "child keeps its own");
    assert_eq!(stone.textures["down"], TextureVariable::Reference("all".into()));
    assert_eq!(stone.elements.as_ref().unwrap().len(), 1);
    let flat = lib.get(&key("minecraft:block/flat")).unwrap();
    assert_eq!(flat.elements, Some(vec![]), "an explicit empty list blocks the parent's elements");
    let child = lib.get(&key("minecraft:block/flat_child")).unwrap();
    assert_eq!(child.ambientocclusion, Some(false));
    assert_eq!(child.elements, Some(vec![]));
    assert!(lib.get(&key("minecraft:block/cube_all")).unwrap().ambient_occlusion());
}

#[test]
fn parent_cycles_terminate() {
    let mut lib = library(&[
        ("minecraft:a", r#"{"parent": "b", "textures": {"x": "block/a"}}"#),
        ("minecraft:b", r#"{"parent": "a", "textures": {"y": "block/b"}}"#),
        ("minecraft:self", r#"{"parent": "self", "textures": {"z": "block/s"}}"#),
    ]);
    assert!(lib.merge_parents().is_empty());
    // sorted order: a merges b, which first merged the not-yet-merged a
    assert_eq!(lib.get(&key("minecraft:a")).unwrap().textures.len(), 2);
    assert_eq!(lib.get(&key("minecraft:b")).unwrap().textures.len(), 2);
    assert_eq!(lib.get(&key("minecraft:self")).unwrap().textures.len(), 1);
}

#[test]
fn references_resolve_against_the_merged_child_map() {
    let lib = library(&[
        ("minecraft:block/cube", CUBE),
        ("minecraft:block/cube_all", CUBE_ALL),
        (
            "minecraft:block/grass",
            r##"{"parent": "block/cube_all", "textures": {"all": "block/dirt", "up": "#top", "top": "Block/Grass_Top"}}"##,
        ),
        (
            "minecraft:block/loop",
            r##"{"parent": "block/cube_all", "textures": {"all": "#up", "up": "#all", "east": "block/x", "west": "#nothing"}}"##,
        ),
    ]);
    let baked = lib.bake(&|_| Some(1.0));
    let tex = |m: &str, d: Direction| baked.get(&key(m)).unwrap().elements[0].face(d).unwrap().texture.clone();
    assert_eq!(tex("minecraft:block/grass", Direction::Up), Some(key("minecraft:block/grass_top")));
    assert_eq!(tex("minecraft:block/grass", Direction::North), Some(key("minecraft:block/dirt")));
    assert_eq!(tex("minecraft:block/loop", Direction::Up), None);
    assert_eq!(tex("minecraft:block/loop", Direction::North), None);
    assert_eq!(tex("minecraft:block/loop", Direction::East), Some(key("minecraft:block/x")));
    assert_eq!(tex("minecraft:block/loop", Direction::West), None);
    assert_eq!(tex("minecraft:block/cube", Direction::Up), None);
    let mut unresolved: Vec<_> =
        baked.unresolved_references.iter().filter(|(m, _)| m.path() == "block/loop").map(|(_, n)| n.as_str()).collect();
    unresolved.sort_unstable();
    assert_eq!(unresolved, ["down", "north", "south", "up", "west"]);

    let face = baked.get(&key("minecraft:block/grass")).unwrap().elements[0].face(Direction::Up).unwrap().clone();
    assert_eq!((face.cullface, face.tinted, face.uv), (Some(Direction::Up), true, [0.0, 0.0, 16.0, 16.0]));
}

#[test]
fn reference_chain_through_missing_and_null() {
    let textures = [
        ("a".to_owned(), TextureVariable::Reference("b".into())),
        ("b".to_owned(), TextureVariable::Reference("c".into())),
        ("c".to_owned(), TextureVariable::Path(key("minecraft:block/c"))),
        ("n".to_owned(), TextureVariable::Null),
        ("m".to_owned(), TextureVariable::Reference("n".into())),
    ]
    .into_iter()
    .collect();
    assert_eq!(TextureVariable::Reference("a".into()).resolve(&textures), Some(&key("minecraft:block/c")));
    assert_eq!(TextureVariable::Reference("m".into()).resolve(&textures), None);
    assert_eq!(TextureVariable::Reference("zz".into()).resolve(&textures), None);
    assert_eq!(TextureVariable::Reference("a".into()).resolve(&Default::default()), None);
}

#[test]
fn properties_use_the_first_full_cube_and_opaque_textures() {
    let half = r##"{"elements": [{"to": [16, 8, 16], "faces": {"up": {"texture": "#t"}}}, {"faces": {
        "down": {"texture": "#t"}, "up": {"texture": "#t"}, "north": {"texture": "#t"}, "south": {"texture": "#t"},
        "west": {"texture": "#t"}, "east": {"texture": "#t"}}}], "textures": {"t": "block/t"}}"##;
    let (glass, unloaded) = (half.replace("block/t", "block/glass"), half.replace("block/t", "block/nope"));
    let lib = library(&[
        ("minecraft:opaque", half),
        ("minecraft:glass", &glass),
        ("minecraft:unloaded", &unloaded),
        ("minecraft:partial", r#"{"elements": [{"faces": {"up": {}}}]}"#),
        ("minecraft:none", "{}"),
    ]);
    let alpha = |p: &ResourcePath| match p.path() {
        "block/t" => Some(1.0),
        "block/glass" => Some(0.5),
        _ => None,
    };
    let baked = lib.bake(&alpha);
    let props = |m: &str| {
        let b = baked.get(&key(m)).unwrap();
        (b.occluding, b.culling)
    };
    assert_eq!(props("minecraft:opaque"), (true, true));
    assert_eq!(props("minecraft:glass"), (true, false));
    assert_eq!(props("minecraft:unloaded"), (true, false));
    assert_eq!(props("minecraft:partial"), (false, false));
    assert_eq!(props("minecraft:none"), (false, false));
}

#[test]
fn used_texture_keys_skip_references_and_face_paths() {
    let lib = library(&[
        (
            "minecraft:a",
            r##"{"textures": {"x": "block/a", "y": "#x", "z": "plain"}, "elements": [{"faces": {"up": {"texture": "block/direct"}}}]}"##,
        ),
        ("minecraft:item/b", r#"{"textures": {"layer0": {"sprite": "Item/B"}}}"#),
    ]);
    let mut keys: Vec<_> = lib.collect_used_texture_keys().into_iter().map(|k| k.to_string()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["bluemap:block/missing", "minecraft:block/a", "minecraft:item/b"]);
}

#[test]
fn packs_load_first_wins_and_failures_free_the_key() {
    let mut lib = ModelLibrary::new();
    lib.load_pack(&zip_pack(&[
        ("assets/minecraft/models/block/a.json", r#"{"textures": {"x": "block/first"}}"#),
        ("assets/minecraft/models/block/broken.json", r#"{"elements": 5}"#),
        ("assets/minecraft/models/block/empty.json", ""),
        ("assets/mymod/models/Block/Upper.json", "{}"),
        ("assets/minecraft/models/block/readme.txt", "x"),
    ]));
    lib.load_pack(&zip_pack(&[
        ("assets/minecraft/models/block/a.json", r#"{"textures": {"x": "block/second"}}"#),
        ("assets/minecraft/models/block/broken.json", "{}"),
        ("assets/minecraft/models/block/empty.json", "{}"),
    ]));
    assert_eq!(lib.len(), 4);
    assert_eq!(
        lib.get(&key("minecraft:block/a")).unwrap().textures["x"],
        TextureVariable::Path(key("minecraft:block/first"))
    );
    assert_eq!(lib.get(&key("minecraft:block/broken")), Some(&Model::default()));
    assert_eq!(lib.get(&key("minecraft:block/empty")), Some(&Model::default()));
    assert!(lib.get(&key("mymod:Block/Upper")).is_some(), "file keys keep their case");
    assert_eq!(lib.failures().len(), 1);
    assert_eq!(lib.failures()[0].0, key("minecraft:block/broken"));
}

#[test]
fn uv_rotation_steps_floor() {
    let face = |rotation| BakedFace { uv: [0.0; 4], texture: None, cullface: None, rotation, tinted: false };
    assert_eq!([0, 90, 180, 270, 360, -90, 45, -1].map(|r| face(r).uv_rotation_steps()), [0, 1, 2, 3, 0, 3, 0, 3]);
}
