//! Live data pushed by the shim: the player batch (filtered into each map's `players.json` here, like
//! `PluginLivePlayerInfoTransformer` + `LivePlayersDataSupplier`) and each map's `MarkerGson` JSON.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use bm_ipc::{MarkerAssembler, PlayerInfo};
use bm_map::players::{LivePlayer, players_json};

use super::session::Session;
use crate::log;

/// A map whose `live/markers.json` was served this recently keeps its markers demanded.
const READ_WINDOW: Duration = Duration::from_secs(30);
/// Markers are demanded this long after a load and before a storage write, so writes use fresh JSON.
const WARMUP: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct LiveData {
    players: Mutex<Vec<PlayerInfo>>,
    markers: Mutex<HashMap<String, String>>,
    assembler: Mutex<MarkerAssembler>,
}

impl LiveData {
    pub fn online_count(&self) -> usize {
        self.players.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn set_players(&self, batch: Vec<PlayerInfo>, session: Option<&Session>) {
        *self.players.lock().unwrap_or_else(PoisonError::into_inner) = batch;
        if let Some(s) = session {
            self.publish_players(s);
        }
    }

    /// Recomputes every map's `players.json` from the last batch.
    pub fn publish_players(&self, s: &Session) {
        for (id, live) in &s.lives {
            live.set_players(self.players_json(s, id));
        }
    }

    pub fn players_json(&self, s: &Session, map: &str) -> String {
        let players = self.players.lock().unwrap_or_else(PoisonError::into_inner);
        let config = &s.service.config.plugin;
        let state = s.state.lock().unwrap_or_else(PoisonError::into_inner);
        let map_world = s.map_server_world.get(map).cloned().flatten();
        let visible = players.iter().filter_map(|p| {
            let correct_world = map_world.as_deref() == Some(p.world.as_str());
            let hidden = (config.hide_different_world && !correct_world)
                || state.is_hidden(&p.uuid)
                || (config.hide_invisible && p.invisible)
                || (config.hide_vanished && p.vanished)
                || (config.hide_sneaking && p.sneaking)
                || config.hidden_game_modes.iter().any(|g| g == &p.gamemode)
                || (p.sky_light < config.hide_below_sky_light && p.block_light < config.hide_below_block_light);
            (!hidden).then(|| LivePlayer {
                uuid: &p.uuid,
                name: &p.name,
                foreign: !correct_world,
                position: [p.x, p.y, p.z],
                rotation: [p.pitch, p.yaw, 0.0],
            })
        });
        players_json(visible)
    }

    /// One `Markers` frame; complete JSON is validated (#202: one bad marker kills the webapp) and served.
    pub fn on_markers(&self, map: &str, more: bool, body: Vec<u8>, session: Option<&Session>) {
        let Some(json) = self.assembler.lock().unwrap_or_else(PoisonError::into_inner).push(map, more, body) else {
            return;
        };
        let json = match String::from_utf8(json) {
            Ok(j) if serde_json::from_str::<serde::de::IgnoredAny>(&j).is_ok() => j,
            _ => {
                log::warn(&format!("Ignoring invalid marker JSON for map '{map}'"));
                return;
            }
        };
        if let Some(live) = session.and_then(|s| s.lives.get(map)) {
            live.set_markers(json.clone());
        }
        self.markers.lock().unwrap_or_else(PoisonError::into_inner).insert(map.to_owned(), json);
    }

    /// The shim's last JSON for `map`, else its config marker sets.
    pub fn markers_json(&self, s: &Session, map: &str) -> Option<String> {
        if let Some(json) = self.markers.lock().unwrap_or_else(PoisonError::into_inner).get(map) {
            return Some(json.clone());
        }
        s.maps.get(map).map(|m| m.markers_json.clone())
    }

    /// Forgets pushed markers (a reload gives the API fresh, empty sets).
    pub fn clear_markers(&self) {
        self.markers.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    /// `BmMap.saveMarkerState` for every map.
    pub fn write_markers(&self, s: &Session) {
        for map in s.maps.all() {
            let Some(json) = self.markers_json(s, &map.id) else { continue };
            if let Err(e) = map.storage.write_item(&bm_storage::ItemKey::Markers, json.as_bytes()) {
                log::error(&format!("Failed to save markers for map '{}'! {e}", map.id));
            }
        }
    }

    /// `Plugin.savePlayerStates`.
    pub fn write_players(&self, s: &Session) {
        for map in s.maps.all() {
            let json = self.players_json(s, &map.id);
            if let Err(e) = map.storage.write_item(&bm_storage::ItemKey::Players, json.as_bytes()) {
                log::error(&format!("Failed to save players for map '{}'! {e}", map.id));
            }
        }
    }
}

/// Maps whose markers the core needs now: viewers (SSE or recent `markers.json` reads), a fresh load, or a storage
/// write coming up within [`WARMUP`].
pub fn marker_demand(s: &Session, write_due_in: Option<Duration>) -> Vec<String> {
    let warm = s.loaded_at.elapsed() < WARMUP || write_due_in.is_some_and(|d| d < WARMUP);
    let mut maps: Vec<String> = s
        .lives
        .iter()
        .filter(|(_, live)| warm || live.sse_clients() > 0 || live.markers_read_within(READ_WINDOW))
        .map(|(id, _)| id.clone())
        .collect();
    maps.sort();
    maps
}

/// When the next marker write happens, for [`marker_demand`].
pub fn next_due(last: Instant, every: Duration) -> Option<Duration> {
    (!every.is_zero()).then(|| (last + every).saturating_duration_since(Instant::now()))
}
