//! bm-config's typed configs → the input structs of bm-map's settings writers and mask builder.

use bm_compress::Compression;
use bm_config::{MapConfig, MaskShape, RenderMask, WebappConfig};
use bm_map::mask::{MaskConfig, MaskShape as MapMaskShape};
use bm_map::settings;

pub fn compression(c: bm_config::Compression) -> Compression {
    match c {
        bm_config::Compression::None => Compression::None,
        bm_config::Compression::Gzip => Compression::Gzip,
        bm_config::Compression::Deflate => Compression::Deflate,
        bm_config::Compression::Zstd => Compression::Zstd,
        bm_config::Compression::Lz4 => Compression::Lz4,
    }
}

pub fn format(f: bm_config::StorageFormat) -> bm_storage::Format {
    match f {
        bm_config::StorageFormat::Compat => bm_storage::Format::Compat,
        bm_config::StorageFormat::Optimized => bm_storage::Format::Optimized,
    }
}

pub fn mask_configs(masks: &[RenderMask]) -> Vec<MaskConfig> {
    masks.iter().map(mask_config).collect()
}

fn mask_config(m: &RenderMask) -> MaskConfig {
    let shape = match &m.shape {
        MaskShape::Box { min, max } => MapMaskShape::Box { min: *min, max: *max },
        MaskShape::Ellipse { center, radius, min_y, max_y } => MapMaskShape::Ellipse {
            center_x: center[0],
            center_z: center[1],
            radius_x: radius[0],
            radius_z: radius[1],
            min_y: *min_y,
            max_y: *max_y,
        },
        MaskShape::Polygon { points, min_y, max_y } => {
            MapMaskShape::Polygon { min_y: *min_y, max_y: *max_y, shape: points.clone() }
        }
        MaskShape::Blur { size, masks } => MapMaskShape::Blur { size: *size, masks: mask_configs(masks) },
    };
    MaskConfig { subtract: m.subtract, shape }
}

pub fn map_settings(c: &MapConfig) -> settings::MapConfig {
    settings::MapConfig {
        loader: c.loader.to_string(),
        world: c.world.clone(),
        dimension: c.dimension.as_ref().map(ToString::to_string),
        dimension_type: c.dimension_type.as_ref().map(ToString::to_string),
        name: c.name.clone(),
        sorting: c.sorting,
        start_pos: c.start_pos,
        sky_color: c.sky_color.clone(),
        void_color: c.void_color.clone(),
        ambient_light: c.ambient_light,
        sky_light: c.sky_light,
        remove_caves_below_y: c.remove_caves_below_y,
        cave_detection_ocean_floor: c.cave_detection_ocean_floor,
        cave_detection_uses_block_light: c.cave_detection_uses_block_light,
        render_mask: mask_configs(&c.render_mask),
        min_inhabited_time: c.min_inhabited_time,
        min_inhabited_time_radius: c.min_inhabited_time_radius,
        render_edges: c.render_edges,
        edge_light_strength: c.edge_light_strength,
        enable_perspective_view: c.enable_perspective_view,
        enable_flat_view: c.enable_flat_view,
        enable_free_flight_view: c.enable_free_flight_view,
        enable_hires: c.enable_hires,
        check_for_removed_regions: c.check_for_removed_regions,
        storage: c.storage.clone(),
        ignore_missing_light_data: c.ignore_missing_light_data,
        hires_tile_size: c.hires_tile_size,
        lowres_tile_size: c.lowres_tile_size,
        lod_count: c.lod_count,
        lod_factor: c.lod_factor,
    }
}

pub fn webapp_settings(c: &WebappConfig) -> settings::WebappConfig {
    settings::WebappConfig {
        enabled: c.enabled,
        update_settings_file: c.update_settings_file,
        webroot: c.webroot.clone(),
        use_cookies: c.use_cookies,
        default_to_flat_view: c.default_to_flat_view,
        start_location: c.start_location.clone(),
        resolution_default: c.resolution_default,
        min_zoom_distance: c.min_zoom_distance,
        max_zoom_distance: c.max_zoom_distance,
        hires_slider_max: c.hires_slider_max,
        hires_slider_default: c.hires_slider_default,
        hires_slider_min: c.hires_slider_min,
        lowres_slider_max: c.lowres_slider_max,
        lowres_slider_default: c.lowres_slider_default,
        lowres_slider_min: c.lowres_slider_min,
        map_data_root: c.map_data_root.clone(),
        live_data_root: c.live_data_root.clone(),
        client_decompression: c.client_decompression,
        scripts: c.scripts.clone(),
        styles: c.styles.clone(),
    }
}
