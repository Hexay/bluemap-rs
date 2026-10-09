use bm_format::compact::is_compact;
use bm_format::grid::Grid;
use bm_format::prbm::TileModel;

use crate::file::FileStorage;
use crate::{Compression, GridKey, MapStorage};

/// One block top face per entry of `ys`, in a row along x.
fn prbm(ys: &[f32]) -> Vec<u8> {
    let mut m = TileModel::default();
    for (x, &y) in ys.iter().enumerate() {
        let x = x as f32;
        let (a, b, c, d) = ([x, y, 0.], [x, y, 1.], [x + 1., y, 1.], [x + 1., y, 0.]);
        for tri in [[a, b, c], [a, c, d]] {
            tri.iter().for_each(|p| m.position.extend(p));
            m.color.extend([1., 0.5, 0.25]);
            m.sunlight.push(15);
            m.blocklight.push(0);
            m.material.push(0);
        }
        m.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
        m.ao.extend([1.; 6]);
    }
    let mut out = Vec::new();
    m.write_prbm(&mut out).unwrap();
    out
}

#[test]
fn hires_tiles_round_trip_with_or_without_a_grid() {
    let dir = tempfile::tempdir().unwrap();
    let map = FileStorage::new(dir.path(), Compression::Gzip).optimized_map("m").unwrap();
    let (terrain, empty) = (prbm(&[70., 70.5, 71.]), prbm(&[]));
    map.write_grid(GridKey::Hires, (3, -2), &terrain).unwrap();
    assert_eq!(map.origin((3, -2)), None);
    map.set_hires_grid(Grid { size: [32; 2], offset: [2; 2] });
    assert_eq!(map.origin((3, -2)), Some([98, -62]));
    map.write_grid(GridKey::Hires, (4, -2), &terrain).unwrap();
    map.write_grid(GridKey::Hires, (5, -2), &empty).unwrap();
    for (tile, raw) in [((3, -2), &terrain), ((4, -2), &terrain), ((5, -2), &empty)] {
        assert!(is_compact(&map.read_hires_blob(tile).unwrap().unwrap()));
        assert_eq!(map.read_grid(GridKey::Hires, tile).unwrap().unwrap().data, *raw);
    }
}
