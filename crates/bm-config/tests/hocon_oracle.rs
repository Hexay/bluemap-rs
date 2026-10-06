//! Every `tests/hocon/*.conf` must parse exactly like BlueMap 5.28's Configurate did (`*.expected`, written by
//! `tools/probe.sh expect`): same JSON tree, or an error on the same line.

use std::path::{Path, PathBuf};

use bm_config::{ConfigError, Value, hocon};
use serde_json::Value as J;

fn normalize(v: J) -> J {
    match v {
        J::Number(n) => J::from(n.as_f64().unwrap()),
        J::Array(items) => J::Array(items.into_iter().map(normalize).collect()),
        J::Object(map) => J::Object(map.into_iter().map(|(k, v)| (k, normalize(v))).collect()),
        other => other,
    }
}

fn expected_error_line(expected: &str) -> Option<usize> {
    let rest = &expected[expected.find("(line ")? + 6..];
    rest[..rest.find(',')?].parse().ok()
}

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hocon");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "conf"))
        .collect();
    files.sort();
    files
}

#[test]
fn corpus_matches_configurate() {
    let mut failures = Vec::new();
    let files = corpus();
    assert!(files.len() >= 15, "corpus missing");
    for conf in files {
        let expected = std::fs::read_to_string(conf.with_extension("expected")).unwrap();
        let expected = expected.trim();
        let ours = hocon::parse_file(&conf);
        let name = conf.file_name().unwrap().to_string_lossy().into_owned();
        if expected.starts_with("ERROR") {
            let line = expected_error_line(expected).expect("oracle errors carry a line");
            match ours {
                Err(ConfigError::Parse(e)) if e.line == line => println!("{name}: {e}"),
                other => failures.push(format!("{name}: expected an error on line {line}, got {other:?}")),
            }
            continue;
        }
        match ours {
            Ok(map) => {
                let got = normalize(Value::Object(map).to_json());
                let want = normalize(serde_json::from_str(expected).unwrap());
                if got != want {
                    failures.push(format!("{name}:\n  want {want}\n  got  {got}"));
                }
            }
            Err(e) => failures.push(format!("{name}: unexpected error {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
