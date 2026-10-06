//! Drop-in acceptance: `bluemap -c <Java BlueMap's config copy> -r` must produce Java BlueMap 5.28's webroot for every
//! golden fixture (map data and settings; webapp files excluded). Incremental checks live in `tools/accept.py`.
//! Run: `cargo test -p bm-cli --release --test acceptance -- --ignored --nocapture`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURES: [&str; 5] = ["vanilla", "structures", "nether", "dimensions", "debug"];
const MC: &str = "26.3";

fn work() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    // `work/` is git-ignored, so a worktree finds it in an enclosing checkout
    here.ancestors().map(|d| d.join("work")).find(|d| d.join("bluemap").is_dir()).expect("work/bluemap")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() { copy_dir(&path, &target) } else { _ = fs::copy(&path, &target).unwrap() }
    }
}

/// Points a relative path setting at the fresh directory (`key: value` lines, as BlueMap's templates write them).
fn set_conf(file: &Path, key: &str, value: &str) {
    let text = fs::read_to_string(file).unwrap();
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| match l.strip_prefix(key).filter(|r| r.trim_start().starts_with(':')) {
            Some(_) => {
                found = true;
                format!("{key}: {value}")
            }
            None => l.to_owned(),
        })
        .collect();
    if !found {
        lines.push(format!("{key}: {value}"));
    }
    fs::write(file, lines.join("\n")).unwrap();
}

fn prepare(fixture: &str) -> PathBuf {
    let src = work().join("bluemap").join(fixture);
    let out = work().join("accept-test").join(fixture);
    _ = fs::remove_dir_all(&out);
    copy_dir(&src.join("config"), &out.join("config"));
    let cfg = out.join("config");
    set_conf(&cfg.join("core.conf"), "data", "\"data\"");
    set_conf(&cfg.join("webapp.conf"), "webroot", "\"web\"");
    set_conf(&cfg.join("webserver.conf"), "webroot", "\"web\"");
    set_conf(&cfg.join("storages/file.conf"), "root", "\"web/maps\"");
    let jar = format!("minecraft-client-{MC}.jar");
    fs::create_dir_all(out.join("data")).unwrap();
    fs::copy(src.join("data").join(&jar), out.join("data").join(&jar)).unwrap();
    out
}

#[test]
#[ignore = "needs Java BlueMap 5.28 golden renders under work/bluemap (tools/render_golden.py)"]
fn renders_like_java_bluemap() {
    let mut failed = Vec::new();
    for fixture in FIXTURES {
        let dir = prepare(fixture);
        let status = Command::new(env!("CARGO_BIN_EXE_bluemap"))
            .args(["-c", "config", "-v", MC, "-r"])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert!(status.success(), "{fixture}: bluemap exited with {status}");
        let golden = work().join("bluemap").join(fixture).join("web");
        let comparison = bm_golden::compare::compare_webroots(&golden, &dir.join("web")).unwrap();
        println!("=== {fixture}\n{comparison}");
        if !comparison.ok() {
            failed.push(fixture);
        }
    }
    assert!(failed.is_empty(), "webroots differ from Java's: {failed:?}");
}
