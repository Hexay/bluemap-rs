//! `BlueMapCLI.renderMaps`: webapp settings, resources and maps, then a render queue worked off on this thread,
//! with BlueMap's progress log every 10 s; with `-u` the file watchers keep feeding it until shutdown.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use anyhow::Result;
use bm_engine::{
    LoadedMaps, MapContext, RenderQueue, RenderTask, Service, TaskEvent, TileUpdateStrategy, UpdateEvent, UpdateStats,
    run_queue,
};
use bm_web::MapRegistry;

use crate::eta::{self, ProgressTracker};
use crate::log;
use crate::shutdown::Shutdown;
use crate::watch::Watchers;

/// BlueMap's CLI reports progress every 10 s.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

/// `createOrUpdateWebApp`, resources, then every selected map; maps that failed to load are returned by id.
pub fn load_maps(service: &Service, maps: Option<&str>, force_webapp: bool) -> Result<(Vec<MapContext>, Vec<String>)> {
    if service.config.webapp.enabled {
        bm_web::install_webapp(&service.config.webapp.webroot, force_webapp)?;
        service.write_webapp_settings()?;
    }
    service.resources()?;
    let selected: Option<Vec<&str>> = maps.map(|m| m.split(',').collect());
    let (mut loaded, mut failed) = (Vec::new(), Vec::new());
    for id in service.map_ids(|id| selected.as_ref().is_none_or(|s| s.contains(&id))) {
        match open_map(service, &id) {
            Ok(Some(map)) => loaded.push(map),
            Ok(None) => {}
            Err(e) => {
                log::error(&format!("Failed to load map '{id}': {e}"));
                failed.push(id);
            }
        }
    }
    Ok((loaded, failed))
}

pub fn open_map(service: &Service, id: &str) -> bm_engine::Result<Option<MapContext>> {
    let map = service.open_map(id)?;
    match &map {
        Some(map) => {
            log::info(&format!("Loading map '{id}'..."));
            map.warnings.iter().for_each(|w| log::warn(w));
        }
        None => log::info(&format!(
            "The map '{id}' has no world configured. The map will be displayed, but it will not be updated by this \
             bluemap instance!"
        )),
    }
    Ok(map)
}

/// Renders `maps` (and with `watch`, keeps updating them until shutdown). Maps loaded later get live routes on
/// `web` (the running webserver's maps). Returns whether no tile failed.
pub fn run(
    service: &Service,
    maps: Vec<MapContext>,
    failed: Vec<String>,
    strategy: TileUpdateStrategy,
    watch: bool,
    web: Option<&MapRegistry>,
    shutdown: &Shutdown,
) -> Result<bool> {
    let queue = Arc::new(RenderQueue::new());
    let loaded = LoadedMaps::default();
    let count = maps.len();
    for map in maps {
        let id = map.id.clone();
        loaded.insert(map);
        queue.schedule(RenderTask::full(id, strategy));
    }
    let watchers = Watchers::new(service, queue.clone());
    if watch {
        loaded.all().iter().for_each(|m| watchers.start(m));
    }
    {
        let queue = queue.clone();
        shutdown.on_trigger(move || {
            log::info("Stopping...");
            queue.stop();
        });
    }
    log::info(&format!("Start updating {count} maps ..."));

    let resources = service.resources()?;
    // dropping the senders stops the helper threads
    let (progress_stop, progress_rx) = channel::<()>();
    let (retry_stop, retry_rx) = channel::<()>();
    let start = Instant::now();
    let mut report = Report::default();
    let sse = service.config.webserver.sse_enabled;
    let prepare = |map: &mut MapContext| {
        if let Some(web) = web {
            crate::web::attach_live(web, map, sse);
        }
    };
    std::thread::scope(|s| {
        s.spawn(|| progress_log(&queue, progress_rx));
        if watch && !failed.is_empty() {
            s.spawn(|| watchers.retry_failed(failed, &loaded, &prepare, retry_rx));
        }
        run_queue(&queue, &loaded, resources, !watch, &mut |event| report.on_event(event, &queue, start));
        drop((progress_stop, retry_stop));
    });
    watchers.close();
    if !shutdown.is_triggered() {
        log::info("Stopping...");
    }
    log::info("Saving...");
    log::info("Stopped.");
    Ok(!report.failed)
}

#[derive(Default)]
struct Report {
    failed: bool,
    up_to_date: bool,
    total: UpdateStats,
}

impl Report {
    fn on_event(&mut self, event: TaskEvent, queue: &RenderQueue, start: Instant) {
        let finished = matches!(event, TaskEvent::Finished(..));
        match event {
            TaskEvent::Started(_) | TaskEvent::Update(_, UpdateEvent::Progress(_)) => {}
            TaskEvent::Update(_, UpdateEvent::Warning(w)) => log::warn(&w),
            TaskEvent::Finished(task, Ok(s)) => {
                log::info(&format!(
                    "Map '{}': {} regions, {} tiles rendered, {} skipped, {} deleted, {} lowres tiles saved",
                    task.map, s.regions, s.tiles_rendered, s.tiles_skipped, s.tiles_deleted, s.lowres_saves
                ));
                self.failed |= s.tile_errors > 0;
                add(&mut self.total, s);
            }
            TaskEvent::Finished(task, Err(e)) => {
                self.failed = true;
                log::error(&format!("Failed to update map '{}': {e}", task.map));
            }
        }
        if finished && !self.up_to_date && queue.pending() == 0 && !queue.is_stopped() {
            self.up_to_date = true;
            log::info("Your maps are now all up-to-date!");
            log::info(&format!(
                "({} tiles rendered in {:.1}s)",
                self.total.tiles_rendered,
                start.elapsed().as_secs_f64()
            ));
        }
    }
}

/// BlueMapCLI's `updateInfoTask`, fed by `RenderManager`'s progress tracker on the same thread.
fn progress_log(queue: &RenderQueue, stop: Receiver<()>) {
    let start = Instant::now();
    let mut tracker = ProgressTracker::default();
    let mut next_log = PROGRESS_INTERVAL;
    let mut was_idle = false;
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(eta::SAMPLE_INTERVAL) {
        let now = start.elapsed();
        tracker.sample(queue.current_run(), now.as_millis() as i64);
        if now < next_log {
            continue;
        }
        next_log += PROGRESS_INTERVAL;
        match queue.current() {
            None => {
                if !was_idle {
                    log::info("Waiting for changes on the world-files...");
                }
                was_idle = true;
            }
            Some((description, progress)) => {
                was_idle = false;
                let percent = java_double((progress * 100_000.0).round() / 1000.0);
                log::info(&format!("{description}: {percent}%{}", eta::suffix(tracker.remaining_ms(progress))));
            }
        }
    }
}

/// `Double.toString` for the plain-notation range progress values live in.
fn java_double(v: f64) -> String {
    if v.fract() == 0.0 { format!("{v:.1}") } else { v.to_string() }
}

fn add(total: &mut UpdateStats, s: &UpdateStats) {
    total.regions += s.regions;
    total.tiles_rendered += s.tiles_rendered;
    total.tiles_skipped += s.tiles_skipped;
    total.tiles_deleted += s.tiles_deleted;
    total.tile_errors += s.tile_errors;
    total.lowres_saves += s.lowres_saves;
}

#[cfg(test)]
mod tests {
    #[test]
    fn progress_prints_like_java() {
        assert_eq!(super::java_double(0.0), "0.0");
        assert_eq!(super::java_double(12.345), "12.345");
        assert_eq!(super::java_double(100.0), "100.0");
    }
}
