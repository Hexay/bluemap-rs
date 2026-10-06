//! Parsed tile → bm-format `TileModel`, to check the PRBM writer reproduces BlueMap's bytes.

use bm_format::prbm::TileModel;

use crate::Tile;

/// A byte value `k` as the float the writer turns back into `k`: `(int)(f * 255)` truncates, so aim mid-step.
fn unit(k: u8) -> f32 {
    (k as f32 + 0.5) / 255.0
}

pub fn to_model(tile: &Tile) -> TileModel {
    let mut m = TileModel::default();
    for f in tile.faces() {
        m.position.extend(f.pos.iter().flatten());
        m.uv.extend(f.uv.iter().flatten());
        m.ao.extend(f.ao.map(unit));
        m.color.extend(f.color.map(unit));
        m.sunlight.push(f.sunlight as u8);
        m.blocklight.push(f.blocklight as u8);
        m.material.push(f.material);
    }
    m
}
