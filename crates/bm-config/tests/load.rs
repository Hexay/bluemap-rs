//! Whole-folder loading: first-start generation (CLI and server), `.json` configs, map ids.

use std::path::{Path, PathBuf};

use bm_config::generate::ServerWorld;
use bm_config::{BlueMapConfig, ConfigError, ConfigOptions, Key, StorageConfig, StorageFormat};

fn options(dir: &Path, opts: ConfigOptions) -> ConfigOptions {
    ConfigOptions {
        working_dir: dir.to_owned(),
        timestamp: "2026-10-06T12:00:01".into(),
        render_thread_count: 2,
        ..opts
    }
}

fn files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            out.extend(files(&path).into_iter().map(|f| format!("{name}/{f}")));
        } else {
            out.push(name);
        }
    }
    out.sort();
    out
}

#[test]
fn fresh_cli_install() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = BlueMapConfig::load(&options(dir.path(), ConfigOptions::cli(dir.path().join("config")))).unwrap();
    assert_eq!(
        files(&dir.path().join("config")),
        [
            "core.conf",
            "maps/end.conf",
            "maps/nether.conf",
            "maps/overworld.conf",
            "storages/file.conf",
            "storages/sql.conf",
            "webapp.conf",
            "webserver.conf"
        ]
    );
    assert_eq!(cfg.core.data, PathBuf::from("data"));
    assert_eq!(cfg.core.render_thread_count, 2);
    assert!(!cfg.core.accept_download);
    assert_eq!(cfg.webserver.log.file.as_deref(), Some("data/logs/webserver.log"));
    for storage in cfg.storages.values() {
        assert_eq!(storage.format(), StorageFormat::Optimized, "new installs default to optimized");
    }
    let StorageConfig::File(file) = &cfg.storages["file"] else { panic!() };
    assert_eq!(file.root, PathBuf::from("web/maps"));
    assert_eq!(cfg.maps["nether"].world, Some(PathBuf::from("world")));
    assert_eq!(cfg.maps["end"].sorting, 200);

    let core = std::fs::read_to_string(dir.path().join("config/core.conf")).unwrap();
    assert!(core.contains("# 2026-10-06T12:00:01\n") && core.contains("Only when the -u flag is used."));
    assert!(core.contains("{\"implementation\":\"bukkit\",\"version\":\"5.28\",\"mcVersion\":\"?\"}"));

    // a second start changes nothing
    let again = BlueMapConfig::load(&options(dir.path(), ConfigOptions::cli(dir.path().join("config")))).unwrap();
    assert_eq!(again, cfg);
}

#[test]
fn fresh_server_install() {
    let dir = tempfile::tempdir().unwrap();
    let world = |folder: &str, dim: &str| ServerWorld {
        world_folder: dir.path().join(folder),
        dimension: Key::parse(dim),
        dimension_type: None,
    };
    let worlds = vec![world("world_nether", "the_nether"), world("world", "overworld")];
    let root = dir.path().join("plugins/BlueMap");
    let cfg = BlueMapConfig::load(&options(dir.path(), ConfigOptions::server(&root, worlds, true))).unwrap();
    assert!(root.join("plugin.conf").is_file());
    assert_eq!(cfg.plugin.hidden_game_modes, ["spectator"]);
    assert_eq!(cfg.core.data, PathBuf::from("bluemap"));
    assert!(!std::fs::read_to_string(root.join("core.conf")).unwrap().contains("metrics:"));
    assert_eq!(cfg.webapp.webroot, PathBuf::from("bluemap/web"));
    assert_eq!(cfg.maps.keys().collect::<Vec<_>>(), ["world", "world_nether"]);
    assert_eq!(cfg.maps["world_nether"].name.as_deref(), Some("world_nether (the_nether)"));
    assert_eq!(cfg.maps["world_nether"].world, Some(PathBuf::from("world_nether")));
    assert_eq!(cfg.maps["world_nether"].sorting, 100);
}

#[test]
fn json_configs_and_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("config");
    BlueMapConfig::load(&options(dir.path(), ConfigOptions::cli(&root))).unwrap();
    std::fs::write(root.join("core.json"), r#"{ "accept-download": true, "render-thread-count": -2 }"#).unwrap();
    std::fs::write(root.join("maps/extra.json"), r#"{ "world": "other", "name": "Extra" }"#).unwrap();
    let cfg = BlueMapConfig::load(&options(dir.path(), ConfigOptions::cli(&root))).unwrap();
    assert!(cfg.core.accept_download, "core.json wins over core.conf, as in BlueMap");
    assert_eq!(cfg.core.resolve_render_thread_count(8), 6);
    assert_eq!(cfg.maps["extra"].name.as_deref(), Some("Extra"));
}

#[test]
fn ambiguous_map_ids_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("config");
    std::fs::create_dir_all(root.join("maps")).unwrap();
    std::fs::write(root.join("maps/my-map.conf"), "").unwrap();
    std::fs::write(root.join("maps/my_map.conf"), "").unwrap();
    let err = BlueMapConfig::load(&options(dir.path(), ConfigOptions::cli(&root))).unwrap_err();
    assert!(matches!(&err, ConfigError::Invalid { message, .. } if message.contains("'my_map'")), "{err}");
}
