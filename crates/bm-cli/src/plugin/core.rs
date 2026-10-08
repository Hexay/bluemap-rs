//! The plugin core's shared state: the shim connection, the current [`Session`] and live data, plus the
//! load/unload/reload cycle and the `Ready`/`StateChanged` messages describing it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use bm_engine::PauseReason;
use bm_ipc::{CoreMsg, MapInfo, PluginInfo, ReadyInfo, StateInfo, WorldInfo};

use super::Hello;
use super::live::LiveData;
use super::outbox::Outbox;
use super::session::{NotReady, Session, world_id};
use super::tasks_dat;
use crate::log;
use crate::throttle::load::LoadMonitor;

pub const CORE_VERSION: &str = crate::VERSION;

pub struct Core {
    pub out: Outbox,
    pub hello: Hello,
    pub worlds: Mutex<Vec<WorldInfo>>,
    pub live: LiveData,
    session: RwLock<Option<Arc<Session>>>,
    /// Held while loading/unloading; requests wait on it, so they see a settled session.
    cycle: Mutex<()>,
    loading: AtomicBool,
    /// Scripts and styles registered through the API (kept across reloads, like `WebFilesManager`).
    pub web_files: Mutex<(Vec<String>, Vec<String>)>,
    /// First unsaved script/style registration (1 s debounce, `WebAppImpl.scheduleUpdateWebAppSettings`).
    pub settings_due: Mutex<Option<Instant>>,
    /// A join/leave asks for a `player-render-limit` check at this time.
    pub limit_check_at: Mutex<Option<Instant>>,
    /// The server's tick time; kept across reloads.
    pub load: Mutex<LoadMonitor>,
}

impl Core {
    pub fn new(out: Outbox, hello: Hello, worlds: Vec<WorldInfo>) -> Self {
        Self {
            out,
            hello,
            worlds: Mutex::new(worlds),
            live: LiveData::default(),
            session: RwLock::default(),
            cycle: Mutex::default(),
            loading: AtomicBool::new(false),
            web_files: Mutex::default(),
            settings_due: Mutex::default(),
            limit_check_at: Mutex::default(),
            load: Mutex::default(),
        }
    }

    /// The current session (also a webserver-only one after `no-maps`), without waiting for a running load.
    pub fn session(&self) -> Option<Arc<Session>> {
        self.session.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Waits for a running load/unload, then returns the session if it has maps (`Plugin.isLoaded`).
    pub fn loaded(&self) -> Option<Arc<Session>> {
        let _cycle = self.cycle.lock().unwrap_or_else(PoisonError::into_inner);
        self.session().filter(|s| s.is_loaded())
    }

    pub fn is_loading(&self) -> bool {
        self.loading.load(Ordering::SeqCst)
    }

    /// `Plugin.load()`; sends `Ready` or `NotReady`. Returns whether maps are loaded.
    pub fn load(&self) -> bool {
        let _cycle = self.cycle.lock().unwrap_or_else(PoisonError::into_inner);
        self.load_locked()
    }

    fn load_locked(&self) -> bool {
        self.loading.store(true, Ordering::SeqCst);
        log::info("Loading...");
        let worlds = self.worlds.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let files = self.web_files.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let result = Session::load(&self.hello, &worlds, (&files.0, &files.1));
        let loaded = match result {
            Ok(session) => {
                let session = Arc::new(session);
                if self.load.lock().unwrap_or_else(PoisonError::into_inner).is_lagging() {
                    session.queue.pause(PauseReason::ServerLoad);
                }
                self.live.publish_players(&session);
                *self.session.write().unwrap_or_else(PoisonError::into_inner) = Some(session.clone());
                tasks_dat::resume(&session);
                self.out.send(CoreMsg::Ready(Box::new(self.ready_info(&session))));
                log::info("Loaded!");
                true
            }
            Err(NotReady { reason, message, kept }) => {
                *self.session.write().unwrap_or_else(PoisonError::into_inner) = kept.map(|s| Arc::new(*s));
                self.out.send(CoreMsg::NotReady { reason: reason.to_owned(), message });
                false
            }
        };
        self.loading.store(false, Ordering::SeqCst);
        loaded
    }

    /// `Plugin.unload()`: saves markers and state, then stops everything. `reloading` tells the shim first.
    pub fn unload(&self, reloading: bool) {
        let _cycle = self.cycle.lock().unwrap_or_else(PoisonError::into_inner);
        self.unload_locked(reloading);
    }

    fn unload_locked(&self, reloading: bool) {
        let Some(session) = self.session.write().unwrap_or_else(PoisonError::into_inner).take() else { return };
        if reloading {
            self.out.send(CoreMsg::Unloading);
        }
        if session.is_loaded() {
            self.live.write_markers(&session);
            tasks_dat::save(&session);
        }
        session.unload();
        if reloading {
            self.live.clear_markers();
        }
    }

    /// `Plugin.reload()`; `light` is accepted but resources are reloaded too.
    pub fn reload(&self, _light: bool) -> bool {
        let _cycle = self.cycle.lock().unwrap_or_else(PoisonError::into_inner);
        self.unload_locked(true);
        self.load_locked()
    }

    /// `Plugin.save()`: plugin state and every map's markers.
    pub fn save(&self) {
        if let Some(s) = self.session().filter(|s| s.is_loaded()) {
            s.save_state();
            tasks_dat::save(&s);
            self.live.write_markers(&s);
        }
    }

    pub fn state_info(&self, s: &Session) -> StateInfo {
        let state = s.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut frozen: Vec<String> =
            s.maps.all().iter().filter(|m| state.is_frozen(&m.id)).map(|m| m.id.clone()).collect();
        frozen.sort();
        StateInfo {
            frozen_maps: frozen,
            hidden_players: state.hidden_players.clone(),
            render_threads_running: !s.queue.is_paused(),
        }
    }

    pub fn state_changed(&self, s: &Session) {
        self.out.send(CoreMsg::StateChanged(self.state_info(s)));
    }

    fn ready_info(&self, s: &Session) -> ReadyInfo {
        let state = self.state_info(s);
        let mut maps: Vec<_> = s.maps.all();
        maps.sort_by_key(|m| m.config.sorting);
        let config = &s.service.config;
        ReadyInfo {
            core_version: CORE_VERSION.to_owned(),
            compat_version: bm_engine::BLUEMAP_VERSION.to_owned(),
            webroot: absolute(&config.webapp.webroot),
            worlds: s.worlds.clone(),
            maps: maps
                .iter()
                .map(|m| MapInfo {
                    id: m.id.clone(),
                    name: m.config.name.clone().unwrap_or_else(|| m.id.clone()),
                    world: world_id(m),
                    tile_size: m.hires_grid.size,
                    tile_offset: m.hires_grid.offset,
                    frozen: state.frozen_maps.contains(&m.id),
                    config_markers: m.markers_json.clone(),
                })
                .collect(),
            storages: config.storages.keys().cloned().collect(),
            plugin: PluginInfo {
                live_player_markers: config.plugin.live_player_markers,
                skin_download: config.plugin.skin_download,
                player_render_limit: config.plugin.player_render_limit,
                metrics: config.core.metrics,
            },
            state,
        }
    }

    /// Asks the shim to save `world` (`None` = all) before an update (`persistWorldChanges`).
    pub fn save_world(&self, world: Option<String>) -> bool {
        let reply = self.out.request(|id| CoreMsg::SaveWorld { id, world }, Duration::from_secs(60));
        reply.is_some_and(|r| r.ok && r.value.as_bool() == Some(true))
    }
}

fn absolute(p: &std::path::Path) -> String {
    std::path::absolute(p).unwrap_or_else(|_| p.to_owned()).display().to_string()
}
