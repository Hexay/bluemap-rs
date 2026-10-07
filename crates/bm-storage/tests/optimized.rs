//! The optimized format: conformance, format detection and in-place conversion, file and SQLite.

mod common;

use std::path::Path;

use bm_storage::{
    Compression, Error, FILE_MARKER, FileStorage, Format, SqlConfig, SqlStorage, convert_file_storage,
};
use tokio::runtime::Runtime;

#[test]
fn file_conformance() {
    let dir = tempfile::tempdir().unwrap();
    common::conformance(&FileStorage::open(dir.path(), Compression::Gzip, Format::Optimized, false).unwrap());
}

#[test]
fn sqlite_conformance() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = SqlConfig { format: Format::Optimized, ..SqlConfig::new(format!("sqlite:{}", dir.path().join("o.db").display())) };
    let storage = SqlStorage::connect(&config, rt.handle().clone()).unwrap();
    common::conformance(&storage);
    storage.close();
}

#[test]
fn file_layout_is_bundles_beside_upstream_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let storage = FileStorage::open(dir.path(), Compression::Gzip, Format::Optimized, false).unwrap();
    common::populate(&storage);
    common::assert_hires(&storage);
    let tree = common::tree(dir.path());
    let names: Vec<&str> = tree.keys().map(String::as_str).filter(|n| !n.contains("/tiles/1/") && !n.contains("rstate")).collect();
    assert_eq!(
        names,
        [FILE_MARKER, "m/hires/x-1z0.bmb", "m/hires/x0z0.bmb", "m/hires/x1z-1.bmb", "m/hires/x2z0.bmb", "m/settings.json", "m/textures.json.gz"]
    );
}

#[test]
fn file_format_detection_refuses_mismatches() {
    let dir = tempfile::tempdir().unwrap();
    let (opt, compat) = (dir.path().join("opt"), dir.path().join("compat"));
    let open = |root: &Path, f: Format| FileStorage::open(root, Compression::Gzip, f, false);
    FileStorage::open(&opt, Compression::Gzip, Format::Optimized, true).unwrap();
    assert!(!opt.join(FILE_MARKER).exists(), "read-only never marks");
    open(&opt, Format::Optimized).unwrap();
    assert!(opt.join(FILE_MARKER).exists());
    assert!(matches!(open(&opt, Format::Compat), Err(Error::FormatMismatch { .. })));
    common::populate(&open(&compat, Format::Compat).unwrap());
    let err = open(&compat, Format::Optimized).err().unwrap();
    assert!(err.to_string().contains("--convert-storage"), "{err}");
    open(&compat, Format::Compat).unwrap();
}

#[test]
fn file_conversion_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("maps");
    common::populate(&FileStorage::new(&root, Compression::Gzip));
    let before = common::tree(&root);

    let stats = convert_file_storage(&root, Compression::Gzip, Format::Optimized, &common::no_progress).unwrap();
    assert_eq!((stats.maps, stats.tiles, stats.already), (1, common::TILES.len(), false));
    assert!(!root.join("m/tiles/0").exists());
    let opt = FileStorage::open(&root, Compression::Gzip, Format::Optimized, false).unwrap();
    common::assert_hires(&opt);
    drop(opt);
    assert!(convert_file_storage(&root, Compression::Gzip, Format::Optimized, &common::no_progress).unwrap().already);

    convert_file_storage(&root, Compression::Gzip, Format::Compat, &common::no_progress).unwrap();
    assert!(!root.join("m/hires").exists() && !root.join(FILE_MARKER).exists());
    common::assert_same_tree_maps(&before, &common::tree(&root));
}

#[test]
fn sqlite_detection_and_conversion() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("bm.db").display());
    common::sql_detection_and_conversion(&|format| SqlConfig { format, ..SqlConfig::new(url.clone()) }, rt.handle());
}
