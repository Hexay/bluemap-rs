use serde_json::json;

use super::*;

#[test]
fn strict_json_is_unchanged() {
    let src = r#"{"a": [1, 2.5, -3e2, true, null, "x\"y\u00e9"], "b": {}}"#;
    assert_eq!(parse(src).unwrap(), serde_json::from_str::<Value>(src).unwrap());
}

#[test]
fn gson_lenient_extensions() {
    let src = "\u{feff}{ // comment\n # hash comment\n /* block */ unquoted: 'single', \"eq\" = 1; arrow => [a, 2,], }";
    assert_eq!(parse(src).unwrap(), json!({"unquoted": "single", "eq": 1, "arrow": ["a", 2]}));
}

#[test]
fn empty_array_slots_are_null() {
    assert_eq!(parse("[1,,2]").unwrap(), json!([1, null, 2]));
    assert_eq!(parse("[,1]").unwrap(), json!([null, 1]));
    assert_eq!(parse("[NaN]").unwrap(), json!([null]));
}

#[test]
fn unquoted_values_become_strings_or_numbers() {
    assert_eq!(parse("{x: minecraft:stone}").unwrap_err().msg, "expected ',' or '}'");
    assert_eq!(parse("[1 2]").unwrap_err().msg, "expected ',' or ']'");
    assert_eq!(parse("{x: 'minecraft:stone', y: 0x10, z: .5}").unwrap(), json!({"x": "minecraft:stone", "y": "0x10", "z": 0.5}));
}

#[test]
fn errors_are_reported_not_panicked() {
    for bad in ["{", "[1", "\"abc", "/* x", "{\"a\" 1}", "", "[\"\\u12\"]"] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    let deep = "[".repeat(1000);
    assert_eq!(parse(&deep).unwrap_err().msg, "nested too deeply");
}

#[test]
fn entries_keep_order_and_duplicates_and_survive_errors() {
    let (entries, err) = parse_entries("{b: 1, a: 2, b: 3}");
    assert_eq!(entries, [("b".into(), json!(1)), ("a".into(), json!(2)), ("b".into(), json!(3))]);
    assert!(err.is_none());
    let (entries, err) = parse_entries("{x: 1, y: [}");
    assert_eq!(entries, [("x".into(), json!(1))]);
    assert!(err.is_some());
    assert!(parse_entries("[1]").1.is_some());
}
