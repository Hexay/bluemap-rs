//! Oracle: file storages written by Java BlueMap 5.28 (`work/bluemap/<fixture>/web/maps`, see
//! `tools/render_golden.py`). Read-only on the originals. Run: `cargo test -p bm-storage --test oracle -- --ignored`.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bm_storage::{Compression, FileStorage, GridKey, ItemKey, MapStorage, SqlConfig, SqlStorage, Storage, copy_map};

fn fixture_roots() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(work) = manifest.ancestors().map(|a| a.join("work/bluemap")).find(|p| p.is_dir()) else {
        panic!("no work/bluemap above {}; run tools/render_golden.py", manifest.display());
    };
    let mut roots: Vec<PathBuf> =
        std::fs::read_dir(work).unwrap().map(|e| e.unwrap().path().join("web/maps")).filter(|p| p.is_dir()).collect();
    roots.sort();
    assert!(!roots.is_empty());
    roots
}

#[test]
#[ignore = "needs Java-rendered fixtures in work/bluemap"]
fn reads_every_java_written_file() {
    let (mut maps, mut cells) = (0, 0);
    for root in fixture_roots() {
        let storage = FileStorage::new(&root, Compression::Gzip).read_only(true);
        for id in storage.map_ids().unwrap() {
            maps += 1;
            let map = storage.file_map(&id).unwrap();
            let mut expected = BTreeSet::new();
            let grids = map.grids().unwrap();
            assert!(grids.contains(&GridKey::Hires) && grids.contains(&GridKey::Lowres(1)), "{id}: {grids:?}");
            assert!(grids.contains(&GridKey::TileState) && grids.contains(&GridKey::RegionState), "{id}");
            for grid in grids {
                let tiles = map.list_grid(grid).unwrap();
                assert!(!tiles.is_empty(), "{id} {grid:?}");
                for tile in tiles {
                    let stored = map.read_grid(grid, tile).unwrap().unwrap();
                    let path = map.grid_cell_path(grid, tile);
                    assert_eq!(stored.data, std::fs::read(&path).unwrap());
                    let raw = stored.decompress().unwrap();
                    match grid {
                        GridKey::Hires => assert_eq!(raw[0], 1, "PRBM version"),
                        GridKey::Lowres(_) => assert!(raw.starts_with(b"\x89PNG")),
                        _ => assert_eq!(raw[0], 10, "NBT compound"),
                    }
                    expected.insert(path);
                    cells += 1;
                }
            }
            for item in ItemKey::FIXED {
                let stored = map.read_item(&item).unwrap().unwrap_or_else(|| panic!("{id} {item:?}"));
                let json = stored.decompress().unwrap();
                assert!(matches!(json.first(), Some(b'{' | b'[')), "{id} {item:?}");
                expected.insert(map.item_path(&item));
            }
            for asset in map.list_assets().unwrap() {
                expected.insert(map.item_path(&ItemKey::Asset(asset)));
            }
            let on_disk: BTreeSet<PathBuf> = common::tree(map.root()).keys().map(|rel| map.root().join(rel)).collect();
            let expected: BTreeSet<PathBuf> = expected.into_iter().map(normalize).collect();
            let on_disk: BTreeSet<PathBuf> = on_disk.into_iter().map(normalize).collect();
            assert_eq!(on_disk, expected, "{id}: API must account for every file");
        }
    }
    eprintln!("oracle: {maps} maps, {cells} grid cells read");
}

fn normalize(p: PathBuf) -> PathBuf {
    PathBuf::from(p.to_str().unwrap().replace('\\', "/"))
}

#[test]
#[ignore = "needs Java-rendered fixtures in work/bluemap"]
fn copy_reproduces_java_tree_byte_for_byte() {
    let tmp = tempfile::tempdir().unwrap();
    for (i, root) in fixture_roots().into_iter().enumerate() {
        let src = FileStorage::new(&root, Compression::Gzip).read_only(true);
        let dst = FileStorage::new(tmp.path().join(i.to_string()), Compression::Gzip);
        for id in src.map_ids().unwrap() {
            let stats = copy_map(src.map(&id).unwrap().as_ref(), dst.map(&id).unwrap().as_ref()).unwrap();
            assert_eq!(stats.transcoded, 0);
            common::assert_same_tree(&root.join(&id), &dst.root().join(&id));
        }
    }
}

#[test]
#[ignore = "needs Java-rendered fixtures in work/bluemap"]
fn java_tree_survives_sqlite_round_trip() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let root = fixture_roots().into_iter().next().unwrap();
    let src = FileStorage::new(&root, Compression::Gzip).read_only(true);
    let sql = SqlStorage::connect(
        &SqlConfig::new(format!("sqlite:{}", tmp.path().join("bm.db").display())),
        rt.handle().clone(),
    )
    .unwrap();
    let dst = FileStorage::new(tmp.path().join("out"), Compression::Gzip);
    for id in src.map_ids().unwrap() {
        copy_map(src.map(&id).unwrap().as_ref(), sql.map(&id).unwrap().as_ref()).unwrap();
        copy_map(sql.map(&id).unwrap().as_ref(), dst.map(&id).unwrap().as_ref()).unwrap();
        common::assert_same_tree(&root.join(&id), &dst.root().join(&id));
    }
    sql.close();
}
