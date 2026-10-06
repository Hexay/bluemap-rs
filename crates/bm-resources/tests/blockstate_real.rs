//! Parses every blockstate of the real client jar and BlueMap's resource extensions, then resolves every state
//! of the vanilla block report. Needs `work/` data; paths overridable via `BM_CLIENT_JAR`, `BM_BLOCKS_REPORT`,
//! `BM_RESOURCE_EXTENSIONS`. Run: `cargo test -p bm-resources --test blockstate_real -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bm_resources::Pack;
use bm_resources::blockstate::BlockStateDef;
use bm_resources::json;
use bm_world::BlockStates;
use serde_json::Value;

fn path(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(default), PathBuf::from)
}

/// Every `assets/<ns>/blockstates/**.json` under `pack` (any subfolder root, for the extension overlays), keyed
/// `ns:name` per root.
fn load(pack: &Pack, failures: &mut Vec<String>) -> BTreeMap<String, BlockStateDef> {
    let mut defs = BTreeMap::new();
    for file in pack.walk("") {
        let segs: Vec<&str> = file.split('/').collect();
        let Some(i) = segs.iter().position(|s| *s == "assets") else { continue };
        if segs.len() < i + 4 || segs[i + 2] != "blockstates" || !file.ends_with(".json") {
            continue;
        }
        let src = pack.read_string(&file).unwrap();
        let parsed = json::parse(&src).map_err(|e| e.to_string());
        match parsed.and_then(|v| BlockStateDef::from_json(&v).map_err(|e| e.to_string())) {
            Ok(def) => {
                let name = segs[i + 3..].join("/");
                defs.insert(format!("{}{}:{}", segs[..i].join("/"), segs[i + 1], name.trim_end_matches(".json")), def);
            }
            Err(e) => failures.push(format!("{file}: {e}")),
        }
    }
    defs
}

#[test]
#[ignore = "needs the 26.3 client jar and block report under work/"]
fn real_blockstates_parse_and_resolve() {
    let jar = path("BM_CLIENT_JAR", "work/bluemap/vanilla/data/minecraft-client-26.3.jar");
    let report = path("BM_BLOCKS_REPORT", "work/data/reports-26.3/reports/blocks.json");
    let extensions = path("BM_RESOURCE_EXTENSIONS", "assets/resourceExtensions");

    let mut failures = Vec::new();
    let vanilla = load(&Pack::open(&jar).expect("client jar"), &mut failures);
    let vanilla_failures = failures.len();
    let ext = load(&Pack::open(&extensions).expect("resource extensions"), &mut failures);
    println!("vanilla: {} parsed, {vanilla_failures} failed", vanilla.len());
    println!("resourceExtensions: {} parsed, {} failed", ext.len(), failures.len() - vanilla_failures);
    for f in &failures {
        println!("  FAIL {f}");
    }

    let report: BTreeMap<String, Value> = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    let registry = BlockStates::default();
    let (mut states, mut no_file, mut unresolved, mut empty_pick) = (0, Vec::new(), Vec::new(), 0);
    for (block, info) in &report {
        let def = vanilla.get(block);
        if def.is_none() {
            no_file.push(block.clone());
        }
        for s in info["states"].as_array().unwrap() {
            states += 1;
            let mut props: Vec<(&str, &str)> = s
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
                .collect();
            let state = registry.get(registry.intern(block, &mut props));
            let Some(def) = def else { continue };
            if def.resolve(&state).next().is_none() {
                unresolved.push(state.key.to_string());
            } else if def.variants_at(&state, 0, 0, 0).next().is_none() {
                empty_pick += 1;
            }
        }
    }
    println!("block report: {} blocks, {states} states", report.len());
    println!("blocks without a vanilla blockstate file: {} {:?}", no_file.len(), no_file);
    println!(
        "states with no matching variant: {} {:?}",
        unresolved.len(),
        unresolved.iter().take(20).collect::<Vec<_>>()
    );
    println!("states resolving to sets but no picked variant: {empty_pick}");
    assert!(failures.is_empty(), "{} blockstate files failed", failures.len());
}
