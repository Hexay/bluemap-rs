//! `live/markers.json` against Java BlueMap 5.28: each `data/markers/<name>.conf` was appended to a map config and
//! written by the CLI with `--markers` (`<name>.cli.json`) and with `-r` (`<name>.render.json`), on JDK 25.

use std::path::Path;

use bm_map::markers::{MarkerError, SetOrder, marker_sets_json};

fn config_json(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/markers").join(format!("{name}.conf"));
    bm_config::load_map_config(&path).unwrap().marker_sets_json()
}

fn golden(name: &str, mode: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/data/markers/{name}.{mode}.json"));
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn matches_java_byte_for_byte() {
    for name in ["all-types", "numbers"] {
        let config = config_json(name);
        for (mode, order) in [("cli", SetOrder::Config), ("render", SetOrder::Map)] {
            let ours = marker_sets_json(&config, order).unwrap();
            assert_eq!(ours.warnings, Vec::<String>::new(), "{name}.{mode}");
            assert!(
                ours.json == golden(name, mode),
                "{name}.{mode}:\n  ours {}\n  java {}",
                ours.json,
                golden(name, mode)
            );
        }
    }
}

#[test]
fn bad_markers_and_sets_are_skipped_not_fatal() {
    let config = r##"{"a":{"markers":{"ok":{"type":"poi"},"unknown":{"type":"circle"},"untyped":{},
        "short":{"type":"line","line":[{"x":1}]},"bad-int":{"type":"poi","sorting":"x"},
        "nan":{"type":"poi","position":{"x":"NaN"}},"color":{"type":"line","line-color":"#ff0000"}}},
        "b":{"sorting":1.5},"c":5}"##;
    let out = marker_sets_json(config, SetOrder::Config).unwrap();
    assert!(
        out.json
            .starts_with(r#"{"a":{"label":"","toggleable":true,"defaultHidden":false,"sorting":0,"markers":{"ok":{"#)
    );
    assert!(out.json.ends_with(r#""listed":true}}}}"#), "{}", out.json);
    assert_eq!(out.warnings.len(), 8, "{:#?}", out.warnings);
    assert!(out.warnings[0].contains("Unknown marker type: circle"));
}

#[test]
fn empty_and_invalid_roots() {
    assert_eq!(marker_sets_json("{}", SetOrder::Map).unwrap().json, "{}");
    assert!(matches!(marker_sets_json("[]", SetOrder::Map), Err(MarkerError::NotAnObject(_))));
    assert!(matches!(marker_sets_json("{", SetOrder::Map), Err(MarkerError::Json(_))));
}
