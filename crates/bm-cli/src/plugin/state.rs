//! `<data>/pluginState.json` (`PluginState.java`, written by Configurate's Gson loader with its default
//! lower-case-dashed field names and indent 0).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct MapState {
    pub update_enabled: bool,
    /// Epoch seconds.
    pub last_full_update: i64,
}

impl Default for MapState {
    fn default() -> Self {
        Self { update_enabled: true, last_full_update: 0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginState {
    pub render_threads_enabled: bool,
    pub maps: BTreeMap<String, MapState>,
    pub hidden_players: Vec<String>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self { render_threads_enabled: true, maps: BTreeMap::new(), hidden_players: Vec::new() }
    }
}

pub fn file(data: &Path) -> PathBuf {
    data.join("pluginState.json")
}

/// Missing file → defaults; unreadable → defaults plus the upstream warning.
pub fn load(data: &Path) -> (PluginState, Option<&'static str>) {
    let Ok(text) = std::fs::read_to_string(file(data)) else { return (PluginState::default(), None) };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(root)) => (from_json(&root), None),
        _ => (PluginState::default(), Some("Failed to load pluginState.json (invalid format), creating a new one...")),
    }
}

fn field<'a>(o: &'a Map<String, Value>, dashed: &str, camel: &str) -> Option<&'a Value> {
    o.get(dashed).or_else(|| o.get(camel))
}

fn from_json(root: &Map<String, Value>) -> PluginState {
    let mut state = PluginState::default();
    if let Some(b) = field(root, "render-threads-enabled", "renderThreadsEnabled").and_then(Value::as_bool) {
        state.render_threads_enabled = b;
    }
    if let Some(maps) = field(root, "maps", "maps").and_then(Value::as_object) {
        for (id, m) in maps {
            let Some(m) = m.as_object() else { continue };
            let mut ms = MapState::default();
            if let Some(b) = field(m, "update-enabled", "updateEnabled").and_then(Value::as_bool) {
                ms.update_enabled = b;
            }
            if let Some(t) = field(m, "last-full-update", "lastFullUpdate").and_then(Value::as_i64) {
                ms.last_full_update = t;
            }
            state.maps.insert(id.clone(), ms);
        }
    }
    if let Some(list) = field(root, "hidden-players", "hiddenPlayers").and_then(Value::as_array) {
        state.hidden_players = list.iter().filter_map(Value::as_str).map(str::to_owned).collect();
    }
    state
}

impl PluginState {
    pub fn map(&mut self, id: &str) -> &mut MapState {
        self.maps.entry(id.to_owned()).or_default()
    }

    pub fn is_frozen(&self, id: &str) -> bool {
        self.maps.get(id).is_some_and(|m| !m.update_enabled)
    }

    pub fn is_hidden(&self, uuid: &str) -> bool {
        self.hidden_players.iter().any(|p| p.eq_ignore_ascii_case(uuid))
    }

    pub fn set_hidden(&mut self, uuid: &str, hidden: bool) {
        self.hidden_players.retain(|p| !p.eq_ignore_ascii_case(uuid));
        if hidden {
            self.hidden_players.push(uuid.to_ascii_lowercase());
        }
    }

    /// Field order as declared in `PluginState`; maps in `ConcurrentHashMap` order.
    pub fn to_json(&self) -> String {
        let quote = |s: &str| Value::from(s).to_string();
        let ids: Vec<&str> = self.maps.keys().map(String::as_str).collect();
        let maps: Vec<String> = bm_java::concurrent_hash_map_order(&ids)
            .into_iter()
            .map(|id| {
                let m = &self.maps[id];
                format!(
                    "{}:{{\"update-enabled\":{},\"last-full-update\":{}}}",
                    quote(id),
                    m.update_enabled,
                    m.last_full_update
                )
            })
            .collect();
        let hidden: Vec<String> = self.hidden_players.iter().map(|p| quote(p)).collect();
        format!(
            "{{\"render-threads-enabled\":{},\"maps\":{{{}}},\"hidden-players\":[{}]}}",
            self.render_threads_enabled,
            maps.join(","),
            hidden.join(",")
        )
    }

    pub fn save(&self, data: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data)?;
        let path = file(data);
        let tmp = path.with_extension("json.filepart");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_camel_case_input() {
        let mut s = PluginState::default();
        s.map("world").update_enabled = false;
        s.map("world").last_full_update = 1_700_000_000;
        s.set_hidden("ABC", true);
        let json = s.to_json();
        assert!(json.starts_with("{\"render-threads-enabled\":true,\"maps\":{\"world\":"), "{json}");
        let Value::Object(root) = serde_json::from_str(&json).unwrap() else { panic!() };
        assert_eq!(from_json(&root), s);
        let Value::Object(camel) =
            serde_json::from_str(r#"{"renderThreadsEnabled":false,"maps":{"a":{"updateEnabled":false}}}"#).unwrap()
        else {
            panic!()
        };
        let parsed = from_json(&camel);
        assert!(!parsed.render_threads_enabled && parsed.is_frozen("a") && !parsed.is_frozen("b"));
    }
}
