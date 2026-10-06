//! `settings.json` byte parity with Java BlueMap 5.28 for every fixture under `work/bluemap/<fixture>`
//! (configs in `config/`, outputs in `web/`). Configs are read with a throwaway flat-HOCON reader.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use bm_map::settings::{MapConfig, WebappConfig, WebappSettings, map_settings_json, sanitize_map_id};

/// Top-level `key: value` pairs; values keep their raw text (quotes stripped), inline objects/arrays verbatim.
fn read_flat_hocon(path: &Path) -> HashMap<String, String> {
    let text = fs::read_to_string(path).unwrap();
    let mut out = HashMap::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let mut value = value.trim().to_owned();
        let open = |v: &str| v.matches(['[', '{']).count() > v.matches([']', '}']).count();
        while open(&value) {
            let next = lines.next().unwrap_or("]").trim();
            if !next.starts_with('#') {
                value.push('\n');
                value.push_str(next);
            }
        }
        out.insert(key.trim().to_owned(), value.trim_matches('"').to_owned());
    }
    out
}

fn string_list(raw: &str) -> Vec<String> {
    raw.trim_matches(['[', ']'])
        .split([',', '\n'])
        .map(|s| s.trim().trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn map_config(kv: &HashMap<String, String>) -> MapConfig {
    let mut c = MapConfig::default();
    let get = |k: &str| kv.get(k).map(String::as_str);
    let flag = |k: &str, d: bool| get(k).map_or(d, |v| v == "true");
    c.name = get("name").map(str::to_owned);
    c.sorting = get("sorting").map_or(0, |v| v.parse().unwrap());
    if let Some(pos) = get("start-pos") {
        let num = |axis: &str| {
            let rest = &pos[pos.find(&format!("{axis}:")).unwrap() + 2..];
            rest.trim_start().split([',', ' ', '}']).next().unwrap().parse().unwrap()
        };
        c.start_pos = [num("x"), num("z")];
    }
    c.sky_color = get("sky-color").unwrap_or(&c.sky_color).to_owned();
    c.void_color = get("void-color").unwrap_or(&c.void_color).to_owned();
    c.ambient_light = get("ambient-light").map_or(c.ambient_light, |v| v.parse().unwrap());
    c.sky_light = get("sky-light").map_or(c.sky_light, |v| v.parse().unwrap());
    c.enable_perspective_view = flag("enable-perspective-view", true);
    c.enable_flat_view = flag("enable-flat-view", true);
    c.enable_free_flight_view = flag("enable-free-flight-view", true);
    c
}

fn webapp_config(kv: &HashMap<String, String>) -> WebappConfig {
    let mut c = WebappConfig::default();
    let int = |k: &str, d: i32| kv.get(k).map_or(d, |v| v.parse().unwrap());
    let flag = |k: &str, d: bool| kv.get(k).map_or(d, |v| v == "true");
    c.update_settings_file = flag("update-settings-file", true);
    c.use_cookies = flag("use-cookies", true);
    c.default_to_flat_view = flag("default-to-flat-view", false);
    c.start_location = kv.get("start-location").cloned();
    c.resolution_default = kv.get("resolution-default").map_or(1.0, |v| v.parse().unwrap());
    c.min_zoom_distance = int("min-zoom-distance", c.min_zoom_distance);
    c.max_zoom_distance = int("max-zoom-distance", c.max_zoom_distance);
    c.hires_slider_max = int("hires-slider-max", c.hires_slider_max);
    c.hires_slider_default = int("hires-slider-default", c.hires_slider_default);
    c.hires_slider_min = int("hires-slider-min", c.hires_slider_min);
    c.lowres_slider_max = int("lowres-slider-max", c.lowres_slider_max);
    c.lowres_slider_default = int("lowres-slider-default", c.lowres_slider_default);
    c.lowres_slider_min = int("lowres-slider-min", c.lowres_slider_min);
    c.client_decompression = flag("client-decompression", false);
    c.scripts = kv.get("scripts").map_or_else(Vec::new, |v| string_list(v));
    c.styles = kv.get("styles").map_or_else(Vec::new, |v| string_list(v));
    c
}

fn fixtures() -> Vec<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    // `work/` is git-ignored, so a worktree finds it in an enclosing checkout
    let root = here.ancestors().map(|d| d.join("work/bluemap")).find(|d| d.is_dir()).expect("work/bluemap");
    let mut out: Vec<PathBuf> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("config/maps").is_dir() && p.join("web/settings.json").is_file())
        .collect();
    out.sort();
    out
}

#[test]
#[ignore = "needs Java BlueMap 5.28 golden webroots under work/bluemap"]
fn settings_json_matches_java() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty());
    let mut compared = 0;
    for fx in &fixtures {
        let mut maps = Vec::new();
        // insertion order = Files.list order of config/maps
        for entry in fs::read_dir(fx.join("config/maps")).unwrap() {
            let path = entry.unwrap().path();
            let id = sanitize_map_id(path.file_stem().unwrap().to_str().unwrap());
            let config = map_config(&read_flat_hocon(&path));
            let golden = fx.join("web/maps").join(&id).join("settings.json");
            let ours = map_settings_json(&id, &config).unwrap();
            assert_eq!(ours, fs::read_to_string(&golden).unwrap(), "{}", golden.display());
            compared += 1;
            maps.push((id, config.sorting));
        }
        let webapp = webapp_config(&read_flat_hocon(&fx.join("config/webapp.conf")));
        let map_refs: Vec<(&str, i32)> = maps.iter().map(|(id, s)| (id.as_str(), *s)).collect();
        let golden = fx.join("web/settings.json");
        let existing = fs::read_to_string(&golden).unwrap();
        let settings = WebappSettings::create_or_update("5.28", &webapp, &map_refs, Some(&existing)).unwrap();
        assert_eq!(settings.to_json().unwrap(), existing, "{}", golden.display());
        compared += 1;
    }
    eprintln!("{compared} settings.json files byte-identical across {} fixtures", fixtures.len());
}
