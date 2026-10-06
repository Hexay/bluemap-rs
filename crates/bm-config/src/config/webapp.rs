use std::path::PathBuf;

use serde::Deserialize;

use crate::de::fields;

/// `webapp.conf` (`WebappConfig.java`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
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
    /// `LinkedHashSet`: file order, duplicates removed.
    #[serde(deserialize_with = "fields::string_set")]
    pub scripts: Vec<String>,
    #[serde(deserialize_with = "fields::string_set")]
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
