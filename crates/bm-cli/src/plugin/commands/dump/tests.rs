//! Shapes checked against a `dump.json` from upstream BlueMap 5.28 on Paper 26.3 (docs/13 §8).

use bm_config::{CoreConfig, PluginConfig, WebappConfig, WebserverConfig};
use serde_json::Value;

use super::configs::config;
use super::java::{Ids, registries};
use super::plugin::{duration, instant};

fn fields(v: &Value) -> Vec<&str> {
    v.as_object().unwrap().keys().map(String::as_str).filter(|k| !k.starts_with('#')).collect()
}

/// Upstream's fields appear in upstream's order; ours (bluemap-rs keys) may follow or sit in between.
fn assert_upstream_fields(v: &Value, upstream: &[&str]) {
    let ours = fields(v);
    let common: Vec<&str> = ours.iter().copied().filter(|k| upstream.contains(k)).collect();
    assert_eq!(common, upstream, "{ours:?}");
}

#[test]
fn configs_use_java_field_names() {
    let mut ids = Ids::default();
    let core = config(&mut ids, "CoreConfig", &CoreConfig::default(), &[("log", "CoreConfig$LogConfig")]);
    assert_upstream_fields(
        &core,
        &[
            "acceptDownload",
            "renderThreadCount",
            "renderThreadPriority",
            "updateCooldown",
            "fullUpdateInterval",
            "metrics",
            "data",
            "scanForModResources",
            "log",
        ],
    );
    assert_eq!(fields(&core["log"]), ["file", "append"]);
    assert!(core["log"]["#identity"].as_str().unwrap().starts_with("CoreConfig$LogConfig@"));
    let web = WebserverConfig {
        additional_headers: vec![("Cache-Control".into(), "max-age=0".into())],
        ..WebserverConfig::default()
    };
    let web = config(&mut ids, "W", &web, &[("additionalHeaders", "java.util.LinkedHashMap")]);
    assert_upstream_fields(&web, &["enabled", "webroot", "ip", "port", "sseEnabled", "log", "additionalHeaders"]);
    assert_eq!(web["additionalHeaders"]["size"], 1);
    assert_eq!(web["additionalHeaders"]["entries"][0]["key"], "Cache-Control");
    let app = config(&mut ids, "A", &WebappConfig::default(), &[("scripts", "java.util.LinkedHashSet")]);
    assert_upstream_fields(
        &app,
        &[
            "enabled",
            "updateSettingsFile",
            "webroot",
            "useCookies",
            "defaultToFlatView",
            "startLocation",
            "resolutionDefault",
            "minZoomDistance",
            "maxZoomDistance",
            "hiresSliderMax",
            "hiresSliderDefault",
            "hiresSliderMin",
            "lowresSliderMax",
            "lowresSliderDefault",
            "lowresSliderMin",
            "mapDataRoot",
            "liveDataRoot",
            "clientDecompression",
            "scripts",
            "styles",
        ],
    );
    assert_eq!(app["scripts"]["size"], 0);
    let plugin = config(&mut ids, "P", &PluginConfig::default(), &[]);
    assert_upstream_fields(
        &plugin,
        &[
            "livePlayerMarkers",
            "hiddenGameModes",
            "hideVanished",
            "hideInvisible",
            "hideSneaking",
            "hideDifferentWorld",
            "hideBelowSkyLight",
            "hideBelowBlockLight",
            "writeMarkersInterval",
            "writePlayersInterval",
            "skinDownload",
            "playerRenderLimit",
        ],
    );
}

#[test]
fn registries_match_upstream() {
    let regs = registries(&mut Ids::default());
    assert_eq!(regs.len(), 15);
    let keys = |i: usize| -> Vec<&str> {
        regs[i]["keys"]["entries"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect()
    };
    assert_eq!(keys(10), ["bluemap:zstd", "bluemap:lz4", "bluemap:gzip", "bluemap:deflate", "bluemap:none"]);
    let none = &regs[10]["entries"]["entries"][4]["value"];
    assert!(
        none["#identity"]
            .as_str()
            .unwrap()
            .starts_with("de.bluecolored.bluemap.core.storage.compression.NoCompression@")
    );
    assert_eq!((none["id"].as_str(), none["fileSuffix"].as_str()), (Some("none"), Some("")));
    let tile_state = &regs[12]["entries"]["entries"][0]["value"];
    assert_eq!(fields(tile_state), ["key"]);
    assert_eq!(tile_state["#toString"], "bluemap:render-error");
    let value_ref = regs[11]["values"]["entries"][0].as_str().unwrap();
    let file = regs[11]["entries"]["entries"][0]["value"]["#identity"].as_str().unwrap();
    assert_eq!(value_ref, format!("<<{file}>>"));
}

#[test]
fn java_time_strings() {
    assert_eq!(duration(86_400), "PT24H");
    assert_eq!(duration(60), "PT1M");
    assert_eq!(duration(3_690), "PT1H1M30S");
    assert_eq!(duration(0), "PT0S");
    assert_eq!(instant(1_791_396_299), "2026-10-07T18:04:59Z");
}
