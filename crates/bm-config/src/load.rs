//! Loading a config folder like `BlueMapConfigManager`: missing files are generated from the templates first,
//! then every file is parsed and mapped onto its typed config.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::config::{
    CoreConfig, LEGACY_MESSAGE, MapConfig, PluginConfig, StorageConfig, WebappConfig, WebserverConfig,
};
use crate::de::{DeError, from_value};
use crate::error::ConfigError;
use crate::generate::{self, ServerWorld};
use crate::hocon;
use crate::value::Value;

/// Where and how to load (and, on first start, generate) the config folder.
#[derive(Debug, Clone)]
pub struct ConfigOptions {
    pub config_root: PathBuf,
    /// Generated paths are written relative to this (Java: the process working directory).
    pub working_dir: PathBuf,
    pub is_cli: bool,
    /// Load `plugin.conf` (server platforms); otherwise plugin defaults.
    pub use_plugin_config: bool,
    /// Write the `metrics` setting into a new `core.conf` (false when the platform decides).
    pub use_metrics_config: bool,
    pub default_data_folder: PathBuf,
    pub default_webroot: PathBuf,
    /// Server worlds for generating `maps/`; empty → `overworld`/`nether`/`end` of `./world`.
    pub auto_config_worlds: Vec<ServerWorld>,
    /// Written into a new `core.conf`.
    pub render_thread_count: i32,
    /// The `# <timestamp>` line of a new `core.conf`.
    pub timestamp: String,
}

impl ConfigOptions {
    /// BlueMapCLI: data in `data`, webroot `web`, no `plugin.conf`.
    pub fn cli(config_root: impl Into<PathBuf>) -> Self {
        Self {
            config_root: config_root.into(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            is_cli: true,
            use_plugin_config: false,
            use_metrics_config: true,
            default_data_folder: PathBuf::from("data"),
            default_webroot: PathBuf::from("web"),
            auto_config_worlds: Vec::new(),
            render_thread_count: default_render_threads(),
            timestamp: utc_timestamp(),
        }
    }

    /// Server plugin/mod: data in `bluemap`, webroot `bluemap/web`, `plugin.conf`, one map per world.
    pub fn server(config_root: impl Into<PathBuf>, worlds: Vec<ServerWorld>, platform_decides_metrics: bool) -> Self {
        Self {
            is_cli: false,
            use_plugin_config: true,
            use_metrics_config: !platform_decides_metrics,
            default_data_folder: PathBuf::from("bluemap"),
            default_webroot: PathBuf::from("bluemap/web"),
            auto_config_worlds: worlds,
            ..Self::cli(config_root)
        }
    }
}

fn default_render_threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    generate::suggest_render_thread_count(cores, None)
}

/// std has no time zone database, so generated timestamps are UTC.
fn utc_timestamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    generate::java_local_date_time(secs, 0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlueMapConfig {
    pub core: CoreConfig,
    pub webserver: WebserverConfig,
    pub webapp: WebappConfig,
    pub plugin: PluginConfig,
    /// By map id (file name with non-word characters replaced by `_`).
    pub maps: BTreeMap<String, MapConfig>,
    /// By storage id (file name without extension).
    pub storages: BTreeMap<String, StorageConfig>,
}

const SUFFIXES: [&str; 2] = [".conf", ".json"];

/// `ConfigManager.resolveConfigFile`. BlueMap's loader registry is a ConcurrentHashMap whose iteration puts
/// `.json` before `.conf`, so when both exist the `.json` file wins.
pub fn resolve_config_file(root: &Path, name: &str) -> PathBuf {
    let json = root.join(format!("{name}.json"));
    if json.is_file() {
        return json;
    }
    root.join(format!("{name}.conf"))
}

fn config_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    SUFFIXES.iter().find_map(|s| name.strip_suffix(s)).map(str::to_owned)
}

fn value_error(file: &Path, e: DeError) -> ConfigError {
    ConfigError::Value { file: file.to_owned(), key: e.key(), message: e.message }
}

/// Parses `path` (HOCON or JSON) and maps it onto `T` with Configurate's rules.
pub fn load_config<T: DeserializeOwned>(path: &Path) -> Result<T, ConfigError> {
    let root = Value::Object(hocon::parse_file(path)?);
    from_value(&root).map_err(|e| value_error(path, e))
}

pub fn load_map_config(path: &Path) -> Result<MapConfig, ConfigError> {
    let map: MapConfig = load_config(path)?;
    map.check_legacy().map_err(|_| ConfigError::invalid(path, LEGACY_MESSAGE))?;
    Ok(map)
}

pub fn load_storage_config(path: &Path) -> Result<StorageConfig, ConfigError> {
    let root = Value::Object(hocon::parse_file(path)?);
    StorageConfig::from_value(&root).map_err(|e| value_error(path, e))
}

fn write_new(path: &Path, content: &str) -> Result<(), ConfigError> {
    let write = || {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, content)
    };
    write().map_err(|source| ConfigError::Write { path: path.to_owned(), source })
}

fn load_or_generate<T: DeserializeOwned>(path: &Path, generate: impl FnOnce() -> String) -> Result<T, ConfigError> {
    if !path.exists() {
        write_new(path, &generate())?;
    }
    load_config(path)
}

/// Config files in `dir`, sorted by name (Java lists in directory order).
fn config_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, ConfigError> {
    let read_err = |source| ConfigError::Read { path: dir.to_owned(), source };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(read_err)? {
        let path = entry.map_err(read_err)?.path();
        if let Some(name) = config_name(&path).filter(|_| path.is_file()) {
            files.push((name, path));
        }
    }
    files.sort();
    Ok(files)
}

impl BlueMapConfig {
    /// Loads the folder in BlueMap's order, generating whatever is missing (later templates use earlier values).
    pub fn load(opts: &ConfigOptions) -> Result<Self, ConfigError> {
        let (root, cwd) = (&opts.config_root, &opts.working_dir);
        let core: CoreConfig = load_or_generate(&resolve_config_file(root, "core"), || {
            let data = &opts.default_data_folder;
            generate::core_conf(
                data,
                cwd,
                opts.use_metrics_config,
                opts.is_cli,
                opts.render_thread_count,
                &opts.timestamp,
            )
        })?;
        let webapp: WebappConfig = load_or_generate(&resolve_config_file(root, "webapp"), || {
            generate::webapp_conf(&opts.default_webroot, cwd)
        })?;
        let webserver: WebserverConfig = load_or_generate(&resolve_config_file(root, "webserver"), || {
            generate::webserver_conf(&webapp.webroot, &core.data, cwd)
        })?;
        let plugin = match opts.use_plugin_config {
            true => load_or_generate(&resolve_config_file(root, "plugin"), generate::plugin_conf)?,
            false => PluginConfig::default(),
        };

        let storage_dir = root.join("storages");
        if !storage_dir.exists() {
            write_new(&storage_dir.join("file.conf"), &generate::file_storage_conf(&webapp.webroot, cwd))?;
            write_new(&storage_dir.join("sql.conf"), &generate::sql_storage_conf())?;
        }
        let mut storages = BTreeMap::new();
        for (id, path) in config_files(&storage_dir)? {
            storages.insert(id, load_storage_config(&path)?);
        }

        let map_dir = root.join("maps");
        if !map_dir.exists() {
            std::fs::create_dir_all(&map_dir).map_err(|source| ConfigError::Write { path: map_dir.clone(), source })?;
            let generated = match opts.auto_config_worlds.is_empty() {
                true => generate::default_map_configs(cwd),
                false => generate::auto_map_configs(&opts.auto_config_worlds, cwd),
            };
            for (id, content) in generated {
                write_new(&map_dir.join(format!("{id}.conf")), &content)?;
            }
        }
        let mut maps = BTreeMap::new();
        for (name, path) in config_files(&map_dir)? {
            let id = generate::sanitise_map_id(&name);
            if maps.contains_key(&id) {
                let msg = format!("at least two map-config file names result in the map id '{id}'; rename this file");
                return Err(ConfigError::invalid(&path, msg));
            }
            maps.insert(id, load_map_config(&path)?);
        }

        Ok(Self { core, webserver, webapp, plugin, maps, storages })
    }
}
