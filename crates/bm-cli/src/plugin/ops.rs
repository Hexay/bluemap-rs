//! Map and render-thread operations shared by commands and API RPCs (`Plugin`, `RenderManagerImpl`,
//! `BlueMapMapImpl.setFrozen`, `MapPurgeTask`).

use std::sync::PoisonError;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bm_engine::{RenderTask, TileUpdateStrategy};

use super::core::Core;
use super::session::Session;
use crate::{log, web};

/// Freezing stops the map's watcher and drops its tasks; unfreezing restarts the watcher and schedules an update.
/// Returns false if nothing changed.
pub fn set_frozen(core: &Core, s: &Session, map: &str, frozen: bool) -> bool {
    {
        let mut state = s.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.is_frozen(map) == frozen {
            return false;
        }
        state.map(map).update_enabled = !frozen;
    }
    if frozen {
        s.stop_watching(map);
        s.queue.remove_where(|t| t.map == map);
    } else {
        s.start_watching(map);
        s.queue.schedule(RenderTask::full(map, TileUpdateStrategy::ForceNone));
    }
    s.save_state();
    core.state_changed(s);
    true
}

/// `/bluemap start|stop`, `RenderManager.start/stop`: persisted in plugin state.
pub fn set_render_threads(core: &Core, s: &Session, running: bool) {
    s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled = running;
    if running {
        s.queue.resume();
    } else {
        s.queue.pause();
    }
    core.state_changed(s);
}

/// `Plugin.checkPausedByPlayerCount`: pauses at `player-render-limit` players, resumes below it.
pub fn check_render_limit(core: &Core, s: &Session) {
    let limit = s.service.config.plugin.player_render_limit;
    let enabled = s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled;
    let paused = s.queue.is_paused();
    if limit > 0 && core.live.online_count() >= limit as usize {
        if !paused {
            s.queue.pause();
            core.state_changed(s);
        }
    } else if paused && enabled {
        s.queue.resume();
        core.state_changed(s);
    }
}

/// `MapPurgeTask`: deletes all of the map's data, then loads it fresh and (unless frozen) renders it again.
pub fn purge(s: &Session, map: &str) -> Result<()> {
    let ctx = s.maps.get(map).context("map is not loaded")?;
    s.stop_watching(map);
    s.queue.remove_where(|t| t.map == map);
    let deadline = Instant::now() + Duration::from_secs(60);
    while s.queue.current_task().is_some_and(|t| t.map == map) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    ctx.storage.delete(&mut |_| true).with_context(|| format!("delete map '{map}'"))?;
    drop(ctx);
    let mut fresh = s.service.open_map(map)?.context("map has no world")?;
    if let Some(live) = s.lives.get(map) {
        web::hook_tiles(&mut fresh, live, s.service.config.webserver.sse_enabled);
    }
    s.maps.insert(fresh);
    if !s.state.lock().unwrap_or_else(PoisonError::into_inner).is_frozen(map) {
        s.start_watching(map);
        s.queue.schedule(RenderTask::full(map, TileUpdateStrategy::ForceNone));
    }
    log::info(&format!("Purged map '{map}'"));
    Ok(())
}

pub fn strategy(force: bool) -> TileUpdateStrategy {
    if force { TileUpdateStrategy::ForceAll } else { TileUpdateStrategy::ForceNone }
}
