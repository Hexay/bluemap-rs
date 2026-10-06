use serde_json::json;

use super::*;

fn v(major: i32, minor: i32) -> PackVersion {
    PackVersion::new(major, minor)
}

#[test]
fn pack_version_forms() {
    assert_eq!(PackVersion::parse_min(&json!(5)).unwrap(), v(5, 0));
    assert_eq!(PackVersion::parse_max(&json!(5)).unwrap(), v(5, i32::MAX));
    assert_eq!(PackVersion::parse_max(&json!(5.0)).unwrap(), v(5, i32::MAX));
    assert_eq!(PackVersion::parse_min(&json!("69.2")).unwrap(), v(69, 2));
    assert_eq!(PackVersion::parse_max(&json!("69")).unwrap(), v(69, i32::MAX));
    assert_eq!(PackVersion::parse_min(&json!(69.1)).unwrap(), v(69, 100_000_000));
    assert_eq!(PackVersion::parse_min(&json!(69.25)).unwrap(), v(69, 250_000_000));
    assert_eq!(PackVersion::parse_min(&json!([7])).unwrap(), v(7, 0));
    assert_eq!(PackVersion::parse_max(&json!([7])).unwrap(), v(7, i32::MAX));
    assert_eq!(PackVersion::parse_max(&json!([7, 3])).unwrap(), v(7, 3));
    for bad in [
        json!("abc"),
        json!("1.2.3"),
        json!("-1"),
        json!(-1.5),
        json!([]),
        json!([1, 2, 3]),
        json!([1.5]),
        json!({}),
        json!(true),
    ] {
        assert!(PackVersion::parse_min(&bad).is_err(), "{bad}");
    }
}

#[test]
fn version_comparisons_are_swapped_like_upstream() {
    let current = v(97, 1);
    assert!(
        current.is_greater_or_equal(v(97, 1))
            && current.is_greater_or_equal(v(98, 0))
            && !current.is_greater_or_equal(v(69, 0))
    );
    assert!(
        current.is_smaller_or_equal(v(97, 0))
            && current.is_smaller_or_equal(v(5, 9))
            && !current.is_smaller_or_equal(v(99, 0))
    );
}

#[test]
fn version_range_forms() {
    let r = |min, max| VersionRange { min_inclusive: min, max_inclusive: max };
    assert_eq!(VersionRange::parse(&json!(22)).unwrap(), r(22, 22));
    assert_eq!(VersionRange::parse(&json!([3, 7, 9])).unwrap(), r(3, 7));
    assert_eq!(VersionRange::parse(&json!({"min_inclusive": 5})).unwrap(), r(5, i32::MAX));
    assert_eq!(VersionRange::parse(&json!({"max_inclusive": "86"})).unwrap(), r(i32::MIN, 86));
    for bad in [json!([3]), json!("5"), json!(1.5), json!({"min_inclusive": "x"})] {
        assert!(VersionRange::parse(&bad).is_err(), "{bad}");
    }
    assert!(VersionRange::default().includes(i32::MIN) && VersionRange::default().includes(i32::MAX));
    assert!(r(5, 7).includes(5) && r(5, 7).includes(7) && !r(5, 7).includes(8));
}

#[test]
fn overlay_inclusion() {
    let meta = PackMeta::parse(
        r#"{ overlays: { entries: [
            { formats: 5, directory: "exact" },
            { formats: [3, 6], directory: "range" },
            { directory: "always" },
            { min_format: 69, directory: "half_new_style_uses_formats" },
            { min_format: 69, max_format: 100, directory: "new_style" },
            { min_format: [98], max_format: "96", directory: "swapped_bounds" },
            { formats: 5 },
        ] } }"#,
    )
    .unwrap();
    let included = |version| {
        meta.overlays
            .iter()
            .filter(|o| o.includes(version))
            .map(|o| o.directory.as_deref().unwrap_or("-"))
            .collect::<Vec<_>>()
    };
    assert_eq!(included(v(5, 0)), ["exact", "range", "always", "half_new_style_uses_formats", "-"]);
    // min ≤ version ≤ max is never true upstream unless the bounds are inverted around it
    assert_eq!(included(v(97, 1)), ["always", "half_new_style_uses_formats", "swapped_bounds"]);
    assert_eq!(included(v(80, 0)), ["always", "half_new_style_uses_formats"]);
}

#[test]
fn malformed_meta_drops_everything() {
    assert!(
        PackMeta::parse(r#"{"pack": {"min_format": "x"}, "overlays": {"entries": [{"directory": "a"}]}}"#).is_err()
    );
    assert!(PackMeta::parse(r#"{"overlays": {"entries": {}}}"#).is_err());
    assert!(PackMeta::parse("[]").is_err());
    let meta = PackMeta::parse(
        r#"{"pack": {"pack_format": 15, "description": "x"}, "overlays": {"entries": [null, {"directory": 3}]}}"#,
    )
    .unwrap();
    assert_eq!(meta.pack.pack_format, VersionRange { min_inclusive: 15, max_inclusive: 15 });
    assert_eq!(meta.overlays.len(), 1);
    assert_eq!(meta.overlays[0].directory.as_deref(), Some("3"));
}

#[test]
fn pack_section_inclusion() {
    let meta = PackMeta::parse(r#"{"pack": {"pack_format": 15, "supported_formats": {"min_inclusive": 20}}}"#).unwrap();
    assert!(meta.pack.includes(v(15, 0)) && meta.pack.includes(v(30, 0)) && !meta.pack.includes(v(16, 0)));
}
