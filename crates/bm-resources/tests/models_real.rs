//! Loads every model of the real 26.3 client jar plus BlueMap's resource extensions, merges parents and resolves
//! textures. Needs the local jar: `cargo test -p bm-resources --test models_real -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::Path;

use bm_resources::Pack;
use bm_resources::model::ModelLibrary;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[test]
#[ignore = "needs work/bluemap/vanilla/data/minecraft-client-26.3.jar"]
fn every_vanilla_model_loads_and_resolves() {
    let root = Path::new(ROOT);
    // the main checkout holds work/; worktrees share assets/ but not work/
    let work = ["work", "../../../work"].iter().map(|w| root.join(w)).find(|w| w.is_dir()).expect("work/ folder");
    let packs = [
        Pack::open(&root.join("assets/resourceExtensions")).unwrap(),
        Pack::open(&work.join("bluemap/vanilla/data/minecraft-client-26.3.jar")).unwrap(),
    ];
    let mut lib = ModelLibrary::new();
    for pack in &packs {
        lib.load_pack(pack);
    }
    let texture_exists = |p: &bm_resources::ResourcePath| {
        packs.iter().any(|pack| pack.exists(&format!("assets/{}/textures/{}.png", p.namespace(), p.path())))
    };
    let used = lib.collect_used_texture_keys();
    let used_missing = used.iter().filter(|k| !texture_exists(k)).count();
    let (models, failures) = (lib.len(), lib.failures().len());
    for (key, err) in lib.failures() {
        println!("failed: {key}: {err}");
    }

    let baked = lib.bake(&|p| texture_exists(p).then_some(1.0));
    let mut missing_parents: BTreeMap<String, usize> = BTreeMap::new();
    for (_, parent) in &baked.missing_parents {
        *missing_parents.entry(parent.to_string()).or_default() += 1;
    }
    let mut unresolved_models: Vec<_> = baked.unresolved_references.iter().map(|(m, _)| m.to_string()).collect();
    unresolved_models.dedup();
    let faces: usize = baked.models.values().flat_map(|m| &m.elements).map(|e| e.faces.iter().flatten().count()).sum();
    let face_textures_missing = baked
        .models
        .values()
        .flat_map(|m| &m.elements)
        .flat_map(|e| e.faces.iter().flatten())
        .filter(|f| f.texture.as_ref().is_some_and(|t| !texture_exists(t)))
        .count();
    let block_models = baked.models.keys().filter(|k| k.path().starts_with("block/")).count();
    let occluding = baked.models.values().filter(|m| m.occluding).count();
    let culling = baked.models.values().filter(|m| m.culling).count();

    println!("models: {models} ({block_models} block/), parse failures: {failures}, faces: {faces}");
    println!("used texture keys: {} ({used_missing} without a png)", used.len());
    println!("missing parents: {missing_parents:?}");
    println!("unresolved face references: {} in {} models", baked.unresolved_references.len(), unresolved_models.len());
    let concrete: Vec<_> = unresolved_models.iter().filter(|m| !m.contains("template")).collect();
    println!("  of which not named template*: {} {:?}", concrete.len(), &concrete[..concrete.len().min(25)]);
    let mut no_png: Vec<String> = baked
        .models
        .values()
        .flat_map(|m| &m.elements)
        .flat_map(|e| e.faces.iter().flatten())
        .filter_map(|f| f.texture.as_ref().filter(|t| !texture_exists(t)).map(|t| t.to_string()))
        .collect();
    no_png.sort_unstable();
    no_png.dedup();
    println!(
        "resolved face textures without a png: {face_textures_missing} ({} distinct) {:?}",
        no_png.len(),
        &no_png[..no_png.len().min(15)]
    );
    println!("occluding: {occluding}, culling (all pngs counted opaque): {culling}");

    assert_eq!(failures, 0);
    assert!(block_models > 1000);
    let stone = baked.get(&bm_resources::ResourcePath::key("minecraft:block/stone")).unwrap();
    assert!(stone.occluding && stone.culling);
}
