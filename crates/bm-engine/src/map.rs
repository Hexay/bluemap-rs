//! `BlueMapService.loadMap` + the `BmMap` constructor: world, storage, mask, texture gallery and the files a map
//! writes when it loads (`textures.json`, `settings.json`, `live/*.json`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bm_config::{BlueMapConfig, Key, MapConfig};
use bm_format::grid::Grid;
use bm_map::mask::{Mask, build_render_mask};
use bm_map::settings::map_settings_json;
use bm_render::RenderSettings;
use bm_resources::datapack::BiomeTable;
use bm_resources::texture::TextureGallery;
use bm_storage::{ItemKey, MapStorage};
use bm_world::{DimensionType, World};

use crate::convert;
use crate::error::{Error, Result};
use crate::resources::Resources;
use crate::storage::Storages;

/// `BmMap`'s hires grid offset (`new Grid(hiresTileSize, 2)`).
const HIRES_OFFSET: i32 = 2;

pub struct MapContext {
    pub id: String,
    pub config: MapConfig,
    pub world: World,
    pub biome_table: BiomeTable,
    pub storage: Arc<dyn MapStorage>,
    pub gallery: TextureGallery,
    pub mask: Mask,
    pub render: RenderSettings,
    pub hires_grid: Grid,
    /// Problems BlueMap only logs (e.g. an unreadable `textures.json`).
    pub warnings: Vec<String>,
}

impl MapContext {
    /// `None` for a map without `world`: display-only, served from storage but never rendered.
    pub fn open(
        id: &str,
        config: &BlueMapConfig,
        resources: &Resources,
        storages: &Storages,
    ) -> Result<Option<Self>> {
        let map = &config.maps[id];
        let Some(configured) = &map.world else { return Ok(None) };
        let (world_folder, dimension) = world_and_dimension(configured, map.dimension.as_ref());
        if !world_folder.is_dir() {
            let abs = std::path::absolute(&world_folder).unwrap_or(world_folder.clone());
            return Err(Error::Invalid(format!(
                "'{}' does not exist or is no directory!\nCheck if the 'world' setting in the config-file for that map \
                 is correct, or remove the entire config-file if you don't want that map.",
                abs.display()
            )));
        }
        let datapack = resources.world_datapack(&world_folder)?;
        let dimension_type = |k: &str| datapack.dimension_type(k).cloned();
        let mut world = World::open(
            &world_folder,
            &dimension.formatted(),
            resources.states.clone(),
            resources.biomes.clone(),
            &dimension_type,
        )?;
        if let Some(key) = &map.dimension_type {
            world.dimension_type = dimension_type(&key.formatted()).unwrap_or(DimensionType::OVERWORLD);
        }
        let biome_table = datapack.biome_table(&resources.biomes);
        let storage = storages.get(config, &map.storage)?.map(id)?;
        let mask = bm_map::mask::Mask::Combined(build_render_mask(&convert::mask_configs(&map.render_mask))?);

        let mut warnings = Vec::new();
        let gallery = load_gallery(storage.as_ref(), resources, id, &mut warnings)?;
        let settings = map_settings_json(id, &convert::map_settings(map))?;
        storage.write_item(&ItemKey::Settings, settings.as_bytes())?;
        storage.write_item(&ItemKey::Players, b"{}")?;
        write_markers(storage.as_ref(), map)?;

        Ok(Some(Self {
            id: id.to_owned(),
            render: render_settings(map, &mask),
            config: map.clone(),
            world,
            biome_table,
            storage,
            gallery,
            mask,
            hires_grid: Grid { size: [map.hires_tile_size; 2], offset: [HIRES_OFFSET; 2] },
            warnings,
        }))
    }

    pub fn save_hires(&self) -> bool {
        self.config.enable_hires
    }
}

/// Without `dimension`, the world folder may itself be a dimension folder (`DIM-1`, `DIM1`, `dimensions/ns/val`).
fn world_and_dimension(world: &Path, dimension: Option<&Key>) -> (PathBuf, Key) {
    if let Some(d) = dimension {
        return (world.to_owned(), d.clone());
    }
    let parts: Vec<String> = world.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let n = parts.len();
    let parent = |levels: usize| world.ancestors().nth(levels).unwrap_or(world).to_owned();
    match parts.last().map(String::as_str) {
        Some("DIM-1") => (parent(1), Key::minecraft("the_nether")),
        Some("DIM1") => (parent(1), Key::minecraft("the_end")),
        _ if n > 3 && parts[n - 3] == "dimensions" => (parent(3), Key::new(&parts[n - 2], &parts[n - 1])),
        _ => (world.to_owned(), Key::minecraft("overworld")),
    }
}

/// The stored gallery (ids stay stable), plus every texture of the pack, written back right away.
fn load_gallery(
    storage: &dyn MapStorage,
    resources: &Resources,
    id: &str,
    warnings: &mut Vec<String>,
) -> Result<TextureGallery> {
    let stored = storage.read_item(&ItemKey::Textures)?;
    let mut gallery = match stored.map(|s| s.decompress()).transpose() {
        Ok(Some(json)) => TextureGallery::read_textures_file(&json).unwrap_or_else(|e| {
            warnings.push(format!("Failed to load textures for map '{id}': {e}"));
            TextureGallery::new()
        }),
        Ok(None) => TextureGallery::new(),
        Err(e) => {
            warnings.push(format!("Failed to load textures for map '{id}': {e}"));
            TextureGallery::new()
        }
    };
    gallery.put_pool(&resources.pack.textures);
    let mut json = String::new();
    gallery.write_textures_file(&mut json);
    storage.write_item(&ItemKey::Textures, json.as_bytes())?;
    Ok(gallery)
}

/// `MarkerGson` of the config's marker sets; only the empty case is written byte-exact so far.
fn write_markers(storage: &dyn MapStorage, map: &MapConfig) -> Result<()> {
    let markers = map.marker_sets_json();
    // TODO: MarkerGson's marker serialization for non-empty `marker-sets`
    let json = if markers.as_object().is_some_and(|m| m.is_empty()) { "{}".to_owned() } else { markers.to_string() };
    storage.write_item(&ItemKey::Markers, json.as_bytes())?;
    Ok(())
}

fn render_settings(c: &MapConfig, mask: &Mask) -> RenderSettings {
    RenderSettings {
        remove_caves_below_y: c.remove_caves_below_y,
        cave_detection_ocean_floor: c.cave_detection_ocean_floor,
        cave_detection_uses_block_light: c.cave_detection_uses_block_light,
        ambient_light: c.ambient_light,
        render_edges: c.render_edges,
        edge_light_strength: c.edge_light_strength,
        ignore_missing_light_data: c.ignore_missing_light_data,
        render_top_only: !c.enable_hires || (!c.enable_perspective_view && !c.enable_free_flight_view),
        mask: crate::mask::render_mask(mask),
    }
}
