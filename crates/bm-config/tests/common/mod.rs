#![allow(dead_code)] // each test binary uses a subset

use std::path::{Path, PathBuf};

/// `work/bluemap/<fixture>` dirs holding configs Java BlueMap 5.28 generated (`tools/render_serve.py`). Found by
/// walking up from the crate, so it works from git worktrees too. Empty when the golden renders don't exist.
pub fn bluemap_fixtures() -> Vec<PathBuf> {
    let Some(root) =
        Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join("work/bluemap")).find(|p| p.is_dir())
    else {
        eprintln!("no work/bluemap found; run tools/render_golden.py to get real BlueMap configs");
        return vec![];
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("config/core.conf").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// Port of `render_serve.py`'s `set_conf`: replace every `key: ...` line, or append one.
pub fn set_conf(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}: {value}");
    let prefix = format!("{key}:");
    let mut found = false;
    let replaced: Vec<&str> = text
        .split('\n')
        .map(|l| {
            if l.starts_with(&prefix) {
                found = true;
                line.as_str()
            } else {
                l
            }
        })
        .collect();
    if found { replaced.join("\n") } else { format!("{}\n{line}\n", text.trim_end()) }
}

/// The value of the first `key: value` line at column 0.
pub fn conf_line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
}

pub fn read_lf(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}
