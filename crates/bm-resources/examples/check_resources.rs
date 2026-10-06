//! Loads a client jar plus BlueMap's resource extensions the way BlueMap orders them, then resolves every state of
//! a vanilla `blocks.json` report to variants, models and textures, reporting anything that doesn't resolve.
//! Usage: cargo run -p bm-resources --release --example check_resources -- <client.jar> <blocks.json> [extensions dir]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use bm_resources::packs::load_order;
use bm_resources::resource_pack::ResourcePack;
use bm_resources::{PackVersions, ResourcePath};
use bm_world::BlockStates;

fn main() {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let [jar, report, rest @ ..] = args.as_slice() else {
        eprintln!("usage: check_resources <client.jar> <blocks.json> [extensions dir]");
        std::process::exit(2);
    };
    let extensions = rest.first().cloned().unwrap_or_else(|| PathBuf::from("assets/resourceExtensions"));
    let versions = PackVersions::read(jar).unwrap();
    let roots = [extensions, jar.clone()];

    let start = Instant::now();
    let resources = load_order(&roots, versions.resource);
    let data = load_order(&roots, versions.data);
    let pack = ResourcePack::load(&resources, &data);
    println!(
        "loaded in {:.2}s: {} packs, {} blockstates, {} models, {} textures, {} biomes, {} failures",
        start.elapsed().as_secs_f64(),
        resources.len(),
        pack.blockstate_count(),
        pack.models.models.len(),
        pack.textures.len(),
        pack.datapack.biome_count(),
        pack.failures.len()
    );
    pack.failures.iter().take(10).for_each(|f| println!("  failure: {f}"));

    let states = BlockStates::default();
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    let mut issues: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut total = 0;
    for (name, block) in report.as_object().unwrap() {
        for s in block["states"].as_array().unwrap() {
            total += 1;
            let mut props: Vec<(&str, &str)> = s
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|p| p.iter().map(|(k, v)| (k.as_str(), v.as_str().unwrap())).collect())
                .unwrap_or_default();
            let state = states.get(states.intern(name, &mut props));
            if pack.blockstate(&state).is_none() {
                issues.entry("no blockstate file").or_default().push(state.key.to_string());
                continue;
            }
            let variants: Vec<_> = pack.variants(&state, 0, 0, 0).collect();
            if variants.is_empty() {
                issues.entry("no matching variant").or_default().push(state.key.to_string());
            }
            for v in variants {
                let Some(model) = pack.model(&v.model) else {
                    issues.entry("missing model").or_default().push(format!("{} -> {}", state.key, v.model));
                    continue;
                };
                let faces = model.elements.iter().flat_map(|e| e.faces.iter().flatten());
                for tex in faces.map(|f| f.texture.clone()) {
                    if tex.as_ref().is_none_or(|t| !pack.textures.contains_key(t)) {
                        let tex = tex.as_ref().map_or("<unresolved>", ResourcePath::as_str).to_owned();
                        issues.entry("missing texture").or_default().push(format!("{} -> {tex}", state.key));
                    }
                }
            }
        }
    }
    println!("{total} report states checked");
    for (kind, list) in &issues {
        let mut unique = list.clone();
        unique.sort();
        unique.dedup();
        println!("{kind}: {} ({} unique)", list.len(), unique.len());
        unique.iter().take(6).for_each(|i| println!("  {i}"));
    }
    for name in ["minecraft:stone", "minecraft:glass", "minecraft:oak_leaves", "minecraft:seagrass"] {
        let p = pack.block_properties(&states.get(states.default_state(name)));
        println!(
            "{name}: culling {} occluding {} waterlogged {} cullingIdentical {}",
            p.is_culling(),
            p.is_occluding(),
            p.is_always_waterlogged(),
            p.is_culling_identical()
        );
    }
}
