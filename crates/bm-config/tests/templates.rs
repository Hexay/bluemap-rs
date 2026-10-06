//! Our first-start files equal what Java BlueMap 5.28's CLI generated, once the edits tools/render_serve.py made
//! afterwards (`set_conf`) are replayed on ours and our `format` block is set aside.

mod common;

use std::path::Path;

use bm_config::generate::{self, DimensionPreset};
use bm_config::{Key, load_map_config};
use common::{conf_line, read_lf, set_conf};

const FORMAT_BLOCK: &str = include_str!("../templates/storage-format.conf");

/// Replays the harness edits: each `key` takes the value it has in the real file.
fn replay(mut ours: String, real: &str, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = conf_line(real, key) {
            ours = set_conf(&ours, key, value);
        }
    }
    ours
}

fn assert_same(fixture: &Path, file: &str, ours: &str, real: &str) {
    if ours != real {
        let line = ours
            .lines()
            .zip(real.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(ours.lines().count().min(real.lines().count()));
        panic!(
            "{}/{file} differs at line {}:\n ours: {:?}\n real: {:?}",
            fixture.display(),
            line + 1,
            ours.lines().nth(line),
            real.lines().nth(line)
        );
    }
}

fn strip_format(generated: String) -> String {
    generated.strip_suffix(FORMAT_BLOCK).expect("storage configs end with the format block").to_owned()
}

#[test]
fn cli_templates_match_bluemap() {
    let fixtures = common::bluemap_fixtures();
    let mut compared = 0;
    for fixture in &fixtures {
        let cfg = fixture.join("config");
        let (data, web) = (Path::new("data"), Path::new("web"));

        let real = read_lf(&cfg.join("core.conf"));
        let timestamp = real.lines().find_map(|l| l.strip_prefix("# 20")).map(|t| format!("20{t}")).unwrap();
        let ours = generate::core_conf(data, fixture, true, true, 1, &timestamp);
        let ours = replay(ours, &real, &["accept-download", "metrics", "render-thread-count"]);
        assert_same(fixture, "core.conf", &ours, &real);

        let real = read_lf(&cfg.join("webserver.conf"));
        let ours = replay(generate::webserver_conf(web, data, fixture), &real, &["ip", "port"]);
        assert_same(fixture, "webserver.conf", &ours, &real);

        let real = read_lf(&cfg.join("webapp.conf"));
        assert_same(fixture, "webapp.conf", &generate::webapp_conf(web, fixture), &real);

        let real = read_lf(&cfg.join("storages/file.conf"));
        assert_same(fixture, "storages/file.conf", &strip_format(generate::file_storage_conf(web, fixture)), &real);
        let real = read_lf(&cfg.join("storages/sql.conf"));
        assert_same(fixture, "storages/sql.conf", &strip_format(generate::sql_storage_conf()), &real);

        for entry in std::fs::read_dir(cfg.join("maps")).unwrap() {
            let path = entry.unwrap().path();
            let real = read_lf(&path);
            let dim = load_map_config(&path).unwrap().dimension.unwrap();
            let (preset, name, sorting) = match dim.value() {
                "overworld" => (DimensionPreset::Overworld, "Overworld", 0),
                "the_nether" => (DimensionPreset::Nether, "Nether", 100),
                _ => (DimensionPreset::End, "End", 200),
            };
            let ours = generate::map_conf(preset, name, Path::new("world"), &dim, &dim, sorting, fixture);
            let ours = replay(ours, &real, &["world", "name"]);
            assert_same(fixture, &path.file_name().unwrap().to_string_lossy(), &ours, &real);
            compared += 1;
        }
        compared += 5;
    }
    println!("{} fixtures, {compared} generated files identical to BlueMap's", fixtures.len());
}

#[test]
fn default_maps_are_overworld_nether_end() {
    let maps = generate::default_map_configs(Path::new("/srv"));
    let ids: Vec<&str> = maps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["overworld", "nether", "end"]);
    assert!(maps[1].1.contains("    subtract: true\n    min-y: 90\n    max-y: 127\n"));
    assert!(!maps[0].1.contains("dimension-type:"));
}

#[test]
fn server_variant_writes_plugin_conf_and_dimension_type() {
    assert_eq!(generate::plugin_conf(), include_str!("../templates/bluemap/plugin.conf"));
    let dim = Key::parse("mymod:mining");
    let conf = generate::map_conf(
        DimensionPreset::Overworld,
        "w",
        Path::new("w"),
        &dim,
        &Key::parse("overworld"),
        300,
        Path::new("."),
    );
    assert!(conf.contains("dimension: \"mymod:mining\"\n"));
    assert!(conf.contains("dimension-type: \"minecraft:overworld\"\n"));
    let core = generate::core_conf(Path::new("bluemap"), Path::new("."), false, false, 2, "2026-01-01T00:00");
    assert!(!core.contains("metrics:"));
    assert!(!core.contains("-u flag"));
    assert!(core.contains("data: \"bluemap\"\n") && core.contains("render-thread-count: 2\n"));
}
