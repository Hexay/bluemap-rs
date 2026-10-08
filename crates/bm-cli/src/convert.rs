//! `--convert-storage <id> --to <format>`: in-place storage conversion, then the storage config's `format:` key
//! is updated so the storage opens again (a mismatch is refused, see `bm_storage::Format`).

use std::path::Path;

use anyhow::{Context, Result, bail};
use bm_engine::Service;
use bm_storage::Format;

use crate::log;

pub fn convert_storage(service: &Service, config_folder: &Path, id: &str, to: &str) -> Result<()> {
    let Some(to) = Format::from_id(to) else { bail!("unknown storage format '{to}' (expected compat or optimized)") };
    log::info(&format!("Converting storage '{id}' to {to} ..."));
    let progress = |map: &str, done: usize, total: usize| log::info(&format!("  {map}: {done}/{total} hires tiles"));
    let stats = service.convert_storage(id, to, &progress)?;
    if stats.already {
        log::info(&format!("Storage '{id}' already was {to}."));
    } else {
        log::info(&format!("Converted {} hires tiles of {} maps.", stats.tiles, stats.maps));
    }
    let file = bm_config::resolve_config_file(&config_folder.join("storages"), id);
    if file.extension().is_some_and(|e| e == "conf") {
        let text = std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        std::fs::write(&file, with_format(&text, to)).with_context(|| format!("write {}", file.display()))?;
        log::info(&format!("Set format: {to} in {}", file.display()));
    } else {
        log::warn(&format!("Set format: {to} in {} yourself, or the storage will not open.", file.display()));
    }
    Ok(())
}

/// `text` with its top-level `format` setting replaced, or appended when missing.
fn with_format(text: &str, to: Format) -> String {
    let is_format = |line: &str| {
        let rest = line.strip_prefix("format").map(str::trim_start);
        !line.starts_with([' ', '\t']) && rest.is_some_and(|r| r.starts_with([':', '=']))
    };
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            if is_format(line) {
                found = true;
                format!("format: {to}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    if !found {
        out.push(format!(
            "# bluemap-rs storage layout (compat or optimized), changed by --convert-storage\nformat: {to}"
        ));
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_key_is_replaced_or_appended() {
        assert_eq!(with_format("root: \"x\"\nformat: compat\n", Format::Optimized), "root: \"x\"\nformat: optimized\n");
        assert_eq!(with_format("format=optimized", Format::Compat), "format: compat\n");
        let appended = with_format("root: x\n  format: nested\n", Format::Optimized);
        assert!(appended.starts_with("root: x\n  format: nested\n#") && appended.ends_with("format: optimized\n"));
    }
}
