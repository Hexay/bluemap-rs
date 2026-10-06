use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;

use super::*;
use crate::manifest::write_verified;

fn v(major: i32, minor: i32) -> PackVersion {
    PackVersion::new(major, minor)
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bm-client-jar-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_jar(path: &Path, version_json: Option<&str>) {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file("assets/.mcassetsroot", SimpleFileOptions::default()).unwrap();
    if let Some(json) = version_json {
        w.start_file("version.json", SimpleFileOptions::default()).unwrap();
        w.write_all(json.as_bytes()).unwrap();
    }
    std::fs::write(path, w.finish().unwrap().into_inner()).unwrap();
}

const MANIFEST: &str = r#"{
  "latest": {"release": "26.3", "snapshot": "26.4-snapshot-1"},
  "versions": [
    {"id": "26.3", "type": "release", "url": "https://piston-meta.mojang.com/v1/26.3.json", "time": "2026-09-15T11:20:48+00:00", "releaseTime": "2026-09-15T11:20:48+00:00"},
    {"id": "1.19.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/1.19.4.json", "time": "2023-03-14T12:56:18+00:00", "releaseTime": "2023-03-14T12:56:18+00:00"},
    {"id": "1.16.5", "type": "release", "url": "https://piston-meta.mojang.com/v1/1.16.5.json", "time": "2021-01-14T16:05:32+00:00", "releaseTime": "2021-01-14T16:05:32+00:00"},
    {"id": "1.13", "type": "release", "url": "https://piston-meta.mojang.com/v1/1.13.json", "time": "2018-07-18T15:11:46+00:00", "releaseTime": "2018-07-18T15:11:46+00:00"},
    {"id": "1.12.2", "type": "release", "url": "https://piston-meta.mojang.com/v1/1.12.2.json", "time": "2017-09-18T08:39:46+00:00", "releaseTime": "2017-09-18T08:39:46+00:00"}
  ]
}"#;

#[test]
fn version_json_pack_versions() {
    let parse = |s| PackVersions::parse_version_json(s).unwrap();
    let full =
        parse(r#"{"pack_version": {"resource_major": 97, "resource_minor": 1, "data_major": 121, "data_minor": 0}}"#);
    assert_eq!(full, PackVersions { resource: v(97, 1), data: v(121, 0) });
    assert_eq!(
        parse(r#"{"pack_version": {"resource": 15, "data": 12}}"#),
        PackVersions { resource: v(15, 0), data: v(12, 0) }
    );
    assert_eq!(parse(r#"{"pack_version": {"data": 12}}"#), PackVersions { resource: v(4, 0), data: v(12, 0) });
    // the old int form only sets the resource major
    assert_eq!(parse(r#"{"pack_version": 6}"#), PackVersions { resource: v(6, 0), data: v(4, 0) });
    assert_eq!(parse(r#"{"id": "1.14"}"#), PackVersions::default());
    assert!(PackVersions::parse_version_json(r#"{"pack_version": "x"}"#).is_err());
}

#[test]
fn jar_selection_uses_release_time_floors() {
    let m = VersionManifest::parse(MANIFEST).unwrap();
    let pick = |id| select_jars(&m, id).map(|s| (s.id, s.resource.id, s.data.id));
    let ids = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
    assert_eq!(pick(None).unwrap(), ids("26.3", "26.3", "26.3"));
    assert_eq!(pick(Some("1.12.2")).unwrap(), ids("1.12.2", "1.13", "1.19.4"));
    assert_eq!(pick(Some("1.16.5")).unwrap(), ids("1.16.5", "1.16.5", "1.19.4"));
    assert!(matches!(pick(Some("1.7")), Err(Error::Manifest(_))));
}

#[test]
fn manifest_validation() {
    let bad_url = MANIFEST.replace("https://piston-meta.mojang.com/v1/1.13.json", "https://evil.example/1.13.json");
    assert!(matches!(VersionManifest::parse(&bad_url), Err(Error::Manifest(_))));
    let bad_id = MANIFEST.replace("\"id\": \"1.13\"", "\"id\": \"../1.13\"");
    assert!(matches!(VersionManifest::parse(&bad_id), Err(Error::Manifest(_))));
    assert!(client_jar_path(Path::new("data"), "a\\b").is_err());
    assert_eq!(
        client_jar_path(Path::new("data"), "26.3").unwrap(),
        Path::new("data").join("minecraft-client-26.3.jar")
    );
}

#[test]
fn release_time_ignores_offsets() {
    let m = VersionManifest::parse(MANIFEST).unwrap();
    let mut a = m.version("1.13").unwrap().clone();
    let mut b = a.clone();
    a.release_time = "2020-01-01T10:00:00+05:00".into();
    b.release_time = "2020-01-01T09:00:00-05:00".into();
    assert_eq!(a.cmp_release(&b), Ordering::Greater);
}

#[test]
fn download_is_verified_before_rename() {
    let dir = temp("verify");
    let file = dir.join("minecraft-client-x.jar");
    let unverified = dir.join("minecraft-client-x.jar.unverified");
    let err = write_verified(&b"abc"[..], &file, "0000000000000000000000000000000000000000").unwrap_err();
    assert!(matches!(err, Error::Checksum { .. }) && !file.exists() && !unverified.exists());
    write_verified(&b"abc"[..], &file, "A9993E364706816ABA3E25717850C26C9CD0D89D").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"abc");
    assert!(!unverified.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn load_falls_back_to_local_jar_without_manifest() {
    let dir = temp("fallback");
    write_jar(&dir.join("minecraft-client-1.20.jar"), Some(r#"{"pack_version": {"resource": 15, "data": 15}}"#));
    let offline = || Err(Error::Http("offline".into()));
    let mc = MinecraftVersion::load_with(offline(), Some("1.20"), &dir, false).unwrap();
    assert_eq!((mc.resource_pack_version, mc.data_pack_version), (v(15, 0), v(15, 0)));
    assert_eq!(mc.resource_pack, mc.data_pack);
    assert!(matches!(MinecraftVersion::load_with(offline(), None, &dir, false), Err(Error::Http(_))));
    assert!(matches!(
        MinecraftVersion::load_with(offline(), Some("1.21"), &dir, false),
        Err(Error::DownloadNotAccepted(_))
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn load_reads_resource_and_data_jars_separately() {
    let dir = temp("split");
    write_jar(&dir.join("minecraft-client-1.13.jar"), None);
    write_jar(&dir.join("minecraft-client-1.19.4.jar"), Some(r#"{"pack_version": {"resource": 13, "data": 12}}"#));
    let manifest = || VersionManifest::parse(MANIFEST);
    let mc = MinecraftVersion::load_with(manifest(), Some("1.12.2"), &dir, true).unwrap();
    assert_eq!(mc.id, "1.12.2");
    assert_eq!((mc.resource_pack_version, mc.data_pack_version), (v(4, 0), v(12, 0)));
    assert!(matches!(
        MinecraftVersion::load_with(manifest(), Some("1.16.5"), &dir, false),
        Err(Error::DownloadNotAccepted(_))
    ));
    std::fs::write(dir.join("minecraft-client-1.13.jar"), b"corrupt").unwrap();
    assert!(MinecraftVersion::load_with(manifest(), Some("1.12.2"), &dir, true).is_err());
    assert!(!dir.join("minecraft-client-1.13.jar").exists() && !dir.join("minecraft-client-1.19.4.jar").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "needs work/bluemap/vanilla/data/minecraft-client-26.3.jar"]
fn real_client_jar_26_3() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data = repo.join("work/bluemap/vanilla/data");
    let offline = Err(Error::Http("offline".into()));
    let mc = MinecraftVersion::load_with(offline, Some("26.3"), &data, false).unwrap();
    assert_eq!((mc.resource_pack_version, mc.data_pack_version), (v(97, 1), v(121, 0)));
    // experimental datapacks shipped in the jar load ahead of vanilla: upstream's feature gate is never set
    let jar: Vec<String> = crate::packs::expand_root(&mc.resource_pack, mc.resource_pack_version)
        .iter()
        .map(|p| p.root().to_owned())
        .collect();
    let nested = ["minecart_improvements", "redstone_experiments", "trade_rebalance"]
        .map(|d| format!("data/minecraft/datapacks/{d}/"));
    assert_eq!(jar, [&nested[..], &[String::new()]].concat());
    let ext = crate::packs::expand_root(&repo.join("assets/resourceExtensions"), mc.resource_pack_version);
    let roots: Vec<&str> = ext.iter().map(|p| p.root()).collect();
    assert_eq!(roots, ["mc26_1/", "mc1_21_9/", "mc1_20_3/", "mc1_17/", "mc1_15/", ""]);
    let data_ext = crate::packs::expand_root(&repo.join("assets/resourceExtensions"), mc.data_pack_version);
    assert_eq!(data_ext.len(), 6);
}

#[test]
#[ignore = "network: fetches Mojang's version manifest"]
fn live_manifest() {
    let m = VersionManifest::fetch().unwrap();
    let sel = select_jars(&m, None).unwrap();
    assert_eq!(sel.resource.id, m.latest.release);
    assert_eq!(select_jars(&m, Some("1.12.2")).unwrap().resource.id, "1.13");
}
