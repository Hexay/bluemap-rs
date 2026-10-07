//! Per-map `settings.json` (`MapSettingsSerializer.java`).

use bm_java::fmt::{double_to_string, float_to_string};
use bm_math::Color;

use crate::gson::{JsonObject, array, int_array};
use super::{MapConfig, SettingsError};

/// `BmMap` builds its hires grid as `new Grid(hiresTileSize, 2)`.
const HIRES_GRID_OFFSET: i32 = 2;

/// The map's `settings.json`; `id` is the display name when the config has none.
pub fn map_settings_json(id: &str, config: &MapConfig) -> Result<String, SettingsError> {
    let hires_size = config.hires_tile_size;
    let hires = JsonObject::new()
        .raw("tileSize", &int_array(&[hires_size, hires_size]))
        .raw("scale", &int_array(&[1, 1]))
        .raw("translate", &int_array(&[HIRES_GRID_OFFSET, HIRES_GRID_OFFSET]))
        .finish();
    let lowres_size = config.lowres_tile_size;
    let lowres = JsonObject::new()
        .raw("tileSize", &int_array(&[lowres_size, lowres_size]))
        .int("lodFactor", config.lod_factor)
        .int("lodCount", config.lod_count)
        .finish();
    Ok(JsonObject::new()
        .string("name", config.name.as_deref().unwrap_or(id))
        .int("sorting", config.sorting)
        .raw("hires", &hires)
        .raw("lowres", &lowres)
        .raw("startPos", &int_array(&config.start_pos))
        .raw("skyColor", &color_json(&config.sky_color, "sky-color")?)
        .raw("voidColor", &color_json(&config.void_color, "void-color")?)
        // JsonPrimitive(Float) prints via Float.toString; the tree writer is lenient, so NaN/Infinity pass through
        .raw("ambientLight", &float_to_string(config.ambient_light))
        .raw("skyLight", &float_to_string(config.sky_light))
        .bool("perspectiveView", config.enable_perspective_view)
        .bool("flatView", config.enable_flat_view)
        .bool("freeFlightView", config.enable_free_flight_view)
        .finish())
}

/// `ColorAdapter.write`: straight RGBA, each float channel widened to double by `JsonWriter.value(double)`.
fn color_json(value: &str, field: &'static str) -> Result<String, SettingsError> {
    let mut color = Color::default();
    color.parse(value).map_err(|source| SettingsError::Color { field, source })?;
    color.straight();
    Ok(array([color.r, color.g, color.b, color.a].into_iter().map(|c| double_to_string(f64::from(c)))))
}
