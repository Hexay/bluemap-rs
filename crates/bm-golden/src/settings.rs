//! The parts of the webapp's root `settings.json` and per-map `maps/<id>/settings.json` the oracle needs.

use bm_format::grid::Grid;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteSettings {
    #[serde(default)]
    pub version: Option<String>,
    pub maps: Vec<String>,
    #[serde(default = "default_maps_root")]
    pub map_data_root: String,
}

fn default_maps_root() -> String {
    "maps".into()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSettings {
    pub hires: HiresSettings,
    pub lowres: LowresSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiresSettings {
    pub tile_size: [i32; 2],
    pub translate: [i32; 2],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LowresSettings {
    pub tile_size: [i32; 2],
    pub lod_factor: i32,
    pub lod_count: u32,
}

impl MapSettings {
    pub fn hires_grid(&self) -> Grid {
        Grid { size: self.hires.tile_size, offset: self.hires.translate }
    }

    /// Blocks per lowres pixel at `lod` (1-based).
    pub fn lod_scale(&self, lod: u32) -> i32 {
        self.lowres.lod_factor.pow(lod - 1)
    }

    pub fn lowres_grid(&self, lod: u32) -> Grid {
        let s = self.lod_scale(lod);
        let [w, h] = self.lowres.tile_size;
        Grid { size: [w * s, h * s], offset: [0, 0] }
    }
}
