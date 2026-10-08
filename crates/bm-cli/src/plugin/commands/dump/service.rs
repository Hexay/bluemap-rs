//! The `BlueMapService` object: configs, worlds, maps and storages.

use std::collections::HashMap;

use bm_engine::MapContext;
use serde_json::{Value, json};

use super::super::super::core::Core;
use super::super::super::session::{Session, world_id};
use super::configs::{COMMON, config, identity_of, map_config, storage_config};
use super::java::{Ids, seen};

/// A `ConcurrentHashMap` in its iteration order.
fn chm(ids: &mut Ids, entries: Vec<(String, Value)>) -> Value {
    let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    let order: Vec<String> = bm_java::concurrent_hash_map_order(&keys).into_iter().map(str::to_owned).collect();
    let mut by_key: HashMap<String, Value> = entries.into_iter().collect();
    let pairs = order.into_iter().filter_map(|k| by_key.remove(&k).map(|v| (json!(k), v))).collect();
    ids.map("java.util.concurrent.ConcurrentHashMap", pairs)
}

fn config_manager(ids: &mut Ids, core: &Core, s: &Session, map_settings: &mut HashMap<String, String>) -> Value {
    let cfg = &s.service.config;
    let hello = &core.hello;
    let cwd = std::env::current_dir().unwrap_or_default();
    let path = |p: &std::path::Path| bm_config::generate::format_path(p, &cwd);
    let core_config = config(
        ids,
        &format!("{COMMON}.config.CoreConfig"),
        &cfg.core,
        &[("log", &format!("{COMMON}.config.CoreConfig$LogConfig"))],
    );
    let webserver = config(
        ids,
        &format!("{COMMON}.config.WebserverConfig"),
        &cfg.webserver,
        &[
            ("log", &format!("{COMMON}.config.WebserverConfig$LogConfig")),
            ("additionalHeaders", "java.util.LinkedHashMap"),
        ],
    );
    let webapp = config(
        ids,
        &format!("{COMMON}.config.WebappConfig"),
        &cfg.webapp,
        &[("scripts", "java.util.LinkedHashSet"), ("styles", "java.util.LinkedHashSet")],
    );
    let plugin = config(
        ids,
        &format!("{COMMON}.config.PluginConfig"),
        &cfg.plugin,
        &[("hiddenGameModes", "java.util.ArrayList")],
    );
    let map_configs = cfg
        .maps
        .iter()
        .map(|(id, c)| {
            let v = map_config(ids, c);
            map_settings.insert(id.clone(), identity_of(&v));
            (json!(id), v)
        })
        .collect();
    let map_configs = ids.map("java.util.Collections$UnmodifiableMap", map_configs);
    let storage_configs = cfg.storages.iter().map(|(id, c)| (json!(id), storage_config(ids, c))).collect();
    let storage_configs = ids.map("java.util.Collections$UnmodifiableMap", storage_configs);
    ids.object(
        &format!("{COMMON}.config.BlueMapConfigManager"),
        vec![
            ("coreConfig", core_config),
            ("webserverConfig", webserver),
            ("webappConfig", webapp),
            ("pluginConfig", plugin),
            ("mapConfigs", map_configs),
            ("storageConfigs", storage_configs),
            ("packsFolder", json!(path(&hello.config_folder.join("packs")))),
            ("minecraftVersion", json!(hello.mc_version)),
            ("modsFolder", json!(hello.mods_folder.as_deref().map(path))),
        ],
    )
}

pub fn blue_map_service(ids: &mut Ids, core: &Core, s: &Session) -> Value {
    let mut map_settings = HashMap::new();
    let manager = config_manager(ids, core, s, &mut map_settings);
    let cfg = &s.service.config;
    let web_files = ids.object(
        &format!("{COMMON}.WebFilesManager"),
        vec![("webRoot", json!(cfg.webapp.webroot.display().to_string()))],
    );
    let version = ids
        .object("de.bluecolored.bluemap.core.resources.MinecraftVersion", vec![("id", json!(core.hello.mc_version))]);
    let maps = s.maps.all();
    let mut world_ids = HashMap::new();
    let worlds: Vec<(String, Value)> = maps
        .iter()
        .filter_map(|m| {
            let id = world_id(m);
            if world_ids.contains_key(&id) {
                return None;
            }
            let world = ids.object(
                "de.bluecolored.bluemap.core.world.mca.MCAWorld",
                vec![
                    ("id", json!(id)),
                    ("worldFolder", json!(m.world.path.display().to_string())),
                    ("dimension", json!(m.world.dimension.to_string())),
                    ("dimensionFolder", json!(m.world.region_dir().parent().map(|p| p.display().to_string()))),
                ],
            );
            world_ids.insert(id.clone(), identity_of(&world));
            Some((id, world))
        })
        .collect();
    let worlds = chm(ids, worlds);
    let maps = maps
        .iter()
        .map(|m| (m.id.clone(), bm_map(ids, m, &world_ids[&world_id(m)], map_settings.get(&m.id))))
        .collect();
    let maps = chm(ids, maps);
    let storages = cfg.storages.iter().map(|(id, c)| (id.clone(), storage_config(ids, c))).collect();
    let storages = chm(ids, storages);
    ids.object(
        &format!("{COMMON}.BlueMapService"),
        vec![
            ("config", manager),
            ("webFilesManager", web_files),
            ("minecraftVersion", version),
            ("worlds", worlds),
            ("maps", maps),
            ("storages", storages),
        ],
    )
}

fn bm_map(ids: &mut Ids, m: &MapContext, world: &str, settings: Option<&String>) -> Value {
    ids.object(
        "de.bluecolored.bluemap.core.map.BmMap",
        vec![
            ("id", json!(m.id)),
            ("name", json!(m.config.name.clone().unwrap_or_else(|| m.id.clone()))),
            ("world", seen(world)),
            ("storage", json!(m.config.storage)),
            ("mapSettings", settings.map_or(Value::Null, |s| seen(s))),
            ("warnings", json!(m.warnings)),
        ],
    )
}
