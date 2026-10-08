//! Object-mapping behaviour checked against BlueMap 5.28's Configurate (`tools/probe.sh typed <Class> <file>`);
//! each case states what the real jar produced.

use bm_config::{
    ConfigError, CoreConfig, Key, MapConfig, MaskShape, PluginConfig, StorageConfig, StorageFormat, Value,
    WebappConfig, WebserverConfig, from_value, hocon,
};

fn parse(src: &str) -> Value {
    Value::Object(hocon::parse_str(src, "test.conf").unwrap())
}

fn map(src: &str) -> MapConfig {
    from_value(&parse(src)).unwrap()
}

fn map_err(src: &str) -> String {
    from_value::<MapConfig>(&parse(src)).unwrap_err().to_string()
}

#[test]
fn empty_files_give_java_defaults() {
    let empty = parse("");
    assert_eq!(from_value::<CoreConfig>(&empty).unwrap(), CoreConfig::default());
    assert_eq!(from_value::<WebserverConfig>(&empty).unwrap().log.format, r#"%1$s "%3$s %4$s %5$s" %6$s %7$s"#);
    assert_eq!(from_value::<WebappConfig>(&empty).unwrap().max_zoom_distance, 100000);
    assert_eq!(from_value::<PluginConfig>(&empty).unwrap().player_render_limit, -1);
    let m = map("");
    assert_eq!((m.cave_detection_ocean_floor, m.edge_light_strength, m.lod_factor), (10000, 15, 5));
    assert_eq!(m.loader, Key::bluemap("anvil"));
    assert!(m.world.is_none() && m.dimension.is_none() && m.render_mask.is_empty());
    match StorageConfig::from_value(&empty).unwrap() {
        StorageConfig::File(f) => {
            assert_eq!((f.compression.as_str(), f.format), ("bluemap:gzip", StorageFormat::Compat))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn lenient_scalars_like_configurate() {
    // jar: sorting=5, ambientLight=0.5, name=5, renderEdges=true, enableHires=false, enableFlatView=false,
    // startPos=(0, 3), minInhabitedTime=3000000000, dimension=minecraft:the_nether, storage=7
    let m = map(r#"
        sorting: "5"
        ambient-light: "0.5"
        name: 5
        render-edges: "yes"
        enable-hires: 0
        enable-flat-view: "f"
        start-pos: { x: 1.7, z: "3" }
        unknown-key: 1
        min-inhabited-time: 3000000000
        dimension: "the_nether"
        dimension-type: "foo:bar"
        loader: "anvil"
        storage: 7
        sky-color: null
    "#);
    assert_eq!((m.sorting, m.ambient_light, m.name.as_deref()), (5, 0.5, Some("5")));
    assert!(m.render_edges && !m.enable_hires && !m.enable_flat_view);
    assert_eq!(m.start_pos, [0, 3]);
    assert_eq!(m.min_inhabited_time, 3_000_000_000);
    assert_eq!(m.dimension, Some(Key::minecraft("the_nether")));
    assert_eq!(m.dimension_type, Some(Key::new("foo", "bar")));
    assert_eq!(m.storage, "7");
    assert_eq!(m.sky_color, "#7dabff", "null keeps the default");
    // jar: sorting=2 (whole double), startPos=(3, 4) (y wins over z), edgeLightStrength=7, name="true"
    let m = map(
        "sorting: 2.0\nstart-pos: { x: 3, y: 4, z: 5 }\nedge-light-strength: \"+7\"\nname: true\nsky-light: 0.30000001",
    );
    assert_eq!((m.sorting, m.start_pos, m.edge_light_strength, m.name.as_deref()), (2, [3, 4], 7, Some("true")));
    assert_eq!(m.sky_light, 0.3);
}

#[test]
fn type_errors_like_configurate() {
    // jar: CoercionFailedException / NumberFormatException / "Value must be provided as a scalar!" / out of range
    for (src, key) in [
        ("sorting: 1.5", "sorting"),
        ("sorting: \"abc\"", "sorting"),
        ("edge-light-strength: \"7.0\"", "edge-light-strength"),
        ("render-edges: \"maybe\"", "render-edges"),
        ("name: [1, 2]", "name"),
        ("loader: \"foo\"", "loader"),
        ("sorting: 2147483648", "sorting"),
        ("ambient-light: \"abc\"", "ambient-light"),
        ("start-pos: [1, 2]", "start-pos"),
        ("start-pos: { x: 3 }", "start-pos"),
    ] {
        let err = map_err(src);
        assert!(err.starts_with(&format!("{key}: ")), "{src} → {err}");
    }
    let err = from_value::<PluginConfig>(&parse("hidden-game-modes: [1, true, \"x\", {a: 1}]")).unwrap_err();
    assert_eq!(err.key(), "hidden-game-modes.3");
}

#[test]
fn lists_and_maps() {
    // jar: hiddenGameModes=[spectator] from a single string; [1, true, x] as strings; writeMarkersInterval=16
    let p: PluginConfig =
        from_value(&parse("hidden-game-modes: \"spectator\"\nwrite-markers-interval: \"0x10\"")).unwrap();
    assert_eq!((p.hidden_game_modes, p.write_markers_interval), (vec!["spectator".to_owned()], 16));
    let p: PluginConfig =
        from_value(&parse("hidden-game-modes: [1, true, \"x\"]\nskin-download: 2\nhide-invisible: \"n\"")).unwrap();
    assert_eq!(p.hidden_game_modes, ["1", "true", "x"]);
    assert!(p.skin_download && !p.hide_invisible);
    let w: WebappConfig = from_value(&parse("scripts: [\"a.js\", \"b.js\", \"a.js\"]")).unwrap();
    assert_eq!(w.scripts, ["a.js", "b.js"]);
    let s: WebserverConfig = from_value(&parse("additional-headers { \"X-A\": 1, \"X-B\": null }\nlog: 5")).unwrap();
    assert_eq!(s.additional_headers, [("X-A".to_owned(), "1".to_owned())]);
    assert!(s.log.file.is_none(), "a scalar for a section leaves its defaults");
}

#[test]
fn render_masks() {
    let m = map(r#"render-mask: [
        { min-x: -10, max-x: 10 }
        { type: circle, center-x: 1, radius: 5, subtract: true }
        { type: "bluemap:ellipse", radius-x: 2, radius-z: 3 }
        { type: polygon, shape: [{x: 0, z: 0}, {x: 1, y: 0}, {x: 1, z: 1}] }
        { type: blur, size: 2, masks: [ { min-y: 0 } ] }
    ]"#);
    let shapes: Vec<&MaskShape> = m.render_mask.iter().map(|r| &r.shape).collect();
    assert_eq!(*shapes[0], MaskShape::Box { min: [-10, i32::MIN, i32::MIN], max: [10, i32::MAX, i32::MAX] });
    assert_eq!(
        *shapes[1],
        MaskShape::Ellipse { center: [1.0, 0.0], radius: [5.0, 5.0], min_y: i32::MIN, max_y: i32::MAX }
    );
    assert!(m.render_mask[1].subtract);
    assert!(matches!(shapes[2], MaskShape::Ellipse { radius: [2.0, 3.0], .. }));
    assert!(matches!(shapes[3], MaskShape::Polygon { points, .. } if points == &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]));
    assert!(matches!(shapes[4], MaskShape::Blur { size: 2, masks } if masks.len() == 1));
    // jar: a non-list render-mask is ignored; unknown types and degenerate boxes fail
    assert!(map("render-mask: { min-x: 1 }").render_mask.is_empty());
    assert!(map_err("render-mask: [ { type: bogus } ]").contains("no mask-type found for key: bogus"));
    assert!(map_err("render-mask: [ { min-x: 10, max-x: 5 } ]").contains("degenerate"));
    assert!(map_err("render-mask: [ { type: polygon, shape: [{x: 0, z: 0}] } ]").contains("at least 3 points"));
}

#[test]
fn storages() {
    let sql = StorageConfig::from_value(&parse(
        "storage-type: SQL\nconnection-url: \"jdbc:sqlite:bluemap.db\"\ncompression: ZSTD\nformat: optimized",
    ))
    .unwrap();
    let StorageConfig::Sql(s) = &sql else { panic!("{sql:?}") };
    assert_eq!(s.dialect(), Ok(bm_config::Dialect::Sqlite));
    assert_eq!(s.connection_init_sql().unwrap().len(), 4);
    assert_eq!(sql.compression(), Ok(bm_config::Compression::Zstd), "legacy upper-case keys still resolve");
    assert_eq!(sql.format(), StorageFormat::Optimized);
    assert!(StorageConfig::from_value(&parse("storage-type: s3")).unwrap_err().to_string().contains("storage-type"));
    assert!(StorageConfig::from_value(&parse("format: fast")).is_err());
    let StorageConfig::Sql(bad) =
        StorageConfig::from_value(&parse("storage-type: sql\ntable-prefix: \"Bad-\"")).unwrap()
    else {
        panic!()
    };
    assert!(bad.table_prefix().is_err());
}

#[test]
fn hidden_throttle_keys() {
    let core = |src: &str| from_value::<CoreConfig>(&parse(src)).map(|c| c.memory_limit);
    assert_eq!(core(""), Ok(None));
    assert_eq!(core("memory-limit: 0"), Ok(None));
    assert_eq!(core("memory-limit: 1048576"), Ok(Some(1 << 20)));
    assert_eq!(core("memory-limit: \"1G\""), Ok(Some(1 << 30)));
    assert_eq!(core("memory-limit: 512MiB"), Ok(Some(512 << 20)));
    assert_eq!(core("memory-limit: 512 M"), Ok(Some(512 << 20)));
    let err = core("memory-limit: lots").unwrap_err();
    assert_eq!(err.key(), "memory-limit");
    assert!(err.to_string().contains("not a memory size"), "{err}");
    assert!(core("memory-limit: -5").is_err());

    let plugin = from_value::<PluginConfig>(&parse("")).unwrap();
    assert_eq!((plugin.render_pause_mspt, plugin.render_resume_mspt), (45.0, 40.0));
    let plugin = from_value::<PluginConfig>(&parse("render-pause-mspt: 0\nrender-resume-mspt: 30.5")).unwrap();
    assert_eq!((plugin.render_pause_mspt, plugin.render_resume_mspt), (0.0, 30.5));
}

#[test]
fn map_errors_name_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.conf");
    std::fs::write(&path, "min-x: 5").unwrap();
    let err = bm_config::load_map_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid { .. }) && err.to_string().contains("outdated"), "{err}");
    std::fs::write(&path, "sorting: \"high\"").unwrap();
    let err = bm_config::load_map_config(&path).unwrap_err().to_string();
    assert!(err.contains("world.conf") && err.contains("'sorting'"), "{err}");
}
