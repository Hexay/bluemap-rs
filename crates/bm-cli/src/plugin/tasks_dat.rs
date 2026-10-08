//! The plugin's `tasks.dat` (`Plugin.save`/`load`); format in [`crate::tasks_dat`].

use bm_engine::RenderTask;

use super::session::Session;
use super::{ops, spawn};
use crate::log;
use crate::tasks_dat::{decode, encode, file, write};

/// `Plugin.save()`'s tasks part: the current task (with its done regions) first, then the queue.
pub fn save(s: &Session) {
    let tasks: Vec<RenderTask> = s.queue.current_task().into_iter().chain(s.queue.pending_tasks()).collect();
    let bytes = encode(&tasks, &[], &|map| s.maps.get(map).and_then(|m| m.world.regions().ok()));
    if let Err(e) = write(&s.service.config.core.data, &bytes) {
        log::error(&format!("Failed to save tasks.dat! {e}"));
    }
}

/// `Plugin.load()`'s tasks part: queues the saved tasks and runs saved purges.
pub fn resume(s: &std::sync::Arc<Session>) {
    let path = file(&s.service.config.core.data);
    let Ok(bytes) = std::fs::read(&path) else { return };
    let loaded = match decode(&bytes, &|map| s.maps.get(map).is_some()) {
        Ok(l) => l,
        Err(e) => {
            log::error(&format!("Failed to load tasks.dat! {e}"));
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    for task in loaded.tasks {
        s.queue.schedule(task);
    }
    for map in loaded.purges {
        let s = s.clone();
        spawn("bluemap-purge", move || {
            if let Err(e) = ops::purge(&s, &map) {
                log::error(&format!("Failed to purge map '{map}': {e:#}"));
            }
        });
    }
}
