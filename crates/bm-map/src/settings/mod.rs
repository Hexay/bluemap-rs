//! The two `settings.json` files the webapp reads, byte-compatible with Java BlueMap:
//! per map (`MapSettingsSerializer`, written to map storage) and the webroot one (`WebFilesManager`).
//! Input structs mirror `MapConfig` / `WebappConfig` with their Java defaults; HOCON parsing lives elsewhere.

mod gson;
mod map;
mod webapp;

pub use map::map_settings_json;
pub use webapp::WebappSettings;

use std::path::PathBuf;

use crate::mask::MaskConfig;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid {field}: {source}")]
    Color { field: &'static str, source: bm_math::ParseColorError },
    #[error("{0} is not a finite number")]
    NonFinite(&'static str),
    #[error("failed to parse webapp settings.json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("update-settings-file is false but there is no settings.json to keep")]
    MissingSettingsFile,
}

/// `maps/<id>.conf` (`MapConfig.java`); `marker-sets` is not modelled yet.
#[derive(Clone, Debug, PartialEq)]
pub struct MapConfig {
    pub loader: String,
    pub world: Option<PathBuf>,
    pub dimension: Option<String>,
    pub dimension_type: Option<String>,
    pub name: Option<String>,
    pub sorting: i32,
    pub start_pos: [i32; 2],
    pub sky_color: String,
    pub void_color: String,
    pub ambient_light: f32,
    pub sky_light: f32,
    pub remove_caves_below_y: i32,
    pub cave_detection_ocean_floor: i32,
    pub cave_detection_uses_block_light: bool,
    pub render_mask: Vec<MaskConfig>,
    pub min_inhabited_time: i64,
    pub min_inhabited_time_radius: i32,
    pub render_edges: bool,
    pub edge_light_strength: i32,
    pub enable_perspective_view: bool,
    pub enable_flat_view: bool,
    pub enable_free_flight_view: bool,
    pub enable_hires: bool,
    pub check_for_removed_regions: bool,
    pub storage: String,
    pub ignore_missing_light_data: bool,
    pub hires_tile_size: i32,
    pub lowres_tile_size: i32,
    pub lod_count: i32,
    pub lod_factor: i32,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            loader: "bluemap:anvil".into(),
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
            hires_tile_size: 32,
            lowres_tile_size: 500,
            lod_count: 3,
            lod_factor: 5,
        }
    }
}

/// `webapp.conf` (`WebappConfig.java`). `scripts`/`styles` are insertion-ordered sets.
#[derive(Clone, Debug, PartialEq)]
pub struct WebappConfig {
    pub enabled: bool,
    pub update_settings_file: bool,
    pub webroot: PathBuf,
    pub use_cookies: bool,
    pub default_to_flat_view: bool,
    pub start_location: Option<String>,
    pub resolution_default: f32,
    pub min_zoom_distance: i32,
    pub max_zoom_distance: i32,
    pub hires_slider_max: i32,
    pub hires_slider_default: i32,
    pub hires_slider_min: i32,
    pub lowres_slider_max: i32,
    pub lowres_slider_default: i32,
    pub lowres_slider_min: i32,
    pub map_data_root: String,
    pub live_data_root: String,
    pub client_decompression: bool,
    pub scripts: Vec<String>,
    pub styles: Vec<String>,
}

impl Default for WebappConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            update_settings_file: true,
            webroot: PathBuf::from("bluemap/web"),
            use_cookies: true,
            default_to_flat_view: false,
            start_location: None,
            resolution_default: 1.0,
            min_zoom_distance: 5,
            max_zoom_distance: 100000,
            hires_slider_max: 500,
            hires_slider_default: 100,
            hires_slider_min: 0,
            lowres_slider_max: 7000,
            lowres_slider_default: 2000,
            lowres_slider_min: 500,
            map_data_root: "maps".into(),
            live_data_root: "maps".into(),
            client_decompression: false,
            scripts: Vec::new(),
            styles: Vec::new(),
        }
    }
}

/// Map id from a config file name (without extension): Java regex `\W` → `_`, one per code point.
pub fn sanitize_map_id(config_name: &str) -> String {
    config_name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}
