//! Whole-webroot comparison of a bluemap-rs render against Java BlueMap's: file listing, hires (byte-identical
//! decompressed PRBM, plus the face diff), lowres pixels per LOD, JSON files byte for byte, render state by meaning.
//! Only map data and the root `settings.json` are compared; the webapp files come from the webserver.

mod rstate;
mod tiles;

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::webroot::read_decompressed;

/// One aspect's checks; failed checks make the comparison fail.
#[derive(Default)]
pub struct Section {
    pub lines: Vec<(bool, String)>,
}

impl Section {
    pub fn check(&mut self, ok: bool, line: String) {
        self.lines.push((ok, line));
    }

    /// Context for the last check, indented.
    pub fn detail(&mut self, line: String) {
        self.lines.push((true, format!("    {line}")));
    }

    pub fn ok(&self) -> bool {
        self.lines.iter().all(|(ok, _)| *ok)
    }
}

pub struct Comparison {
    pub sections: Vec<(String, Section)>,
}

impl Comparison {
    pub fn ok(&self) -> bool {
        self.sections.iter().all(|(_, s)| s.ok())
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (name, section) in &self.sections {
            writeln!(f, "== {name}: {}", if section.ok() { "OK" } else { "DIFFERENT" })?;
            for (ok, line) in &section.lines {
                writeln!(f, "  {} {line}", if *ok { " " } else { "!" })?;
            }
        }
        write!(f, "overall: {}", if self.ok() { "IDENTICAL" } else { "DIFFERENT" })
    }
}

pub fn compare_webroots(golden: &Path, candidate: &Path) -> Result<Comparison> {
    let mut sections = Vec::new();
    let mut listing = Section::default();
    let (g, c) = (data_files(golden)?, data_files(candidate)?);
    let missing: Vec<&String> = g.difference(&c).collect();
    let extra: Vec<&String> = c.difference(&g).collect();
    listing.check(missing.is_empty() && extra.is_empty(), format!(
        "{} files golden, {} candidate: {} missing, {} extra",
        g.len(), c.len(), missing.len(), extra.len()
    ));
    missing.iter().take(10).for_each(|m| listing.detail(format!("missing {m}")));
    extra.iter().take(10).for_each(|m| listing.detail(format!("extra {m}")));
    sections.push(("listing".to_owned(), listing));

    let mut json = Section::default();
    same_bytes(&mut json, golden, candidate, "settings.json")?;
    let maps: BTreeSet<String> = list_dirs(&golden.join("maps"))?.union(&list_dirs(&candidate.join("maps"))?).cloned().collect();
    for id in &maps {
        for item in ["settings.json", "textures.json", "live/markers.json", "live/players.json"] {
            same_bytes(&mut json, golden, candidate, &format!("maps/{id}/{item}"))?;
        }
    }
    sections.push(("json".to_owned(), json));

    for id in &maps {
        let (gm, cm) = (golden.join("maps").join(id), candidate.join("maps").join(id));
        let mut hires = Section::default();
        tiles::hires(golden, candidate, id, &mut hires)?;
        sections.push((format!("{id} hires"), hires));
        let mut lowres = Section::default();
        tiles::lowres(golden, candidate, id, &mut lowres)?;
        sections.push((format!("{id} lowres"), lowres));
        let mut rs = Section::default();
        rstate::compare(&gm, &cm, &mut rs)?;
        sections.push((format!("{id} rstate"), rs));
    }
    Ok(Comparison { sections })
}

/// `rel` (with any compression suffix) decompressed in both webroots.
fn same_bytes(section: &mut Section, golden: &Path, candidate: &Path, rel: &str) -> Result<()> {
    let read = |root: &Path| read_decompressed(&root.join(rel)).ok();
    let (g, c) = (read(golden), read(candidate));
    let line = match (&g, &c) {
        (None, None) => return Ok(()),
        (Some(g), Some(c)) if g == c => format!("{rel}: identical ({} bytes)", g.len()),
        (Some(_), Some(_)) => format!("{rel}: content differs"),
        (Some(_), None) => format!("{rel}: missing"),
        (None, Some(_)) => format!("{rel}: extra"),
    };
    section.check(g == c, line);
    Ok(())
}

/// Root `settings.json` and everything under `maps/`, as `/`-separated paths.
fn data_files(webroot: &Path) -> Result<BTreeSet<String>> {
    let mut files: BTreeSet<String> = walk(&webroot.join("maps"))?.into_iter().map(|f| format!("maps/{f}")).collect();
    if webroot.join("settings.json").is_file() {
        files.insert("settings.json".to_owned());
    }
    Ok(files)
}

/// Files under `dir`, relative and `/`-separated; empty when `dir` doesn't exist.
pub(crate) fn walk(dir: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).context(d.display().to_string()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path.strip_prefix(dir)?.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(out)
}

fn list_dirs(dir: &Path) -> Result<BTreeSet<String>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(BTreeSet::new()) };
    let mut out = BTreeSet::new();
    for entry in entries {
        let entry = entry?;
        if entry.path().is_dir() {
            out.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(out)
}
