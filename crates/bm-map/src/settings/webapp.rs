//! Webroot `settings.json` (`WebFilesManager.Settings`, updated by `BlueMapService.createOrUpdateWebApp`).

use serde::Deserialize;

use super::gson::{JsonObject, string_array};
use super::{SettingsError, WebappConfig};

/// The persisted settings; field defaults are the `Settings` class's own, which differ from `WebappConfig`'s.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebappSettings {
    pub version: Option<String>,
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
    pub map_data_root: Option<String>,
    pub live_data_root: Option<String>,
    pub client_decompression: bool,
    pub maps: Vec<String>,
    pub scripts: Vec<String>,
    pub styles: Vec<String>,
}

impl Default for WebappSettings {
    fn default() -> Self {
        Self {
            version: None,
            use_cookies: true,
            default_to_flat_view: false,
            start_location: None,
            resolution_default: 1.0,
            min_zoom_distance: 5,
            max_zoom_distance: 100000,
            hires_slider_max: 500,
            hires_slider_default: 200,
            hires_slider_min: 50,
            lowres_slider_max: 10000,
            lowres_slider_default: 2000,
            lowres_slider_min: 500,
            map_data_root: Some("maps".into()),
            live_data_root: Some("maps".into()),
            client_decompression: false,
            maps: Vec::new(),
            scripts: Vec::new(),
            styles: Vec::new(),
        }
    }
}

impl WebappSettings {
    pub fn new(version: &str) -> Self {
        Self { version: Some(version.to_owned()), ..Self::default() }
    }

    /// `createOrUpdateWebApp`: rebuilt from config, or (with `update-settings-file = false`) the existing file with
    /// config scripts/styles/maps appended; its `version` is then kept as is. `maps` = (id, sorting) in the order the
    /// config files were listed.
    pub fn create_or_update(
        version: &str,
        config: &WebappConfig,
        maps: &[(&str, i32)],
        existing: Option<&str>,
    ) -> Result<Self, SettingsError> {
        let mut settings = Self::new(version);
        if config.update_settings_file {
            settings.set_from(config);
            settings.maps.clear();
        } else {
            settings = Self::from_json(existing.ok_or(SettingsError::MissingSettingsFile)?)?;
            settings.add_from(config);
        }
        settings.add_maps(maps);
        Ok(settings)
    }

    /// `loadSettings` (Gson into a fresh `Settings`: absent fields keep their defaults, sets drop duplicates).
    pub fn from_json(json: &str) -> Result<Self, SettingsError> {
        let mut settings: Self = serde_json::from_str(json)?;
        for set in [&mut settings.maps, &mut settings.scripts, &mut settings.styles] {
            let items = std::mem::take(set);
            items.into_iter().for_each(|item| insert(set, item));
        }
        Ok(settings)
    }

    /// `Settings.setFrom(WebappConfig)`; leaves `version` and `maps` alone.
    pub fn set_from(&mut self, config: &WebappConfig) {
        self.use_cookies = config.use_cookies;
        self.default_to_flat_view = config.default_to_flat_view;
        self.start_location = config.start_location.clone();
        self.resolution_default = config.resolution_default;
        self.min_zoom_distance = config.min_zoom_distance;
        self.max_zoom_distance = config.max_zoom_distance;
        self.hires_slider_max = config.hires_slider_max;
        self.hires_slider_default = config.hires_slider_default;
        self.hires_slider_min = config.hires_slider_min;
        self.lowres_slider_max = config.lowres_slider_max;
        self.lowres_slider_default = config.lowres_slider_default;
        self.lowres_slider_min = config.lowres_slider_min;
        self.map_data_root = Some(config.map_data_root.clone());
        self.live_data_root = Some(config.live_data_root.clone());
        self.styles.clear();
        self.scripts.clear();
        self.client_decompression = config.client_decompression;
        self.add_from(config);
    }

    /// `Settings.addFrom(WebappConfig)`.
    pub fn add_from(&mut self, config: &WebappConfig) {
        config.scripts.iter().for_each(|s| insert(&mut self.scripts, s.clone()));
        config.styles.iter().for_each(|s| insert(&mut self.styles, s.clone()));
    }

    /// `WebFilesManager.addFrom(Map<String, MapConfig>)`: the configs sit in a `HashMap`, then a stable sort by
    /// `sorting` — so equal sortings come out in `HashMap` order, not file order.
    pub fn add_maps(&mut self, maps: &[(&str, i32)]) {
        let ids: Vec<&str> = maps.iter().map(|&(id, _)| id).collect();
        let mut ordered: Vec<(&str, i32)> = bm_java::hash_map_order(&ids)
            .into_iter()
            .map(|id| maps.iter().rev().find(|&&(m, _)| m == id).copied().unwrap_or((id, 0)))
            .collect();
        ordered.sort_by_key(|&(_, sorting)| sorting);
        ordered.into_iter().for_each(|(id, _)| insert(&mut self.maps, id.to_owned()));
    }

    /// The file bytes as `GSON.toJson(settings)` writes them.
    pub fn to_json(&self) -> Result<String, SettingsError> {
        // Gson's float adapter rejects NaN/Infinity unless serializeSpecialFloatingPointValues is set (it isn't)
        if !self.resolution_default.is_finite() {
            return Err(SettingsError::NonFinite("resolutionDefault"));
        }
        Ok(JsonObject::new()
            .opt_string("version", self.version.as_deref())
            .bool("useCookies", self.use_cookies)
            .bool("defaultToFlatView", self.default_to_flat_view)
            .opt_string("startLocation", self.start_location.as_deref())
            .raw("resolutionDefault", &bm_java::fmt::float_to_string(self.resolution_default))
            .int("minZoomDistance", self.min_zoom_distance)
            .int("maxZoomDistance", self.max_zoom_distance)
            .int("hiresSliderMax", self.hires_slider_max)
            .int("hiresSliderDefault", self.hires_slider_default)
            .int("hiresSliderMin", self.hires_slider_min)
            .int("lowresSliderMax", self.lowres_slider_max)
            .int("lowresSliderDefault", self.lowres_slider_default)
            .int("lowresSliderMin", self.lowres_slider_min)
            .opt_string("mapDataRoot", self.map_data_root.as_deref())
            .opt_string("liveDataRoot", self.live_data_root.as_deref())
            .bool("clientDecompression", self.client_decompression)
            .raw("maps", &string_array(&self.maps))
            .raw("scripts", &string_array(&self.scripts))
            .raw("styles", &string_array(&self.styles))
            .finish())
    }
}

/// `LinkedHashSet.add`.
fn insert(set: &mut Vec<String>, item: String) {
    if !set.contains(&item) {
        set.push(item);
    }
}
