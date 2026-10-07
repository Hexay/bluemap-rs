//! One load cycle of `Plugin.load()`/`unload()`: config, resources, maps, webserver, render worker, watchers and
//! the persisted plugin state. A reload drops the session and builds a new one.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bm_config::generate::{ServerWorld, suggest_render_thread_count};
use bm_config::{BlueMapConfig, ConfigOptions, Key};
use bm_engine::{
    LoadedMaps, LogLevel, MapContext, MapUpdateService, RenderQueue, RenderTask, ResourceOptions, Service, TaskEvent,
    TileUpdateStrategy, WatchSettings, run_queue,
};
use bm_ipc::{CoreWorld, WorldInfo};
use bm_web::LiveMap;

use super::state::{self, PluginState};
use super::{Hello, blockstates};
use crate::log;
use crate::web::{self, Webserver};

/// Why a load ended without a usable session (`NotReady.reason`).
pub struct NotReady {
    pub reason: &'static str,
    pub message: String,
    /// `no-maps` keeps the webserver running, like upstream's `unload(true)`.
    pub kept: Option<Box<Session>>,
}

pub struct Session {
    pub service: Arc<Service>,
    pub maps: Arc<LoadedMaps>,
    pub queue: Arc<RenderQueue>,
    pub lives: BTreeMap<String, Arc<LiveMap>>,
    pub state: Mutex<PluginState>,
    pub worlds: Vec<CoreWorld>,
    /// Map id → `WorldInfo.id` of the server world it renders (`Server.getServerWorld(World)`).
    pub map_server_world: HashMap<String, Option<String>>,
    pub loaded_at: Instant,
    worker: Mutex<Option<JoinHandle<()>>>,
    watchers: Mutex<HashMap<String, MapUpdateService>>,
    web: Mutex<Option<(Webserver, tokio::sync::oneshot::Sender<()>)>>,
}

pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn config_options(hello: &Hello, worlds: &[WorldInfo]) -> ConfigOptions {
    let server_worlds = worlds
        .iter()
        .map(|w| ServerWorld {
            world_folder: relative_to_cwd(Path::new(&w.folder)),
            dimension: Key::parse(&w.dimension),
            dimension_type: w.dimension_type.as_deref().map(Key::parse),
        })
        .collect();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    ConfigOptions {
        render_thread_count: suggest_render_thread_count(cores, hello.max_memory_mib),
        ..ConfigOptions::server(&hello.config_folder, server_worlds, false)
    }
}

/// Generated configs hold paths relative to the server folder, as upstream's `formatPath` writes them.
fn relative_to_cwd(p: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    p.strip_prefix(&cwd).map(Path::to_owned).unwrap_or_else(|_| p.to_owned())
}

fn not_ready(reason: &'static str, message: String) -> NotReady {
    NotReady { reason, message, kept: None }
}

impl Session {
    /// `Plugin.load()`, logging like upstream along the way. `scripts`: API-registered scripts and styles.
    pub fn load(hello: &Hello, worlds: &[WorldInfo], scripts: (&[String], &[String])) -> Result<Self, NotReady> {
        let packs = hello.config_folder.join("packs");
        let _ = std::fs::create_dir_all(&packs);
        bm_engine::find_java_addons(&packs).iter().for_each(|a| log::warn(&a.warning()));
        let config = BlueMapConfig::load(&config_options(hello, worlds))
            .map_err(|e| not_ready("config-error", format!("{e}")))?;
        if let Some(file) = &config.core.log.file
            && let Err(e) = log::add_formatted_file(file, config.core.log.append)
        {
            log::warn(&format!("Failed to open log file {file}: {e}"));
        }
        let (plugin_state, warning) = state::load(&config.core.data);
        if let Some(w) = warning {
            log::warn(w);
        }
        blockstates::write_pack(&config.core.data, &hello.blockstates)
            .map_err(|e| not_ready("config-error", format!("{e:#}")))?;
        let options = ResourceOptions {
            minecraft_version: Some(hello.mc_version.clone()),
            packs_folder: Some(packs),
            mods_folder: hello.mods_folder.clone().filter(|m| m.is_dir()),
        };
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        super::init_render_pool(config.core.resolve_render_thread_count(cores));
        let service = Arc::new(Service::new(config, options));
        if let Err(e) = service.resources() {
            if !matches!(e, bm_engine::Error::MissingResources(_)) {
                return Err(not_ready("config-error", e.to_string()));
            }
            log::warn("BlueMap is missing important resources!");
            log::warn("You must accept the required file download in order for BlueMap to work!");
            let core = bm_config::resolve_config_file(&hello.config_folder, "core");
            log::warn(&format!("Please check: {}", std::path::absolute(&core).unwrap_or(core).display()));
            log::info("If you have changed the config you can simply reload the plugin using: /bluemap reload");
            return Err(not_ready("missing-resources", e.to_string()));
        }

        let maps = Arc::new(LoadedMaps::default());
        let mut lives = BTreeMap::new();
        for id in service.map_ids(|_| true) {
            match crate::render::open_map(&service, &id) {
                Ok(Some(mut map)) => {
                    lives.insert(id, live_map(&service, &mut map));
                    maps.insert(map);
                }
                Ok(None) => {}
                Err(e) => log::error(&format!("Failed to load map '{id}': {e}")),
            }
        }
        let all = maps.all();
        let session = Self {
            worlds: core_worlds(&all, worlds),
            map_server_world: all.iter().map(|m| (m.id.clone(), server_world_of(m, worlds))).collect(),
            service,
            maps,
            queue: Arc::new(RenderQueue::new()),
            lives,
            state: Mutex::new(plugin_state),
            loaded_at: Instant::now(),
            worker: Mutex::default(),
            watchers: Mutex::default(),
            web: Mutex::default(),
        };
        if session.service.config.webserver.enabled
            && let Err(e) = session.start_webserver()
        {
            log::warn(&format!("{e:#}"));
            return Err(not_ready("config-error", format!("{e:#}")));
        }
        if all.is_empty() {
            log::warn("There are no valid maps configured, please check your map-configs! Disabling BlueMap...");
            return Err(NotReady { reason: "no-maps", message: "no valid maps".into(), kept: Some(Box::new(session)) });
        }
        session.write_webapp(scripts);
        session.start_worker();
        session.schedule_full_updates();
        session.start_watchers();
        Ok(session)
    }

    /// `createOrUpdateWebApp(false)` with the API's scripts/styles.
    pub fn write_webapp(&self, (scripts, styles): (&[String], &[String])) {
        if !self.service.config.webapp.enabled {
            return;
        }
        let installed = bm_web::install_webapp(&self.service.config.webapp.webroot, false).map_err(|e| e.to_string());
        let written =
            installed.and_then(|_| self.service.write_webapp_settings_with(scripts, styles).map_err(|e| e.to_string()));
        if let Err(e) = written {
            log::error(&format!("Failed to update webapp settings: {e}"));
        }
    }

    fn start_webserver(&self) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let server = web::serve(&self.service, false, &self.lives, Some(2), async move {
            let _ = rx.await;
        })?;
        *self.web.lock().unwrap_or_else(PoisonError::into_inner) = Some((server, tx));
        Ok(())
    }

    /// `RenderManager` with one worker (tiles fan out on the rayon pool); paused while render threads are off.
    fn start_worker(&self) {
        if !self.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled {
            self.queue.pause();
            log::info("Render-Threads are STOPPED! Use the command 'bluemap start' to start them.");
        }
        let (queue, maps, service) = (self.queue.clone(), self.maps.clone(), self.service.clone());
        let worker = std::thread::Builder::new()
            .name("bluemap-render".into())
            .spawn(move || {
                if let Ok(resources) = service.resources() {
                    run_queue(&queue, &maps, resources, false, &mut log_task_event);
                }
            })
            .expect("spawn render worker");
        *self.worker.lock().unwrap_or_else(PoisonError::into_inner) = Some(worker);
    }

    /// Upstream's load-time full updates: unfrozen maps whose last full update is older than the interval.
    fn schedule_full_updates(&self) {
        let interval = self.service.config.core.full_update_interval_duration().as_secs() as i64;
        if interval <= 0 {
            return;
        }
        let now = now_secs();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut due: Vec<Arc<MapContext>> = self
            .maps
            .all()
            .into_iter()
            .filter(|m| !state.is_frozen(&m.id) && state.map(&m.id).last_full_update + interval <= now)
            .collect();
        due.sort_by_key(|m| m.config.sorting);
        for map in due.iter().rev() {
            state.map(&map.id).last_full_update = now;
            self.queue.schedule_next(RenderTask::full(map.id.clone(), TileUpdateStrategy::ForceNone));
        }
    }

    fn start_watchers(&self) {
        let ids: Vec<String> = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            self.maps.all().iter().filter(|m| !state.is_frozen(&m.id)).map(|m| m.id.clone()).collect()
        };
        ids.iter().for_each(|id| self.start_watching(id));
    }

    pub fn start_watching(&self, id: &str) {
        let Some(map) = self.maps.get(id) else { return };
        let settings = WatchSettings::from_core(&self.service.config.core);
        let log = Arc::new(|level: LogLevel, msg: &str| match level {
            LogLevel::Info => log::info(msg),
            LogLevel::Warning => log::warn(msg),
            LogLevel::Error => log::error(msg),
        });
        self.stop_watching(id);
        match MapUpdateService::start(id, map.world.region_dir().to_owned(), self.queue.clone(), settings, log) {
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

    pub fn is_loaded(&self) -> bool {
        !self.maps.all().is_empty()
    }

    /// `Plugin.save()` minus tasks.dat: plugin state; markers are written by the caller (it holds the JSON).
    pub fn save_state(&self) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Err(e) = state.save(&self.service.config.core.data) {
            log::error(&format!("Failed to save pluginState.json! {e}"));
        }
    }

    /// `Plugin.unload()` after the caller saved markers: empty players, stop rendering, watchers and webserver.
    pub fn unload(&self) {
        let ids: Vec<String> = self.watchers.lock().unwrap_or_else(PoisonError::into_inner).keys().cloned().collect();
        ids.iter().for_each(|id| self.stop_watching(id));
        self.save_state();
        for map in self.maps.all() {
            if let Err(e) = map.storage.write_item(&bm_storage::ItemKey::Players, b"{}") {
                log::error(&format!("Failed to save players for map '{}'! {e}", map.id));
            }
        }
        self.queue.stop();
        if let Some(worker) = self.worker.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = worker.join();
        }
        if let Some((server, stop)) = self.web.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = stop.send(());
            if let Err(e) = server.wait() {
                log::error(&format!("Failed to close the webserver! {e:#}"));
            }
        }
        log::close_files();
    }
}

/// The map's live data: markers from its config until the shim pushes, players if enabled, SSE tile events.
pub fn live_map(service: &Service, map: &mut MapContext) -> Arc<LiveMap> {
    let sse = service.config.webserver.sse_enabled;
    let players = service.config.plugin.live_player_markers;
    let mut live = LiveMap::new(sse).with_markers();
    if players {
        live = live.with_players();
    }
    let live = Arc::new(live);
    live.set_markers(map.markers_json.clone());
    if players {
        live.set_players(bm_map::players::players_json([]));
    }
    web::hook_tiles(map, &live, sse);
    live
}

fn log_task_event(event: TaskEvent) {
    match event {
        TaskEvent::Update(_, bm_engine::UpdateEvent::Warning(w)) => log::warn(&w),
        TaskEvent::Finished(task, Err(e)) => log::error(&format!("Failed to update map '{}': {e}", task.map)),
        TaskEvent::Finished(task, Ok(s)) => log::info(&format!(
            "Map '{}': {} regions, {} tiles rendered, {} skipped, {} deleted, {} failed",
            task.map, s.regions, s.tiles_rendered, s.tiles_skipped, s.tiles_deleted, s.tile_errors
        )),
        _ => {}
    }
}

/// `World.id`: the folder relative to the server folder when inside it, then `#` and the dimension.
pub fn world_id(map: &MapContext) -> String {
    let abs = std::path::absolute(&map.world.path).unwrap_or_else(|_| map.world.path.clone());
    format!("{}#{}", relative_to_cwd(&abs).display(), map.world.dimension)
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_owned());
    norm(a) == norm(b)
}

fn server_world_of(map: &MapContext, worlds: &[WorldInfo]) -> Option<String> {
    worlds
        .iter()
        .find(|w| w.dimension == map.world.dimension && same_folder(Path::new(&w.folder), &map.world.path))
        .map(|w| w.id.clone())
}

fn core_worlds(maps: &[Arc<MapContext>], worlds: &[WorldInfo]) -> Vec<CoreWorld> {
    let mut out: Vec<CoreWorld> = Vec::new();
    for map in maps {
        let id = world_id(map);
        if out.iter().any(|w| w.id == id) {
            continue;
        }
        let save = map.world.region_dir().parent().unwrap_or(map.world.region_dir());
        out.push(CoreWorld {
            id,
            save_folder: std::path::absolute(save).unwrap_or_else(|_| save.to_owned()).display().to_string(),
            server_world: server_world_of(map, worlds),
        });
    }
    out
}
