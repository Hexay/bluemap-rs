//! `-u`: one `MapUpdateService` per map, plus (beyond Java) retrying maps that failed to load, so a map whose
//! world folder appears later still gets rendered and watched.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bm_engine::{
    LoadedMaps, LogFn, LogLevel, MapContext, MapUpdateService, RenderQueue, RenderTask, Service, TileUpdateStrategy,
    WatchSettings,
};

use crate::log;

const RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// The watcher's log lines into ours.
pub fn engine_log() -> LogFn {
    Arc::new(|level: LogLevel, msg: &str| match level {
        LogLevel::Info => log::info(msg),
        LogLevel::Warning => log::warn(msg),
        LogLevel::Error => log::error(msg),
    })
}

pub struct Watchers<'a> {
    service: &'a Service,
    queue: Arc<RenderQueue>,
    settings: WatchSettings,
    running: Mutex<Vec<MapUpdateService>>,
}

impl<'a> Watchers<'a> {
    pub fn new(service: &'a Service, queue: Arc<RenderQueue>) -> Self {
        let settings = WatchSettings::from_core(&service.config.core);
        Self { service, queue, settings, running: Mutex::default() }
    }

    pub fn start(&self, map: &MapContext) {
        let dir = map.world.region_dir().to_owned();
        match MapUpdateService::start(&map.id, dir, self.queue.clone(), self.settings, engine_log()) {
            Ok(watcher) => self.running.lock().unwrap_or_else(PoisonError::into_inner).push(watcher),
            Err(e) => log::error(&format!(
                "Failed to create update-watcher for map: {} (This means the map might not automatically update): {e}",
                map.id
            )),
        }
    }

    /// Re-opens each failed map every 30 s until it loads, then `prepare`s (e.g. gives it live web routes), renders
    /// and watches it. Returns once `stop`'s sender is dropped.
    pub fn retry_failed(
        &self,
        mut failed: Vec<String>,
        loaded: &LoadedMaps,
        prepare: &dyn Fn(&mut MapContext),
        stop: Receiver<()>,
    ) {
        while !failed.is_empty() && matches!(stop.recv_timeout(RETRY_INTERVAL), Err(RecvTimeoutError::Timeout)) {
            failed.retain(|id| match self.service.open_map(id) {
                Ok(Some(mut map)) => {
                    log::info(&format!("Loading map '{id}'..."));
                    prepare(&mut map);
                    let map = loaded.insert(map);
                    self.start(&map);
                    self.queue.schedule(RenderTask::full(id.clone(), TileUpdateStrategy::ForceNone));
                    false
                }
                Ok(None) => false,
                Err(_) => true,
            });
        }
    }

    pub fn close(&self) {
        let running = std::mem::take(&mut *self.running.lock().unwrap_or_else(PoisonError::into_inner));
        running.into_iter().for_each(MapUpdateService::close);
    }
}
