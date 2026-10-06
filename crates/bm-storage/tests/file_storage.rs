mod common;

use std::sync::Arc;

use bm_storage::{Compression, Error, FileStorage, GridKey, ItemKey, Storage};

#[test]
fn conformance() {
    let dir = tempfile::tempdir().unwrap();
    common::conformance(&FileStorage::new(dir.path(), Compression::Gzip));
}

#[test]
fn writes_upstream_paths_and_encodings() {
    let dir = tempfile::tempdir().unwrap();
    let storage = FileStorage::new(dir.path(), Compression::Gzip);
    let map = storage.map("m").unwrap();
    map.write_grid(GridKey::Hires, (-12, 5), b"prbm").unwrap();
    map.write_grid(GridKey::Lowres(2), (103, -7), b"png").unwrap();
    map.write_grid(GridKey::TileState, (0, -1), b"nbt").unwrap();
    map.write_grid(GridKey::RegionState, (0, 0), b"nbt").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
    map.write_item(&ItemKey::Markers, b"{}").unwrap();
    map.write_item(&ItemKey::asset("playerheads/x y.png"), b"head").unwrap();

    let tree = common::tree(dir.path());
    let names: Vec<_> = tree.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "m/assets/playerheads/x_y.png",
            "m/live/markers.json",
            "m/rstate/regions/x0/z0.regions.dat",
            "m/rstate/x0/z-1.tiles.dat",
            "m/textures.json.gz",
            "m/tiles/0/x-1/2/z5.prbm.gz",
            "m/tiles/2/x1/0/3/z-7.png",
        ]
    );
    let gunzip = |p: &str| Compression::Gzip.decompress(&tree[p], 1 << 20).unwrap();
    assert_eq!(gunzip("m/tiles/0/x-1/2/z5.prbm.gz"), b"prbm");
    assert_eq!(gunzip("m/rstate/x0/z-1.tiles.dat"), b"nbt", "rstate is gzip without .gz");
    assert_eq!(tree["m/tiles/2/x1/0/3/z-7.png"], b"png", "lowres is never compressed");
    assert_eq!(tree["m/live/markers.json"], b"{}");
}

#[test]
fn uncompressed_config_drops_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path(), Compression::None).map("m").unwrap();
    map.write_grid(GridKey::Hires, (0, 0), b"prbm").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
    assert_eq!(std::fs::read(dir.path().join("m/tiles/0/x0/z0.prbm")).unwrap(), b"prbm");
    assert_eq!(std::fs::read(dir.path().join("m/textures.json")).unwrap(), b"[]");
}

#[test]
fn listing_ignores_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path(), Compression::Gzip).map("m").unwrap();
    map.write_grid(GridKey::Hires, (1, 2), b"a").unwrap();
    let hires = dir.path().join("m/tiles/0");
    for junk in ["x1/z2.prbm.gz.filepart", "x1/z3.prbm", "x1/notes.txt", "x1/z9.prbm.gz.12-3.filepart"] {
        std::fs::write(hires.join(junk), b"junk").unwrap();
    }
    assert_eq!(map.list_grid(GridKey::Hires).unwrap(), vec![(1, 2)]);
    assert!(map.list_grid(GridKey::Lowres(1)).unwrap().is_empty());
}

#[test]
fn read_only_refuses_writes() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path(), Compression::Gzip).read_only(true).map("m").unwrap();
    assert!(matches!(map.write_item(&ItemKey::Settings, b"{}"), Err(Error::ReadOnly)));
    assert!(matches!(map.delete(&mut |_| true), Err(Error::ReadOnly)));
    assert!(!dir.path().join("m").exists());
}

#[test]
fn rejects_escaping_map_ids() {
    let storage = FileStorage::new("unused", Compression::Gzip);
    for id in ["", "..", "a/b", "a\\b", "C:x"] {
        assert!(matches!(storage.map(id), Err(Error::InvalidMapId(_))), "{id}");
    }
}

#[test]
fn same_map_storage_per_id() {
    let storage = FileStorage::new("unused", Compression::Gzip);
    let (a, b) = (storage.map("m").unwrap(), storage.map("m").unwrap());
    let _held = a.lock_grid(GridKey::Lowres(1), (0, 0));
    assert!(b.key_locks().try_lock(bm_storage::LockKey::Grid(GridKey::Lowres(1), (0, 0))).is_none());
}

/// Many writers and readers on one key: readers always see a complete payload, no temp file survives (#821).
#[test]
fn concurrent_writes_never_tear() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(FileStorage::new(dir.path(), Compression::Gzip));
    let payload = |i: usize| vec![i as u8; 64 * 1024 + i];
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let storage = storage.clone();
            std::thread::spawn(move || {
                let map = storage.map("m").unwrap();
                for i in 0..25 {
                    if t % 2 == 0 {
                        map.write_grid(GridKey::Lowres(1), (0, 0), &payload(t * 25 + i)).unwrap();
                    } else if let Some(stored) = map.read_grid(GridKey::Lowres(1), (0, 0)).unwrap() {
                        let n = stored.data.len() - 64 * 1024;
                        assert_eq!(stored.data, payload(n), "torn read");
                    }
                }
            })
        })
        .collect();
    threads.into_iter().for_each(|t| t.join().unwrap());
    let tree = common::tree(dir.path());
    assert_eq!(tree.keys().collect::<Vec<_>>(), ["m/tiles/1/x0/z0.png"]);
}

#[test]
fn delete_can_abort_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path(), Compression::Gzip).map("m").unwrap();
    for x in 0..20 {
        map.write_grid(GridKey::Hires, (x, 0), b"t").unwrap();
    }
    map.delete(&mut |_| false).unwrap();
    assert!(map.exists().unwrap());
    map.delete(&mut |_| true).unwrap();
    assert!(!map.exists().unwrap());
}
