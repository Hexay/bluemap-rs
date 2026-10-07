//! Backend-neutral conformance checks, run against every storage implementation.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use bm_storage::{GridKey, ItemKey, Storage};

pub const GRIDS: [GridKey; 6] = [
    GridKey::Hires,
    GridKey::Lowres(1),
    GridKey::Lowres(3),
    GridKey::TileState,
    GridKey::ChunkState,
    GridKey::RegionState,
];

pub fn conformance(storage: &dyn Storage) {
    let map = storage.map("world").unwrap();
    assert!(storage.map_ids().unwrap().is_empty());
    assert!(!map.exists().unwrap());
    assert_eq!(map.read_grid(GridKey::Hires, (0, 0)).unwrap(), None);
    assert_eq!(map.read_item(&ItemKey::Settings).unwrap(), None);
    assert!(map.list_grid(GridKey::Hires).unwrap().is_empty());

    for (i, grid) in GRIDS.into_iter().enumerate() {
        for tile in [(0, 0), (-12, 5), (103, -7)] {
            let raw = format!("{grid:?} {tile:?} {i}").repeat(20);
            map.write_grid(grid, tile, raw.as_bytes()).unwrap();
            let stored = map.read_grid(grid, tile).unwrap().unwrap();
            assert_eq!(stored.compression, map.grid_compression(grid));
            assert_eq!(stored.decompress().unwrap(), raw.as_bytes());
            assert!(map.grid_exists(grid, tile).unwrap());
        }
        let mut tiles = map.list_grid(grid).unwrap();
        tiles.sort();
        assert_eq!(tiles, vec![(-12, 5), (0, 0), (103, -7)], "{grid:?}");
    }
    let mut grids = map.grids().unwrap();
    grids.retain(|g| !map.list_grid(*g).unwrap().is_empty());
    assert_eq!(grids, GRIDS.to_vec());

    let pre = map.grid_compression(GridKey::Hires).compress(b"pre").unwrap();
    map.write_grid_encoded(GridKey::Hires, (1, 1), &pre).unwrap();
    assert_eq!(map.read_grid(GridKey::Hires, (1, 1)).unwrap().unwrap().decompress().unwrap(), b"pre");
    map.delete_grid(GridKey::Hires, (1, 1)).unwrap();
    map.delete_grid(GridKey::Hires, (1, 1)).unwrap();
    assert!(!map.grid_exists(GridKey::Hires, (1, 1)).unwrap());

    let items = [
        ItemKey::Settings,
        ItemKey::Textures,
        ItemKey::Markers,
        ItemKey::Players,
        ItemKey::asset("playerheads/ab.png"),
    ];
    for item in &items {
        let raw = format!("{{\"item\":\"{item:?}\"}}");
        map.write_item(item, raw.as_bytes()).unwrap();
        let stored = map.read_item(item).unwrap().unwrap();
        assert_eq!(stored.compression, map.item_compression(item));
        assert_eq!(stored.decompress().unwrap(), raw.as_bytes());
        assert!(map.item_exists(item).unwrap());
    }
    assert_eq!(map.list_assets().unwrap(), vec!["playerheads/ab.png"]);
    map.delete_item(&ItemKey::Players).unwrap();
    assert!(!map.item_exists(&ItemKey::Players).unwrap());

    assert!(map.exists().unwrap());
    assert_eq!(storage.map_ids().unwrap(), vec!["world"]);
    let other = storage.map("other").unwrap();
    other.write_item(&ItemKey::Settings, b"{}").unwrap();
    let mut progress = Vec::new();
    map.delete(&mut |p| {
        progress.push(p);
        true
    })
    .unwrap();
    assert!(!map.exists().unwrap());
    assert!(progress.last().is_some_and(|p| (*p - 1.0).abs() < 1e-9), "{progress:?}");
    assert_eq!(map.read_item(&ItemKey::Settings).unwrap(), None);
    assert!(map.list_grid(GridKey::Hires).unwrap().is_empty());
    assert_eq!(other.read_item(&ItemKey::Settings).unwrap().unwrap().data, b"{}");
}

/// Relative path (with `/`) → bytes of every file under `root`.
pub fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_str().unwrap().replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// Same file set and bytes, except hires `.prbm.gz` cells, which compare decompressed (gzip output depends on
/// the encoder, e.g. Java's zlib vs ours).
pub fn assert_same_tree_maps(a: &BTreeMap<String, Vec<u8>>, b: &BTreeMap<String, Vec<u8>>) {
    let names = |t: &BTreeMap<String, Vec<u8>>| t.keys().cloned().collect::<Vec<_>>();
    assert_eq!(names(a), names(b), "file sets differ");
    let gunzip = |d: &[u8]| bm_storage::Compression::Gzip.decompress(d, bm_storage::MAX_DECODED).unwrap();
    for (name, bytes) in a {
        let same = if name.ends_with(".prbm.gz") { gunzip(bytes) == gunzip(&b[name]) } else { bytes == &b[name] };
        assert!(same, "{name} differs");
    }
}

pub fn assert_same_tree(a: &Path, b: &Path) {
    let (ta, tb) = (tree(a), tree(b));
    let names = |t: &BTreeMap<String, Vec<u8>>| t.keys().cloned().collect::<Vec<_>>();
    assert_eq!(names(&ta), names(&tb), "file sets differ");
    for (name, bytes) in &ta {
        assert!(bytes == &tb[name], "{name} differs");
    }
}
