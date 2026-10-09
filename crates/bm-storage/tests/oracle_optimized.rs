//! Oracle for the optimized format on Java BlueMap 5.28 storages (`work/bluemap/<fixture>/web/maps`, copied to
//! a temp dir): compat → optimized → compat in place reproduces the Java tree (hires compared decompressed: our
//! gzip bytes differ from Java's zlib), and every tile reads back as the Java PRBM. Prints hires sizes.
//! Run: `cargo test -p bm-storage --release --test oracle_optimized -- --ignored --nocapture`.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use bm_storage::{Compression, FileStorage, Format, GridKey, Storage, convert_file_storage};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// (files, bytes) of tree entries whose path contains `part`.
fn sized(tree: &BTreeMap<String, Vec<u8>>, part: &str) -> (usize, u64) {
    tree.iter().filter(|(k, _)| k.contains(part)).fold((0, 0), |(n, b), (_, v)| (n + 1, b + v.len() as u64))
}

/// The fixtures' hires grid (BlueMap's default), so block hash offsets are modelled as in a real render.
fn default_grid(_: &str) -> Option<bm_format::grid::Grid> {
    Some(bm_format::grid::Grid { size: [32; 2], offset: [2; 2] })
}

#[test]
#[ignore = "needs Java-rendered fixtures in work/bluemap"]
fn java_storages_convert_losslessly() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let work = here.ancestors().map(|a| a.join("work/bluemap")).find(|p| p.is_dir()).expect("work/bluemap");
    let tmp = tempfile::tempdir().unwrap();
    let (mut gz_total, mut opt_total, mut files_total, mut bundles_total) = (0, 0, 0, 0);
    let mut fixtures: Vec<_> = std::fs::read_dir(&work).unwrap().map(|e| e.unwrap().path()).collect();
    fixtures.sort();
    for fixture in fixtures {
        let src = fixture.join("web/maps");
        if !src.is_dir() {
            continue;
        }
        let name = fixture.file_name().unwrap().to_string_lossy().into_owned();
        let root = tmp.path().join(&name);
        copy_dir(&src, &root);
        let java = common::tree(&root);
        let (files, gz) = sized(&java, "/tiles/0/");

        convert_file_storage(&root, Compression::Gzip, Format::Optimized, &default_grid, &|_, _, _| {}).unwrap();
        let opt_tree = common::tree(&root);
        let (bundles, opt) = sized(&opt_tree, "/hires/");
        let storage = FileStorage::open(&root, Compression::Gzip, Format::Optimized, true).unwrap();
        let compat = FileStorage::new(&src, Compression::Gzip).read_only(true);
        for id in storage.map_ids().unwrap() {
            let (o, c) = (storage.map(&id).unwrap(), compat.map(&id).unwrap());
            let tiles = c.list_grid(GridKey::Hires).unwrap();
            assert_eq!(o.list_grid(GridKey::Hires).unwrap().len(), tiles.len());
            for t in tiles {
                let want = c.read_grid(GridKey::Hires, t).unwrap().unwrap().decompress().unwrap();
                assert!(o.read_grid(GridKey::Hires, t).unwrap().unwrap().data == want, "{name}/{id} {t:?}");
            }
        }
        drop(storage);

        convert_file_storage(&root, Compression::Gzip, Format::Compat, &default_grid, &|_, _, _| {}).unwrap();
        common::assert_same_tree_maps(&java, &common::tree(&root));
        println!(
            "{name:16} hires {files:5} files {:8.2} MB gzip -> {bundles:4} bundles {:7.2} MB ({:.1}x smaller)",
            gz as f64 / 1e6,
            opt as f64 / 1e6,
            gz as f64 / opt as f64
        );
        (gz_total, opt_total, files_total, bundles_total) =
            (gz_total + gz, opt_total + opt, files_total + files, bundles_total + bundles);
    }
    println!(
        "total            hires {files_total:5} files {:8.2} MB gzip -> {bundles_total:4} bundles {:7.2} MB ({:.1}x smaller)",
        gz_total as f64 / 1e6,
        opt_total as f64 / 1e6,
        gz_total as f64 / opt_total as f64
    );
}
