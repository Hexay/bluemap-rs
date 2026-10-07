//! An optimized storage serves the same map as its compat original: per hires tile the same decompressed PRBM
//! bytes and the same coding choices, for every Accept-Encoding and for `.gz` URLs; everything else unchanged.

mod common;

use std::path::Path;

use bm_format::prbm::TileModel;
use bm_storage::{Compression, FileStorage, Format, GridKey, ItemKey, Storage, convert_file_storage};
use bm_web::{MapRoute, WebApp, WebOptions};
use common::{Served, get, gunzip};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dst = to.join(e.file_name());
        if e.path().is_dir() { copy_dir(&e.path(), &dst) } else { std::fs::copy(e.path(), &dst).map(drop).unwrap() }
    }
}

fn serve(storage: &FileStorage, ids: &[String], web: &Path) -> Served {
    let mut app = WebApp::new(WebOptions::new(web)).unwrap();
    for id in ids {
        app.add_map(id.clone(), MapRoute { storage: storage.map(id).unwrap(), live: None }).unwrap();
    }
    Served::start(app)
}

/// Converts a copy of the compat storage at `compat` and compares both over HTTP; returns the tiles compared.
fn compare(compat: &Path, scratch: &Path) -> usize {
    let opt_root = scratch.join("opt/maps");
    copy_dir(compat, &opt_root);
    convert_file_storage(&opt_root, Compression::Gzip, Format::Optimized, &|_, _, _| {}).unwrap();
    let a = FileStorage::open(compat, Compression::Gzip, Format::Compat, true).unwrap();
    let b = FileStorage::open(&opt_root, Compression::Gzip, Format::Optimized, true).unwrap();
    let ids = a.map_ids().unwrap();
    let (sa, sb) = (serve(&a, &ids, &scratch.join("wa")), serve(&b, &ids, &scratch.join("wb")));
    let mut tiles = 0;
    for id in &ids {
        for (x, z) in a.map(id).unwrap().list_grid(GridKey::Hires).unwrap() {
            let path = format!("/maps/{id}/{}", bm_format::grid::tile_file(0, (x, z)));
            for accept in [&[("Accept-Encoding", "gzip")][..], &[]] {
                let (ra, rb) = (get(sa.addr, &path, accept), get(sb.addr, &path, accept));
                assert_eq!((ra.status, rb.status), (200, 200), "{path}");
                assert_eq!(ra.header("content-type"), rb.header("content-type"));
                assert_eq!(ra.header("content-encoding"), rb.header("content-encoding"), "{path} {accept:?}");
                assert!(ra.decoded() == rb.decoded(), "{path} differs");
            }
            let (ga, gb) = (get(sa.addr, &format!("{path}.gz"), &[]), get(sb.addr, &format!("{path}.gz"), &[]));
            assert!(gunzip(&ga.body) == gunzip(&gb.body), "{path}.gz differs");
            tiles += 1;
        }
        for item in ["settings.json", "textures.json", "tiles/1/x0/z0.png", "tiles/0/x99/z99.prbm"] {
            let path = format!("/maps/{id}/{item}");
            let (ra, rb) = (get(sa.addr, &path, &[]), get(sb.addr, &path, &[]));
            assert_eq!((ra.status, ra.decoded()), (rb.status, rb.decoded()), "{path}");
        }
    }
    tiles
}

fn prbm(quads: usize, y: f32) -> Vec<u8> {
    let mut m = TileModel::default();
    for q in 0..quads {
        let (x, z) = ((q % 32) as f32, (q / 32) as f32 + 0.05);
        for tri in [[[x, y, z], [x, y, z + 1.], [x + 1., y, z + 1.]], [[x, y, z], [x + 1., y, z + 1.], [x + 1., y, z]]] {
            tri.iter().for_each(|p| m.position.extend(p));
            m.color.extend([0.5, 0.5, 1.]);
            m.sunlight.push(15);
            m.blocklight.push(2);
            m.material.push(q as u32 / 40);
        }
        m.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
        m.ao.extend([1., 1., 0.5, 1., 0.5, 0.75]);
    }
    let mut out = Vec::new();
    m.write_prbm(&mut out).unwrap();
    out
}

#[test]
fn optimized_serves_identical_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("compat/maps");
    let map = FileStorage::new(&root, Compression::Gzip).map("world").unwrap();
    for (i, t) in [(0, 0), (-3, 17), (16, -1)].into_iter().enumerate() {
        map.write_grid(GridKey::Hires, t, &prbm(50 + 30 * i, 64. + i as f32)).unwrap();
    }
    map.write_grid(GridKey::Lowres(1), (0, 0), b"\x89PNG lowres").unwrap();
    map.write_item(&ItemKey::Settings, b"{}").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
    assert_eq!(compare(&root, dir.path()), 3);
}

#[test]
#[ignore = "needs Java BlueMap golden renders under work/bluemap"]
fn optimized_serves_java_golden_identically() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let work = here.ancestors().map(|a| a.join("work/bluemap")).find(|p| p.is_dir()).expect("work/bluemap");
    let dir = tempfile::tempdir().unwrap();
    for fixture in ["vanilla", "nether"] {
        let tiles = compare(&work.join(fixture).join("web/maps"), &dir.path().join(fixture));
        eprintln!("{fixture}: {tiles} hires tiles served identically");
        assert!(tiles > 0);
    }
}
