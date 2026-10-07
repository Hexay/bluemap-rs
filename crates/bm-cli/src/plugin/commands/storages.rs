//! `StoragesCommand`: list storages, show one, delete an unloaded map from one.

use anyhow::Result;
use bm_config::StorageConfig;
use serde_json::Value;

use super::super::session::Session;
use super::super::text::{self, BASE, HIGHLIGHT, NEGATIVE, POSITIVE, hl};
use super::Say;
use crate::log;

/// Upstream lists a storage as loaded once something opened it; our storages open with the first map using them.
fn is_loaded(s: &Session, storage: &str) -> bool {
    s.maps.all().iter().any(|m| m.config.storage == storage)
}

fn icon(loaded: bool) -> Value {
    if loaded { text::span("✔", POSITIVE) } else { text::span("❌", BASE) }
}

fn entry(loaded: bool, id: &str) -> Vec<Value> {
    vec![icon(loaded), text::span(" ", BASE), text::span(id, HIGHLIGHT)]
}

pub fn list(s: &Session, say: Say) -> i32 {
    let body = s.service.config.storages.keys().map(|id| entry(is_loaded(s, id), id)).collect();
    say(text::paragraph("Storages", body));
    1
}

/// `isMapLoaded(mapId, storageId)`.
fn map_loaded(s: &Session, map: &str, storage: &str) -> bool {
    s.maps.get(map).is_some_and(|m| m.config.storage == storage)
}

fn key(name: impl std::fmt::Debug) -> String {
    format!("bluemap:{}", format!("{name:?}").to_lowercase())
}

pub fn show(s: &Session, id: &str, say: Say) -> i32 {
    match details(s, id, say) {
        Ok(body) => {
            say(text::paragraph(&format!("Storage '{id}'"), body));
            1
        }
        Err(e) => {
            log::error(&format!("Failed to load storage '{id}': {e:#}"));
            say(text::one("There was an error trying to load this storage, see console for details.", NEGATIVE));
            0
        }
    }
}

fn details(s: &Session, id: &str, say: Say) -> Result<Vec<Vec<Value>>> {
    let config = s.service.config.storages.get(id).ok_or_else(|| anyhow::anyhow!("no storage '{id}'"))?;
    if !is_loaded(s, id) {
        say(text::lines(text::fill("Initializing storage '%'...", &[(id, "")], BASE)));
    }
    let storage = s.service.storage(id)?;
    let mut lines = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_default();
    match config {
        StorageConfig::File(c) => {
            lines.push(text::format("Type: %", &["bluemap:file"]));
            lines.push(text::format("Path: %", &[&bm_config::generate::format_path(&c.root, &cwd)]));
            lines.push(text::format("Compression: %", &[&c.compression().map(key).unwrap_or_default()]));
        }
        StorageConfig::Sql(c) => {
            lines.push(text::format("Type: %", &["bluemap:sql"]));
            let dialect = c.dialect().map(|d| format!("bluemap:{}", d.key())).unwrap_or_default();
            lines.push(text::format("Dialect: %", &[&dialect]));
            lines.push(text::format("Compression: %", &[&c.compression().map(key).unwrap_or_default()]));
        }
    }
    lines.push(Vec::new());
    lines.push(vec![text::span("Maps:", BASE)]);
    lines.extend(storage.map_ids()?.iter().take(20).map(|m| entry(map_loaded(s, m, id), m)));
    Ok(lines)
}

/// Upstream schedules a `StorageDeleteTask`; this deletes on the command's thread.
pub fn delete(s: &Session, id: &str, map: &str, say: Say) -> i32 {
    if map_loaded(s, map, id) {
        let purge = format!("/bluemap purge {map}");
        let mut line = vec![text::span("Can't delete a loaded map!", NEGATIVE)];
        let mut rest = text::fill(
            "Unload the map by removing it's config-file first,\nor use % if you want to purge it.",
            &[hl(&purge)],
            BASE,
        );
        line.append(&mut rest[0]);
        rest[0] = line;
        say(text::lines(rest));
        return 0;
    }
    let storage = match s.service.storage(id) {
        Ok(storage) => storage,
        Err(e) => {
            log::error(&format!("Failed to load storage '{id}': {e:#}"));
            say(text::one("There was an error trying to load this storage, see console for details.", NEGATIVE));
            return 0;
        }
    };
    let mut scheduled = text::fill("Scheduled a new task to delete map % from storage %", &[hl(map), hl(id)], POSITIVE);
    scheduled.extend(text::fill("Use % to see the progress", &[hl("/bluemap")], BASE));
    say(text::lines(scheduled));
    match storage.map(map).and_then(|m| m.delete(&mut |_| true)) {
        Ok(()) => log::info(&format!("Deleted map '{map}' from storage '{id}'")),
        Err(e) => log::error(&format!("Failed to delete map '{map}' from storage '{id}': {e}")),
    }
    1
}
