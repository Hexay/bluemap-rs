//! `live/markers.json` from a map config's `marker-sets`, byte-identical to BlueMap 5.28: `MapConfig.parseMarkerSets`
//! (Configurate JSON → MarkerGson) then `MarkerGson.INSTANCE.toJson`.
//!
//! Deliberate deviation (docs/08-github-issues.md #202): Java rejects the whole `marker-sets` on any bad marker
//! (and fails to load the map on render); here a bad marker or set is skipped with a warning.

mod json;
mod model;
mod read;
mod write;

use std::collections::HashMap;

use bm_java::concurrent_hash_map_order;

use crate::gson::{JsonObject, quote};
use json::Json;
use model::Marker;

#[derive(Debug, thiserror::Error)]
pub enum MarkerError {
    #[error("marker-sets is not valid JSON: {0}")]
    Json(String),
    #[error("marker-sets must be an object, got {0}")]
    NotAnObject(&'static str),
}

/// Key order of the top-level marker-set map, which differs between Java's two writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOrder {
    /// `BlueMapCLI.updateMarkers` (`--markers`): the parsed `LinkedTreeMap`, i.e. config order.
    Config,
    /// `BmMap.saveMarkerState` (map load/render) and the webserver's `LiveMarkersDataSupplier`: the map's
    /// `ConcurrentHashMap`.
    Map,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkersJson {
    pub json: String,
    /// One line per skipped marker set or marker.
    pub warnings: Vec<String>,
}

/// `config_json` is `bm_config::MapConfig::marker_sets_json()` (Configurate's JSON text of `marker-sets`).
pub fn marker_sets_json(config_json: &str, order: SetOrder) -> Result<MarkersJson, MarkerError> {
    let root = json::parse(config_json).map_err(MarkerError::Json)?;
    let Json::Obj(sets) = root else { return Err(MarkerError::NotAnObject(root.kind())) };
    let mut warnings = Vec::new();
    let mut parsed: Vec<(&str, String)> = Vec::new();
    for (id, set) in &sets {
        if parsed.iter().any(|(seen, _)| seen == id) {
            warnings.push(format!("marker-set '{id}': duplicate id, skipped"));
            continue;
        }
        match marker_set_json(id, set, &mut warnings) {
            Ok(json) => parsed.push((id, json)),
            Err(e) => warnings.push(format!("marker-set '{id}' skipped: {e}")),
        }
    }
    let config_order: Vec<&str> = parsed.iter().map(|(id, _)| *id).collect();
    let ids = match order {
        SetOrder::Config => config_order,
        SetOrder::Map => concurrent_hash_map_order(&config_order),
    };
    let json = ordered_object(&ids, &parsed);
    Ok(MarkersJson { json, warnings })
}

/// `MarkerSet` through the streaming reader; its `markers` field is a `ConcurrentHashMap`.
fn marker_set_json(id: &str, set: &Json, warnings: &mut Vec<String>) -> Result<String, String> {
    let Json::Obj(members) = set else { return Err(format!("expected an object, got {}", set.kind())) };
    let (mut label, mut toggleable, mut default_hidden, mut sorting) = (String::new(), true, false, 0);
    let mut markers: Vec<(&str, String)> = Vec::new();
    for (key, value) in members {
        let field = |e: String| format!("{key}: {e}");
        match key.as_str() {
            "label" => label = read::string(value).map_err(field)?,
            "toggleable" => toggleable = read::boolean(value).map_err(field)?,
            "default-hidden" => default_hidden = read::boolean(value).map_err(field)?,
            "sorting" => sorting = read::stream_int(value).map_err(field)?,
            "markers" => {
                let Json::Obj(entries) = value else {
                    return Err(field(format!("expected an object, got {}", value.kind())));
                };
                markers.clear();
                for (marker_id, marker) in entries {
                    match Marker::read(marker) {
                        Ok(m) => {
                            markers.retain(|(k, _)| k != marker_id);
                            markers.push((marker_id, m.to_json()));
                        }
                        Err(e) => warnings.push(format!("marker '{marker_id}' in marker-set '{id}' skipped: {e}")),
                    }
                }
            }
            _ => {}
        }
    }
    let markers_json =
        ordered_object(&concurrent_hash_map_order(&markers.iter().map(|(k, _)| *k).collect::<Vec<_>>()), &markers);
    Ok(JsonObject::new()
        .string("label", &label)
        .bool("toggleable", toggleable)
        .bool("defaultHidden", default_hidden)
        .int("sorting", sorting)
        .raw("markers", &markers_json)
        .finish())
}

/// `{"key":json,...}` with the members of `entries` in `order`.
fn ordered_object(order: &[&str], entries: &[(&str, String)]) -> String {
    let by_key: HashMap<&str, &str> = entries.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let members: Vec<String> = order.iter().map(|k| format!("{}:{}", quote(k), by_key[k])).collect();
    format!("{{{}}}", members.join(","))
}
