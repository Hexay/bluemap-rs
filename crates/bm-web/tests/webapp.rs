//! Webroot setup: bundled files written like `WebFilesManager.updateFiles`, user files kept.

use bm_map::settings::WebappConfig;
use bm_web::{WEBAPP_VERSION, install_webapp, webapp_files, write_settings};

#[test]
fn install_writes_bundled_files_and_keeps_user_files() {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(web.join("custom")).unwrap();
    std::fs::write(web.join("custom/script.js"), b"user").unwrap();
    std::fs::write(web.join("settings.json"), b"{\"scripts\":[\"custom/script.js\"]}").unwrap();

    assert!(install_webapp(&web, false).unwrap());
    let files: Vec<String> = webapp_files().map(|f| f.into_owned()).collect();
    for must in ["index.html", "sql.php", "lang/en.conf", "lang/settings.conf", "assets/index-BhhubAws.js"] {
        assert!(files.iter().any(|f| f == must), "{must} bundled");
        assert!(web.join(must).is_file(), "{must} written");
    }
    assert!(!files.iter().any(|f| f.ends_with(".filepart") || f == "settings.json"));
    assert_eq!(std::fs::read(web.join("custom/script.js")).unwrap(), b"user");
    assert_eq!(std::fs::read(web.join("settings.json")).unwrap(), b"{\"scripts\":[\"custom/script.js\"]}");

    std::fs::write(web.join("index.html"), b"edited").unwrap();
    assert!(!install_webapp(&web, false).unwrap(), "index.html present: nothing to do");
    assert_eq!(std::fs::read(web.join("index.html")).unwrap(), b"edited");
    assert!(install_webapp(&web, true).unwrap(), "forced (-g) overwrites bundled files");
    assert_ne!(std::fs::read(web.join("index.html")).unwrap(), b"edited");
    assert_eq!(std::fs::read(web.join("custom/script.js")).unwrap(), b"user");
}

#[test]
fn settings_merge_or_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path();
    let config = WebappConfig { scripts: vec!["a.js".into()], ..WebappConfig::default() };
    write_settings(web, WEBAPP_VERSION, &config, &[("world", 0), ("nether", -1)]).unwrap();
    let json = std::fs::read_to_string(web.join("settings.json")).unwrap();
    assert!(json.starts_with("{\"version\":\"5.28\""), "{json}");
    assert!(json.contains("\"maps\":[\"nether\",\"world\"]") && json.contains("\"scripts\":[\"a.js\"]"), "{json}");

    std::fs::write(web.join("settings.json"), json.replace("\"scripts\":[\"a.js\"]", "\"scripts\":[\"mine.js\"]"))
        .unwrap();
    let keep = WebappConfig { update_settings_file: false, ..config };
    write_settings(web, WEBAPP_VERSION, &keep, &[("world", 0)]).unwrap();
    let merged = std::fs::read_to_string(web.join("settings.json")).unwrap();
    assert!(merged.contains("\"scripts\":[\"mine.js\",\"a.js\"]"), "{merged}");

    let empty = tempfile::tempdir().unwrap();
    assert!(write_settings(empty.path(), WEBAPP_VERSION, &keep, &[]).is_err(), "nothing to keep");
}
