//! `BlueMap-Plugin-Timer`: periodic save (10 min), `write-markers-interval`, `write-players-interval`, the
//! debounced settings.json rewrite, delayed `player-render-limit` checks, marker demand and the `memory-limit`
//! guard, on one 1 Hz thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use bm_ipc::CoreMsg;

use super::core::Core;
use super::live::{marker_demand, next_due};
use super::ops;
use crate::throttle::memory::MemoryGuard;

const SAVE_EVERY: Duration = Duration::from_secs(600);
const SETTINGS_DEBOUNCE: Duration = Duration::from_secs(1);

fn secs(n: i32) -> Duration {
    Duration::from_secs(n.max(0) as u64)
}

/// Runs until `stop` is set.
pub fn spawn(core: Arc<Core>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("bluemap-plugin-timer".into())
        .spawn(move || {
            let mut last_save = Instant::now();
            let (mut last_markers, mut last_players) = (Instant::now(), Instant::now());
            let mut demand: Option<Vec<String>> = None;
            let mut session_seen = std::ptr::null();
            let mut memory: Option<MemoryGuard> = None;
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
                let Some(s) = core.session() else { continue };
                // a new session means a new API instance in the shim: announce demand again
                if !std::ptr::eq(Arc::as_ptr(&s), session_seen) {
                    session_seen = Arc::as_ptr(&s);
                    demand = None;
                    memory = s.service.config.core.memory_limit.map(MemoryGuard::new);
                }
                if let (Some(guard), Some(rss)) = (&mut memory, bm_ipc::resident_memory()) {
                    let was_paused = s.queue.is_paused();
                    guard.check(rss, &s.queue, Instant::now());
                    if s.queue.is_paused() != was_paused {
                        core.state_changed(&s);
                    }
                }
                let config = &s.service.config.plugin;
                let (markers_every, players_every) =
                    (secs(config.write_markers_interval), secs(config.write_players_interval));

                let due = core
                    .settings_due
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .filter(|t| t.elapsed() >= SETTINGS_DEBOUNCE);
                if due.is_some() {
                    *core.settings_due.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    let files = core.web_files.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    s.write_webapp((&files.0, &files.1));
                }
                let check =
                    core.limit_check_at.lock().unwrap_or_else(PoisonError::into_inner).filter(|t| *t <= Instant::now());
                if check.is_some() {
                    *core.limit_check_at.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    ops::check_render_limit(&core, &s);
                }
                if !s.is_loaded() {
                    continue;
                }

                let write_due = [next_due(last_save, SAVE_EVERY), next_due(last_markers, markers_every)]
                    .into_iter()
                    .flatten()
                    .min();
                let wanted = marker_demand(&s, write_due);
                if demand.as_ref() != Some(&wanted) {
                    core.out.send(CoreMsg::MarkerDemand { maps: wanted.clone() });
                    demand = Some(wanted);
                }
                if last_save.elapsed() >= SAVE_EVERY {
                    last_save = Instant::now();
                    core.save();
                }
                if !markers_every.is_zero() && last_markers.elapsed() >= markers_every {
                    last_markers = Instant::now();
                    core.live.write_markers(&s);
                }
                if !players_every.is_zero() && last_players.elapsed() >= players_every {
                    last_players = Instant::now();
                    core.live.write_players(&s);
                }
            }
        })
        .expect("spawn plugin timer")
}
