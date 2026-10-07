use std::path::PathBuf;

use serde::Deserialize;

use super::mask::{self, RenderMask};
use crate::de::fields;
use crate::key::Key;
use crate::value::Value;

/// `maps/<id>.conf` (`MapConfig.java`). Defaults are the Java field initialisers (the template writes some
/// different values, e.g. `cave-detection-ocean-floor: -5`, `edge-light-strength: 8`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MapConfig {
    /// Hidden key; only `bluemap:anvil` exists.
    #[serde(deserialize_with = "fields::world_loader")]
    pub loader: Key,
    /// `None`: display-only map, served from storage but never rendered.
    pub world: Option<PathBuf>,
    /// `None`: inferred from the world folder layout (legacy `DIM-1`/`DIM1`).
    #[serde(deserialize_with = "fields::opt_key")]
    pub dimension: Option<Key>,
    #[serde(deserialize_with = "fields::opt_key")]
    pub dimension_type: Option<Key>,
    /// `None`: the map id.
    pub name: Option<String>,
    pub sorting: i32,
    /// `[x, z]`.
    #[serde(deserialize_with = "fields::vec2i")]
    pub start_pos: [i32; 2],
    pub sky_color: String,
    pub void_color: String,
    pub ambient_light: f32,
    pub sky_light: f32,
    pub remove_caves_below_y: i32,
    pub cave_detection_ocean_floor: i32,
    pub cave_detection_uses_block_light: bool,
    #[serde(deserialize_with = "mask::deserialize")]
    pub render_mask: Vec<RenderMask>,
    pub min_inhabited_time: i64,
    /// Hidden key.
    pub min_inhabited_time_radius: i32,
    pub render_edges: bool,
    pub edge_light_strength: i32,
    pub enable_perspective_view: bool,
    pub enable_flat_view: bool,
    pub enable_free_flight_view: bool,
    pub enable_hires: bool,
    /// Hidden key.
    pub check_for_removed_regions: bool,
    /// Storage config id (`storages/<id>.conf`).
    pub storage: String,
    pub ignore_missing_light_data: bool,
    /// Raw `marker-sets` node; see [`MapConfig::marker_sets_json`].
    pub marker_sets: Option<Value>,
    /// Hidden keys.
    pub hires_tile_size: i32,
    pub lowres_tile_size: i32,
    pub lod_count: i32,
    pub lod_factor: i32,
    /// Pre-3.x bounds keys; their presence is rejected by [`MapConfig::check_legacy`].
    min_x: Option<i32>,
    max_x: Option<i32>,
    min_z: Option<i32>,
    max_z: Option<i32>,
    min_y: Option<i32>,
    max_y: Option<i32>,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            loader: Key::bluemap("anvil"),
            world: None,
            dimension: None,
            dimension_type: None,
            name: None,
            sorting: 0,
            start_pos: [0, 0],
            sky_color: "#7dabff".into(),
            void_color: "#000000".into(),
            ambient_light: 0.0,
            sky_light: 1.0,
            remove_caves_below_y: 55,
            cave_detection_ocean_floor: 10000,
            cave_detection_uses_block_light: false,
            render_mask: Vec::new(),
            min_inhabited_time: 0,
            min_inhabited_time_radius: 0,
            render_edges: true,
            edge_light_strength: 15,
            enable_perspective_view: true,
            enable_flat_view: true,
            enable_free_flight_view: true,
            enable_hires: true,
            check_for_removed_regions: true,
            storage: "file".into(),
            ignore_missing_light_data: false,
            marker_sets: None,
            hires_tile_size: 32,
            lowres_tile_size: 500,
            lod_count: 3,
            lod_factor: 5,
            min_x: None,
            max_x: None,
            min_z: None,
            max_z: None,
            min_y: None,
            max_y: None,
        }
    }
}

pub(crate) const LEGACY_MESSAGE: &str = "Your map-configuration is outdated! Looks like you updated BlueMap but did not follow the upgrade-instructions correctly. To fix your config, make sure to follow all relevant upgrade-instructions from BlueMap's changelogs: https://github.com/BlueMap-Minecraft/BlueMap/releases";

impl MapConfig {
    /// `checkLegacy()`: old `min-x`/`max-x`/… bounds keys mean an unmigrated config.
    pub fn check_legacy(&self) -> Result<(), &'static str> {
        let legacy = [self.min_x, self.max_x, self.min_z, self.max_z, self.min_y, self.max_y];
        if legacy.iter().any(Option::is_some) { Err(LEGACY_MESSAGE) } else { Ok(()) }
    }

    /// `marker-sets` as the JSON text BlueMap feeds to MarkerGson (`{}` when absent); see
    /// [`Value::to_configurate_json`].
    pub fn marker_sets_json(&self) -> String {
        self.marker_sets.as_ref().map_or_else(|| "{}".to_owned(), Value::to_configurate_json)
    }
}
