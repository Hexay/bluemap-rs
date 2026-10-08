//! `startWatchingMap`/`stopWatchingMap`: one `MapUpdateService` per unfrozen map, its periodic full updates
//! anchored at and recorded into the map's `lastFullUpdate` (persisted with the next `pluginState.json` save).

use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bm_engine::{FullUpdates, MapUpdateService, WatchSettings};

use super::session::Session;
use crate::log;

impl Session {
    pub(super) fn start_watchers(&self) {
        let ids: Vec<String> = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            self.maps.all().iter().filter(|m| !state.is_frozen(&m.id)).map(|m| m.id.clone()).collect()
        };
        ids.iter().for_each(|id| self.start_watching(id));
    }

    pub fn start_watching(&self, id: &str) {
        let Some(map) = self.maps.get(id) else { return };
        let settings = WatchSettings::from_core(&self.service.config.core);
        self.stop_watching(id);
        let dir = map.world.region_dir().to_owned();
        let full = self.full_updates(id);
        match MapUpdateService::start_with(id, dir, self.queue.clone(), settings, crate::watch::engine_log(), full) {
            Ok(w) => {
                self.watchers.lock().unwrap_or_else(PoisonError::into_inner).insert(id.to_owned(), w);
            }
            Err(e) => log::error(&format!(
                "Failed to create update-watcher for map: {id} (This means the map might not automatically update): {e}"
            )),
        }
    }

    pub fn stop_watching(&self, id: &str) {
        let removed = self.watchers.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
        if let Some(w) = removed {
            w.close();
        }
    }

    fn full_updates(&self, id: &str) -> FullUpdates {
        let last_secs = self.state.lock().unwrap_or_else(PoisonError::into_inner).map(id).last_full_update;
        let last = UNIX_EPOCH + Duration::from_secs(last_secs.max(0) as u64);
        let (state, id) = (self.state.clone(), id.to_owned());
        let on_scheduled = Arc::new(move |at: SystemTime| {
            let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            state.lock().unwrap_or_else(PoisonError::into_inner).map(&id).last_full_update = secs;
        });
        FullUpdates { last, on_scheduled }
    }
}
