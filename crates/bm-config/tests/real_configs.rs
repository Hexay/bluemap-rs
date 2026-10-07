//! Configs written by Java BlueMap 5.28 (CLI, then edited by tools/render_serve.py) load into typed structs.

mod common;

use std::path::{Path, PathBuf};

use bm_config::{BlueMapConfig, ConfigOptions, Dialect, Key, MaskShape, StorageConfig, StorageFormat};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let dest = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

/// Loads a copy, so the read-only originals are never touched even if something were missing.
fn load(fixture: &Path) -> BlueMapConfig {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&fixture.join("config"), tmp.path());
    let mut opts = ConfigOptions::cli(tmp.path());
    opts.working_dir = fixture.to_owned();
    let before = std::fs::read_dir(tmp.path()).unwrap().count();
    let cfg = BlueMapConfig::load(&opts).unwrap_or_else(|e| panic!("{}: {e}", fixture.display()));
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), before, "loading generated files");
    cfg
}

#[test]
fn every_generated_config_loads() {
    let fixtures = common::bluemap_fixtures();
    let mut maps = 0;
    for fixture in &fixtures {
        let cfg = load(fixture);
        let name = fixture.file_name().unwrap().to_string_lossy();

        assert!(cfg.core.accept_download, "{name}");
        assert!(!cfg.core.metrics);
        assert_eq!(cfg.core.data, PathBuf::from("data"));
        assert!(cfg.core.render_thread_count >= 1);
        assert_eq!(cfg.core.render_thread_priority, 5, "commented out in the template → Java default");
        assert_eq!(cfg.core.log.file.as_deref(), Some("data/logs/debug.log"));
        assert_eq!(cfg.webapp.webroot, PathBuf::from("web"));
        assert_eq!(cfg.webapp.scripts, Vec::<String>::new());
        assert_eq!(cfg.webapp.lowres_slider_max, 7000);
        assert_eq!(cfg.webserver.webroot, PathBuf::from("web"));
        assert_eq!(cfg.webserver.ip, "127.0.0.1");
        assert!(cfg.webserver.port > 0);
        assert_eq!(cfg.webserver.log.format, r#"%1$s "%3$s %4$s %5$s" %6$s %7$s"#);
        assert_eq!(cfg.webserver.additional_headers[0].0, "Cache-Control");
        assert_eq!(cfg.webserver.additional_headers[1], ("CDN-Cache-Control".into(), "max-age=60".into()));

        match &cfg.storages["file"] {
            StorageConfig::File(f) => {
                assert_eq!(f.root, PathBuf::from("web/maps"));
                assert_eq!(f.compression, "gzip");
                assert!(f.atomic);
                assert_eq!(f.format, StorageFormat::Compat, "BlueMap-made storages have no format key");
            }
            other => panic!("{other:?}"),
        }
        match &cfg.storages["sql"] {
            StorageConfig::Sql(s) => {
                assert_eq!(s.dialect(), Ok(Dialect::Mysql));
                assert_eq!(
                    s.connection_properties,
                    [("user".into(), "root".into()), ("password".into(), String::new())]
                );
                assert_eq!(s.table_prefix(), Ok("bluemap_"));
                assert_eq!(s.connection_init_sql(), Ok(vec![]));
            }
            other => panic!("{other:?}"),
        }

        for (id, map) in &cfg.maps {
            maps += 1;
            let dim = map.dimension.clone().expect("templates set the dimension");
            assert!(map.world.is_some() && map.name.is_some(), "{name}/{id}");
            assert_eq!(map.cave_detection_ocean_floor, -5);
            assert_eq!(map.edge_light_strength, 8);
            assert_eq!(map.storage, "file");
            assert_eq!(map.start_pos, [0, 0]);
            assert_eq!(map.marker_sets_json(), "{}");
            // the template's `{ #min-x ... }` entry is a box without bounds
            let full = MaskShape::Box { min: [i32::MIN; 3], max: [i32::MAX; 3] };
            assert_eq!(map.render_mask[0].shape, full);
            if dim == Key::minecraft("the_nether") {
                assert_eq!(map.sky_color, "#290000");
                assert_eq!(map.ambient_light, 0.6);
                assert_eq!(map.remove_caves_below_y, -10000);
                let ceiling = &map.render_mask[1];
                assert!(ceiling.subtract);
                assert_eq!(
                    ceiling.shape,
                    MaskShape::Box { min: [i32::MIN, 90, i32::MIN], max: [i32::MAX, 127, i32::MAX] }
                );
            } else {
                assert_eq!(map.render_mask.len(), 1);
            }
        }
    }
    println!("loaded {} BlueMap config folders, {maps} maps", fixtures.len());
}
