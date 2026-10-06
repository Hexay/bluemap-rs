use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;

use super::*;

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, SimpleFileOptions::default()).unwrap();
        w.write_all(body).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bm-packs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_tree(root: &Path, files: &[(&str, Vec<u8>)]) {
    for (name, body) in files {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
}

/// `<nested file or "-">:<folder inside it>` per pack.
fn describe(packs: &[Pack]) -> Vec<String> {
    packs.iter().map(|p| format!("{}:{}", p.origin.rsplit_once("!/").map_or("-", |(_, f)| f), p.root())).collect()
}

const META: &str = r#"{"overlays": {"entries": [
    {"formats": {"min_inclusive": 5}, "directory": "ov_a"},
    {"formats": {"max_inclusive": 4}, "directory": "ov_old"},
    {"formats": [5, 99], "directory": "ov_b"},
    {"directory": "ov_missing"},
    {"directory": ""}
]}}"#;

const FABRIC: &str = r#"{"jars": [
    {"file": "META-INF/jars/inner.jar"}, {"file": "missing.jar"}, {"nofile": 1}, {"file": "META-INF/jars/skipped.jar"}
]}"#;

fn layered_files() -> Vec<(&'static str, Vec<u8>)> {
    let inner = zip_bytes(&[("pack.mcmeta", b"{}")]);
    vec![
        ("fabric.mod.json", FABRIC.into()),
        ("META-INF/jars/inner.jar", inner.clone()),
        ("META-INF/jars/skipped.jar", inner.clone()),
        ("pack.mcmeta", META.into()),
        ("data/ns/datapacks/dp_folder/pack.mcmeta", b"{}".to_vec()),
        ("data/ns/datapacks/dp_zip.zip", inner),
        ("data/ns/worldgen/biome/x.json", b"{}".to_vec()),
        ("ov_a/assets/x.json", b"{}".to_vec()),
        ("ov_b/assets/x.json", b"{}".to_vec()),
        ("ov_old/assets/x.json", b"{}".to_vec()),
    ]
}

const LAYERED_ORDER: [&str; 6] = [
    "META-INF/jars/inner.jar:",
    "-:data/ns/datapacks/dp_folder/",
    "data/ns/datapacks/dp_zip.zip:",
    "-:ov_b/",
    "-:ov_a/",
    "-:",
];

#[test]
fn zip_root_layers_nested_jars_datapacks_and_overlays() {
    let files = layered_files();
    let dir = temp("zip");
    let jar = dir.join("mod.jar");
    let entries: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    std::fs::write(&jar, zip_bytes(&entries)).unwrap();
    assert_eq!(describe(&expand_root(&jar, PackVersion::new(10, 0))), LAYERED_ORDER);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn folder_root_layers_like_a_zip() {
    let dir = temp("folder");
    write_tree(&dir, &layered_files());
    assert_eq!(describe(&expand_root(&dir, PackVersion::new(10, 0))), LAYERED_ORDER);
    let old = describe(&expand_root(&dir, PackVersion::new(4, 0)));
    assert_eq!(&old[3..], ["-:ov_old/", "-:"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unreadable_roots_are_skipped() {
    let dir = temp("bad");
    std::fs::write(dir.join("broken.zip"), b"not a zip").unwrap();
    write_tree(&dir, &[("p/pack.mcmeta", b"{ broken".to_vec()), ("p/ov/x", b"".to_vec())]);
    assert!(expand_root(&dir.join("broken.zip"), PackVersion::new(1, 0)).is_empty());
    assert!(expand_root(&dir.join("nope"), PackVersion::new(1, 0)).is_empty());
    assert_eq!(describe(&expand_root(&dir.join("p"), PackVersion::new(1, 0))), ["-:"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn self_referencing_overlays_terminate() {
    let dir = temp("cycle");
    write_tree(&dir, &[("pack.mcmeta", br#"{"overlays": {"entries": [{"directory": "."}]}}"#.to_vec())]);
    let packs = expand_root(&dir, PackVersion::new(1, 0));
    assert_eq!(packs.len(), MAX_DEPTH as usize + 1);
    std::fs::remove_dir_all(dir).unwrap();
}

fn extensions_overlays(version: PackVersion) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/resourceExtensions");
    expand_root(&root, version).iter().map(|p| p.root().trim_end_matches('/').to_owned()).collect()
}

#[test]
fn resource_extensions_overlays_follow_pack_format() {
    assert_eq!(
        extensions_overlays(PackVersion::new(97, 1)),
        ["mc26_1", "mc1_21_9", "mc1_20_3", "mc1_17", "mc1_15", ""]
    );
    assert_eq!(
        extensions_overlays(PackVersion::new(80, 0)),
        ["signs", "beds", "mc26_1", "mc1_21_9", "mc1_20_3", "mc1_17", "mc1_15", ""]
    );
    assert_eq!(
        extensions_overlays(PackVersion::new(86, 0)),
        ["signs", "mc26_1", "mc1_21_9", "mc1_20_3", "mc1_17", "mc1_15", ""]
    );
    assert_eq!(extensions_overlays(PackVersion::new(4, 0)), ["signs", "beds", ""]);
}

#[test]
fn pack_roots_order() {
    let dir = temp("roots");
    let (packs, mods, data) = (dir.join("packs"), dir.join("mods"), dir.join("data"));
    write_tree(&packs, &[("a.zip", vec![]), ("c.zip", vec![]), ("b/pack.mcmeta", vec![])]);
    write_tree(&mods, &[("m2.jar", vec![]), ("m1.jar", vec![]), ("notes.txt", vec![]), ("dir.jar/x", vec![])]);
    write_tree(&data, &[("defaultBlockstates.zip", vec![])]);
    let mut config = PackRootsConfig {
        packs_folder: Some(packs.clone()),
        mods_folder: Some(mods.clone()),
        scan_for_mod_resources: true,
        data_root: data.clone(),
        resource_extensions: dir.join("ext"),
    };
    let world = dir.join("world/datapacks/x");
    let jar = dir.join("client.jar");
    let expected = [
        packs.join("c.zip"),
        packs.join("b"),
        packs.join("a.zip"),
        world.clone(),
        mods.join("m1.jar"),
        mods.join("m2.jar"),
        data.join("defaultBlockstates.zip"),
        dir.join("ext"),
        jar.clone(),
    ];
    assert_eq!(pack_roots(&config, std::slice::from_ref(&world), &jar).unwrap(), expected);
    config.scan_for_mod_resources = false;
    config.packs_folder = Some(dir.join("missing"));
    assert_eq!(pack_roots(&config, &[], &jar).unwrap(), [data.join("defaultBlockstates.zip"), dir.join("ext"), jar]);
    std::fs::remove_dir_all(dir).unwrap();
}
