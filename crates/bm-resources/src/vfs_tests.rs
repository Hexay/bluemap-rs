use std::io::Write;

use zip::write::SimpleFileOptions;

use super::*;

const FILES: [(&str, &str); 4] = [
    ("pack.mcmeta", "{}"),
    ("assets/minecraft/models/block/stone.json", "{\"parent\":\"block/cube_all\"}"),
    ("assets/minecraft/models/block/sub/x.json", "{}"),
    ("overlay_1/assets/minecraft/models/block/stone.json", "{\"overlay\":true}"),
];

fn zip_bytes() -> Arc<[u8]> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in FILES {
        w.start_file(name, SimpleFileOptions::default()).unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner().into()
}

fn dir_pack() -> (PathBuf, Pack) {
    let root = std::env::temp_dir().join(format!("bm-vfs-{}", std::process::id()));
    for (name, body) in FILES {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    let pack = Pack::open(&root).unwrap();
    (root, pack)
}

fn check(pack: &Pack) {
    assert_eq!(pack.read_string("pack.mcmeta").as_deref(), Some("{}"));
    assert!(pack.read("nope.json").is_none());
    assert!(pack.is_dir("assets/minecraft") && !pack.is_dir("assets/minecraft/models/block/stone.json"));
    assert!(pack.exists("assets") && pack.exists("pack.mcmeta") && !pack.exists("x"));
    assert_eq!(pack.list(""), ["assets", "overlay_1", "pack.mcmeta"]);
    assert_eq!(pack.list("assets/minecraft/models/block"), ["stone.json", "sub"]);
    assert_eq!(
        pack.walk("assets/minecraft/models"),
        ["assets/minecraft/models/block/stone.json", "assets/minecraft/models/block/sub/x.json"]
    );
    let overlay = pack.sub("overlay_1");
    assert_eq!(overlay.read_string("assets/minecraft/models/block/stone.json").as_deref(), Some("{\"overlay\":true}"));
    assert_eq!(overlay.walk("assets"), ["assets/minecraft/models/block/stone.json"]);
}

#[test]
fn zip_packs() {
    check(&Pack::zip(zip_bytes(), "test.zip".into()).unwrap());
}

#[test]
fn folder_packs_behave_like_zips() {
    let (root, pack) = dir_pack();
    check(&pack);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn not_a_zip_is_an_error() {
    assert!(Pack::zip(Arc::from(&b"not a zip"[..]), "x".into()).is_err());
}
