//! The optimized format: conformance, format detection and in-place conversion, file and SQLite.

mod common;

use std::path::Path;

use bm_format::prbm::TileModel;
use bm_storage::{
    Compression, Error, FILE_MARKER, FileStorage, Format, GridKey, ItemKey, SqlConfig, SqlStorage, Storage,
    convert_file_storage, convert_sql_storage,
};
use tokio::runtime::Runtime;

/// A real PRBM tile with `quads` quads.
fn prbm(quads: usize, seed: f32) -> Vec<u8> {
    let mut m = TileModel::default();
    for q in 0..quads {
        let (x, y, z) = ((q % 32) as f32, 64. + seed + (q % 5) as f32, (q / 32) as f32);
        let (a, b, c, d) = ([x, y, z], [x, y, z + 1.], [x + 1., y, z + 1.], [x + 1., y, z]);
        for tri in [[a, b, c], [a, c, d]] {
            tri.iter().for_each(|p| m.position.extend(p));
            m.color.extend([1., 0.5, 0.25]);
            m.sunlight.push(15);
            m.blocklight.push(0);
            m.material.push((q / 50) as u32);
        }
        m.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
        m.ao.extend([1., 0.75, 0.5, 1., 0.5, 0.25]);
    }
    let mut out = Vec::new();
    m.write_prbm(&mut out).unwrap();
    out
}

const TILES: [(i32, i32); 5] = [(0, 0), (-1, 7), (15, 15), (16, -16), (40, 3)];

/// Fills map `m` of a compat storage with hires, lowres, rstate and items.
fn populate(storage: &dyn Storage) {
    let map = storage.map("m").unwrap();
    for (i, &t) in TILES.iter().enumerate() {
        map.write_grid(GridKey::Hires, t, &prbm(100 + i * 37, i as f32)).unwrap();
        map.write_grid(GridKey::Lowres(1), t, b"\x89PNG lowres").unwrap();
        map.write_grid(GridKey::TileState, t, b"nbt").unwrap();
    }
    map.write_item(&ItemKey::Settings, b"{}").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
}

fn assert_hires(storage: &dyn Storage) {
    let map = storage.map("m").unwrap();
    let mut tiles = map.list_grid(GridKey::Hires).unwrap();
    tiles.sort();
    let mut expected = TILES.to_vec();
    expected.sort();
    assert_eq!(tiles, expected);
    for (i, &t) in TILES.iter().enumerate() {
        assert_eq!(map.read_grid(GridKey::Hires, t).unwrap().unwrap().decompress().unwrap(), prbm(100 + i * 37, i as f32));
    }
}

fn no_progress(_: &str, _: usize, _: usize) {}

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
    populate(&storage);
    assert_hires(&storage);
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
    populate(&open(&compat, Format::Compat).unwrap());
    let err = open(&compat, Format::Optimized).err().unwrap();
    assert!(err.to_string().contains("--convert-storage"), "{err}");
    open(&compat, Format::Compat).unwrap();
}

#[test]
fn file_conversion_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("maps");
    populate(&FileStorage::new(&root, Compression::Gzip));
    let before = common::tree(&root);

    let stats = convert_file_storage(&root, Compression::Gzip, Format::Optimized, &no_progress).unwrap();
    assert_eq!((stats.maps, stats.tiles, stats.already), (1, TILES.len(), false));
    assert!(!root.join("m/tiles/0").exists());
    let opt = FileStorage::open(&root, Compression::Gzip, Format::Optimized, false).unwrap();
    assert_hires(&opt);
    drop(opt);
    assert!(convert_file_storage(&root, Compression::Gzip, Format::Optimized, &no_progress).unwrap().already);

    convert_file_storage(&root, Compression::Gzip, Format::Compat, &no_progress).unwrap();
    assert!(!root.join("m/hires").exists() && !root.join(FILE_MARKER).exists());
    common::assert_same_tree_maps(&before, &common::tree(&root));
}

#[test]
fn sqlite_detection_and_conversion() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("bm.db").display());
    let config = |format| SqlConfig { format, ..SqlConfig::new(url.clone()) };
    let compat = SqlStorage::connect(&config(Format::Compat), rt.handle().clone()).unwrap();
    populate(&compat);
    compat.close();
    let refused = SqlStorage::connect(&config(Format::Optimized), rt.handle().clone()).err().unwrap();
    assert!(matches!(refused, Error::FormatMismatch { found: Format::Compat, .. }), "{refused}");

    let stats = convert_sql_storage(&config(Format::Compat), rt.handle().clone(), Format::Optimized, &no_progress).unwrap();
    assert_eq!(stats.tiles, TILES.len());
    assert!(SqlStorage::connect(&config(Format::Compat), rt.handle().clone()).is_err());
    let opt = SqlStorage::connect(&config(Format::Optimized), rt.handle().clone()).unwrap();
    assert_hires(&opt);
    assert_eq!(opt.map("m").unwrap().read_item(&ItemKey::Settings).unwrap().unwrap().data, b"{}");
    opt.close();

    convert_sql_storage(&config(Format::Optimized), rt.handle().clone(), Format::Compat, &no_progress).unwrap();
    let back = SqlStorage::connect(&config(Format::Compat), rt.handle().clone()).unwrap();
    assert_hires(&back);
    back.close();
}
