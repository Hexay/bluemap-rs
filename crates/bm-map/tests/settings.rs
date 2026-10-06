//! `settings.json` writers on synthetic inputs; expected strings copied from Java BlueMap 5.28 output.

use bm_map::settings::{MapConfig, SettingsError, WebappConfig, WebappSettings, map_settings_json, sanitize_map_id};

const VANILLA_MAP: &str = r#"{"name":"vanilla","sorting":0,"hires":{"tileSize":[32,32],"scale":[1,1],"translate":[2,2]},"lowres":{"tileSize":[500,500],"lodFactor":5,"lodCount":3},"startPos":[0,0],"skyColor":[0.4901960790157318,0.6705882549285889,1.0,1.0],"voidColor":[0.0,0.0,0.0,1.0],"ambientLight":0.1,"skyLight":1.0,"perspectiveView":true,"flatView":true,"freeFlightView":true}"#;
const NETHER_MAP: &str = r#"{"name":"Nether","sorting":100,"hires":{"tileSize":[32,32],"scale":[1,1],"translate":[2,2]},"lowres":{"tileSize":[500,500],"lodFactor":5,"lodCount":3},"startPos":[0,0],"skyColor":[0.16078431904315948,0.0,0.0,1.0],"voidColor":[0.08235294371843338,0.0,0.0,1.0],"ambientLight":0.6,"skyLight":1.0,"perspectiveView":true,"flatView":true,"freeFlightView":true}"#;
const DIMENSIONS_ROOT: &str = r#"{"version":"5.28","useCookies":true,"defaultToFlatView":false,"resolutionDefault":1.0,"minZoomDistance":5,"maxZoomDistance":100000,"hiresSliderMax":500,"hiresSliderDefault":100,"hiresSliderMin":0,"lowresSliderMax":7000,"lowresSliderDefault":2000,"lowresSliderMin":500,"mapDataRoot":"maps","liveDataRoot":"maps","clientDecompression":false,"maps":["overworld","nether","end"],"scripts":[],"styles":[]}"#;

#[test]
fn map_settings_match_java() {
    let vanilla = MapConfig { ambient_light: 0.1, ..MapConfig::default() };
    assert_eq!(map_settings_json("vanilla", &vanilla).unwrap(), VANILLA_MAP);
    let nether = MapConfig {
        name: Some("Nether".into()),
        sorting: 100,
        sky_color: "#290000".into(),
        void_color: "#150000".into(),
        ambient_light: 0.6,
        ..MapConfig::default()
    };
    assert_eq!(map_settings_json("nether", &nether).unwrap(), NETHER_MAP);
}

#[test]
fn map_settings_quirks() {
    let config = MapConfig {
        name: Some("<a & 'b'>".into()),
        start_pos: [-5, 12],
        sky_color: "#ff000080".into(),
        ambient_light: 1e-4,
        sky_light: f32::NAN,
        hires_tile_size: 64,
        lod_count: 2,
        enable_flat_view: false,
        ..MapConfig::default()
    };
    let json = map_settings_json("x", &config).unwrap();
    let html_safe = ["003ca ", "0026 ", "0027b", "0027", "003e"].map(|e| format!("\\u{e}")).concat();
    assert!(json.starts_with(&format!("{{\"name\":\"{html_safe}\",")), "{json}");
    assert!(json.contains(r#""hires":{"tileSize":[64,64],"scale":[1,1],"translate":[2,2]}"#), "{json}");
    assert!(json.contains(r#""lodCount":2}"#) && json.contains(r#""startPos":[-5,12]"#), "{json}");
    assert!(json.contains(r#""skyColor":[1.0,0.0,0.0,0.501960813999176]"#), "{json}");
    assert!(json.contains(r#""ambientLight":1.0E-4,"skyLight":NaN"#) && json.contains(r#""flatView":false"#), "{json}");
    let bad = MapConfig { void_color: "nope".into(), ..MapConfig::default() };
    assert!(matches!(map_settings_json("x", &bad), Err(SettingsError::Color { field: "void-color", .. })));
}

#[test]
fn root_settings_match_java() {
    let maps = [("end", 200), ("nether", 100), ("overworld", 0)];
    let settings = WebappSettings::create_or_update("5.28", &WebappConfig::default(), &maps, None).unwrap();
    assert_eq!(settings.to_json().unwrap(), DIMENSIONS_ROOT);
}

#[test]
fn equal_sortings_keep_hash_map_order() {
    // String.hashCode buckets in a 16-slot table: "b" → 2, "a" → 1, "c" → 3, "q" → 1 (inserted after "a")
    let maps = [("c", 0), ("q", 0), ("b", 0), ("a", 0), ("z", -1)];
    let settings = WebappSettings::create_or_update("v", &WebappConfig::default(), &maps, None).unwrap();
    assert_eq!(settings.maps, ["z", "q", "a", "b", "c"]);
}

#[test]
fn keep_existing_file_when_not_updating() {
    let config = WebappConfig {
        update_settings_file: false,
        scripts: vec!["js/a.js".into(), "js/a.js".into()],
        start_location: Some("ignored".into()),
        ..WebappConfig::default()
    };
    let existing =
        r#"{"version":"5.1","hiresSliderDefault":7,"maps":["old"],"scripts":["js/b.js"],"startLocation":"x"}"#;
    let settings = WebappSettings::create_or_update("5.28", &config, &[("new", 0)], Some(existing)).unwrap();
    let json = settings.to_json().unwrap();
    assert!(json.starts_with(r#"{"version":"5.1","useCookies":true,"defaultToFlatView":false,"startLocation":"x","#));
    assert!(
        json.contains(r#""hiresSliderMax":500,"hiresSliderDefault":7,"hiresSliderMin":50,"lowresSliderMax":10000,"#)
    );
    assert!(json.ends_with(r#""maps":["old","new"],"scripts":["js/b.js","js/a.js"],"styles":[]}"#), "{json}");
    assert!(matches!(
        WebappSettings::create_or_update("5.28", &config, &[], None),
        Err(SettingsError::MissingSettingsFile)
    ));
}

#[test]
fn start_location_and_float_resolution() {
    let config =
        WebappConfig { start_location: Some("world:1:2:3".into()), resolution_default: 0.1, ..WebappConfig::default() };
    let json = WebappSettings::create_or_update("5.28", &config, &[], None).unwrap().to_json().unwrap();
    assert!(json.contains(r#""startLocation":"world:1:2:3","resolutionDefault":0.1,"#), "{json}");
    let nan = WebappConfig { resolution_default: f32::INFINITY, ..WebappConfig::default() };
    let settings = WebappSettings::create_or_update("5.28", &nan, &[], None).unwrap();
    assert!(matches!(settings.to_json(), Err(SettingsError::NonFinite(_))));
}

#[test]
fn map_ids_from_file_names() {
    assert_eq!(sanitize_map_id("vanilla-512"), "vanilla_512");
    assert_eq!(sanitize_map_id("My World.v2"), "My_World_v2");
    assert_eq!(sanitize_map_id("wörld😀"), "w_rld_");
}
