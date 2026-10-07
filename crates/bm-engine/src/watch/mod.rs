//! `MapUpdateService`: watches a map's region folder and schedules region updates (debounced like Java: at least
//! 5 s after the last change, `update-cooldown` apart per region) plus a full update every `full-update-interval`.
//!
//! Unlike Java, it never stops watching silently (docs/08 "Watchers die silently"): a missing folder is waited
//! for, watcher errors are logged and the watcher re-created, lost events trigger a fingerprint rescan, and every
//! `region-file-check-interval` the folder is rescanned anyway.

mod files;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bm_config::CoreConfig;
use bm_format::grid::Tile;
use bm_map::renderstate::TileUpdateStrategy;
use files::{BoxedWatcher, Fingerprints, Msg};

use crate::error::{Error, Result};
use crate::queue::RenderQueue;
use crate::task::RenderTask;

/// How often a missing region folder or a failed watcher is retried.
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
pub struct WatchSettings {
    /// Quiet time after a region file's last change before it is updated (Java: 5 s).
    pub debounce: Duration,
    pub update_cooldown: Duration,
    /// Zero disables the periodic full update.
    pub full_update_interval: Duration,
    /// Zero disables the periodic rescan.
    pub scan_interval: Duration,
}

impl WatchSettings {
    pub fn from_core(core: &CoreConfig) -> Self {
        Self {
            debounce: Duration::from_secs(5),
            update_cooldown: core.update_cooldown_duration(),
            full_update_interval: core.full_update_interval_duration(),
            scan_interval: core.region_file_check_interval_duration(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

pub type LogFn = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;

pub struct MapUpdateService {
    tx: Sender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl MapUpdateService {
    pub fn start(
        map: &str,
        region_dir: PathBuf,
        queue: Arc<RenderQueue>,
        settings: WatchSettings,
        log: LogFn,
    ) -> Result<Self> {
        let (tx, rx) = channel();
        let now = Instant::now();
        let mut service = Service {
            map: map.to_owned(),
            dir: region_dir,
            queue,
            settings,
            log,
            tx: tx.clone(),
            watcher: None,
            files: Fingerprints::default(),
            armed: HashMap::new(),
            last_scheduled: HashMap::new(),
            next_full: positive(settings.full_update_interval).map(|d| now + d),
            next_scan: positive(settings.scan_interval).map(|d| now + d),
            next_retry: now,
        };
        let handle = std::thread::Builder::new()
            .name(format!("bluemap-watch-{map}"))
            .spawn(move || {
                (service.log)(LogLevel::Info, &format!("Started watching map '{}' for updates...", service.map));
                service.try_watch(false);
                loop {
                    let timeout = service.next_deadline().saturating_duration_since(Instant::now());
                    match rx.recv_timeout(timeout) {
                        Ok(Msg::Close) | Err(RecvTimeoutError::Disconnected) => break,
                        Ok(msg) => service.handle(msg),
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    service.on_time(Instant::now());
                }
                (service.log)(LogLevel::Info, &format!("Stopped watching map '{}' for updates.", service.map));
            })
            .map_err(|e| Error::Invalid(format!("failed to start the update-watcher for map '{map}': {e}")))?;
        Ok(Self { tx, handle: Some(handle) })
    }

    /// Stops watching; pending (debounced) region updates are dropped, like Java.
    pub fn close(mut self) {
        let _ = self.tx.send(Msg::Close);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for MapUpdateService {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Close);
    }
}

fn positive(d: Duration) -> Option<Duration> {
    (!d.is_zero()).then_some(d)
}

struct Service {
    map: String,
    dir: PathBuf,
    queue: Arc<RenderQueue>,
    settings: WatchSettings,
    log: LogFn,
    tx: Sender<Msg>,
    watcher: Option<BoxedWatcher>,
    files: Fingerprints,
    /// Region → when its debounced update is due.
    armed: HashMap<Tile, Instant>,
    last_scheduled: HashMap<Tile, Instant>,
    next_full: Option<Instant>,
    next_scan: Option<Instant>,
    next_retry: Instant,
}

impl Service {
    fn log(&self, level: LogLevel, msg: &str) {
        (self.log)(level, msg);
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Changed(regions) => regions.into_iter().for_each(|r| self.arm(r)),
            Msg::Rescan => {
                self.log(
                    LogLevel::Warning,
                    &format!("Region-file watcher for map '{}' lost events, rescanning", self.map),
                );
                self.rescan();
            }
            Msg::Error(e) => {
                self.log(
                    LogLevel::Warning,
                    &format!("Region-file watcher for map '{}' failed: {e} (restarting it)", self.map),
                );
                self.watcher = None;
                self.next_retry = Instant::now() + RETRY_INTERVAL;
            }
            Msg::Close => {}
        }
    }

    fn next_deadline(&self) -> Instant {
        let retry = Some(self.next_retry);
        [retry, self.next_full, self.next_scan, self.armed.values().min().copied()]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(self.next_retry)
    }

    fn on_time(&mut self, now: Instant) {
        if now >= self.next_retry {
            self.next_retry = now + RETRY_INTERVAL;
            self.check_watcher();
        }
        let due: Vec<Tile> = self.armed.iter().filter(|&(_, &at)| at <= now).map(|(&r, _)| r).collect();
        for region in due {
            self.armed.remove(&region);
            self.schedule_region(region, now);
        }
        if let (Some(at), Some(every)) = (self.next_full, positive(self.settings.full_update_interval))
            && at <= now
        {
            self.log(LogLevel::Info, &format!("Start updating map '{}'...", self.map));
            self.queue.schedule_next(RenderTask::full(self.map.clone(), TileUpdateStrategy::ForceNone));
            let mut next = at + every;
            while next <= now {
                next += every;
            }
            self.next_full = Some(next);
        }
        if let (Some(at), Some(every)) = (self.next_scan, positive(self.settings.scan_interval))
            && at <= now
        {
            self.next_scan = Some(now + every);
            self.rescan();
        }
    }

    /// A watcher whose folder vanished is dropped; a missing watcher is (re)started once the folder exists.
    fn check_watcher(&mut self) {
        let exists = self.dir.is_dir();
        if self.watcher.is_some() && !exists {
            self.log(
                LogLevel::Warning,
                &format!("Region folder of map '{}' disappeared: {}", self.map, self.dir.display()),
            );
            self.watcher = None;
            self.rescan();
        } else if self.watcher.is_none() && exists {
            self.try_watch(true);
        }
    }

    /// Starts the watcher if the folder exists. `rescan`: compare fingerprints to catch changes made while not
    /// watching; otherwise only record them (the initial full update covers the current files).
    fn try_watch(&mut self, rescan: bool) {
        if !self.dir.is_dir() {
            return;
        }
        let (watcher, native_error) = files::watch(&self.dir, &self.tx);
        match watcher {
            Ok(w) => {
                if let Some(e) = native_error {
                    self.log(
                        LogLevel::Warning,
                        &format!("Can't watch '{}' for changes ({e}), polling it instead", self.dir.display()),
                    );
                }
                self.watcher = Some(w);
            }
            Err(e) => {
                self.log(LogLevel::Error, &format!("Failed to watch map '{}' for updates: {e} (retrying)", self.map));
                return;
            }
        }
        if rescan {
            self.rescan();
        } else if let Err(e) = self.files.rescan(&self.dir) {
            self.log(LogLevel::Warning, &format!("Failed to list region files of map '{}': {e}", self.map));
        }
    }

    fn rescan(&mut self) {
        match self.files.rescan(&self.dir) {
            Ok(changed) => changed.into_iter().for_each(|r| self.arm(r)),
            Err(e) => self.log(LogLevel::Warning, &format!("Failed to list region files of map '{}': {e}", self.map)),
        }
    }

    /// `updateRegion`: (re)arms the region's timer, `debounce` out and `update-cooldown` after its last update.
    fn arm(&mut self, region: Tile) {
        let now = Instant::now();
        let since_last = self.last_scheduled.get(&region).map_or(Duration::MAX, |t| now.duration_since(*t));
        let delay = self.settings.update_cooldown.saturating_sub(since_last).max(self.settings.debounce);
        self.armed.insert(region, now + delay);
    }

    fn schedule_region(&mut self, region: Tile, now: Instant) {
        let cooldown = self.settings.update_cooldown;
        self.last_scheduled.retain(|_, t| now.duration_since(*t) < cooldown);
        self.queue.schedule(RenderTask::region(self.map.clone(), region));
        self.last_scheduled.insert(region, now);
        let (x, z) = region;
        self.log(LogLevel::Info, &format!("Scheduled update for region-file: ({x}, {z}) (Map: {})", self.map));
    }
}

#[cfg(test)]
mod tests;
