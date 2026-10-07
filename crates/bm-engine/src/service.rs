//! `BlueMapService`: lazily loaded resources and storages for a loaded config, shared by every map.

use std::sync::OnceLock;

use bm_config::BlueMapConfig;
use bm_map::settings::WebappSettings;

use crate::convert;
use crate::error::{Result, io};
use crate::map::MapContext;
use crate::resources::{ResourceOptions, Resources};
use crate::storage::Storages;

/// The BlueMap release this build is a drop-in for; written as the webapp `settings.json` version.
pub const BLUEMAP_VERSION: &str = "5.28";

pub struct Service {
    pub config: BlueMapConfig,
    options: ResourceOptions,
    resources: OnceLock<Resources>,
    storages: Storages,
}

impl Service {
    pub fn new(config: BlueMapConfig, options: ResourceOptions) -> Self {
        Self { config, options, resources: OnceLock::new(), storages: Storages::default() }
    }

    /// `createOrUpdateWebApp` without the webapp files: (re)writes the webroot `settings.json`.
    pub fn write_webapp_settings(&self) -> Result<()> {
        let webapp = convert::webapp_settings(&self.config.webapp);
        let file = webapp.webroot.join("settings.json");
        let existing = match webapp.update_settings_file {
            true => None,
            false => Some(std::fs::read_to_string(&file).map_err(io("read", &file))?),
        };
        let maps: Vec<(&str, i32)> = self.config.maps.iter().map(|(id, m)| (id.as_str(), m.sorting)).collect();
        let settings = WebappSettings::create_or_update(BLUEMAP_VERSION, &webapp, &maps, existing.as_deref())?;
        std::fs::create_dir_all(&webapp.webroot).map_err(io("create", &webapp.webroot))?;
        std::fs::write(&file, settings.to_json()?).map_err(io("write", &file))
    }

    /// Loads the client jar and packs on first use (`getOrLoadResourcePack`).
    pub fn resources(&self) -> Result<&Resources> {
        if let Some(r) = self.resources.get() {
            return Ok(r);
        }
        let loaded = Resources::load(&self.config, &self.options)?;
        Ok(self.resources.get_or_init(|| loaded))
    }

    /// `None` for a map without a world (display-only).
    pub fn open_map(&self, id: &str) -> Result<Option<MapContext>> {
        MapContext::open(id, &self.config, self.resources()?, &self.storages)
    }

    /// The storage of map `id` (`getOrLoadStorage(config.storage).map(id)`), e.g. for the webserver.
    pub fn map_storage(&self, id: &str) -> Result<std::sync::Arc<dyn bm_storage::MapStorage>> {
        let map = self.config.maps.get(id).ok_or_else(|| crate::Error::Invalid(format!("no map '{id}'")))?;
        Ok(self.storages.get(&self.config, &map.storage)?.map(id)?)
    }

    /// Converts storage `id` in place between compat and optimized; nothing else may use it meanwhile.
    pub fn convert_storage(
        &self,
        id: &str,
        to: bm_storage::Format,
        progress: bm_storage::Progress,
    ) -> Result<bm_storage::ConvertStats> {
        self.storages.convert(&self.config, id, to, progress)
    }

    /// `BlueMapCLI.updateMarkers` for one map: its config marker sets, written to its storage. Returns warnings
    /// about skipped invalid markers.
    pub fn write_config_markers(&self, id: &str) -> Result<Vec<String>> {
        let map = self.config.maps.get(id).ok_or_else(|| crate::Error::Invalid(format!("no map '{id}'")))?;
        let mut warnings = Vec::new();
        let order = bm_map::markers::SetOrder::Config;
        crate::map::write_markers_ordered(self.map_storage(id)?.as_ref(), map, order, &mut warnings)?;
        Ok(warnings)
    }

    /// Configured map ids passing `filter`, by `sorting` (stable, ties by id).
    pub fn map_ids(&self, filter: impl Fn(&str) -> bool) -> Vec<String> {
        let mut ids: Vec<(&String, i32)> =
            self.config.maps.iter().filter(|(id, _)| filter(id)).map(|(id, m)| (id, m.sorting)).collect();
        ids.sort_by_key(|&(_, sorting)| sorting);
        ids.into_iter().map(|(id, _)| id.clone()).collect()
    }
}
