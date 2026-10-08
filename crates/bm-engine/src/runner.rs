//! `RenderManager`'s worker: takes tasks off a [`RenderQueue`] and runs them as map updates. A failing or
//! panicking task is reported and the worker carries on, so updates never stop silently.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, PoisonError, RwLock};

use crate::error::{Error, Result};
use crate::map::MapContext;
use crate::job::JobControl;
use crate::queue::{Next, RenderQueue};
use crate::resources::Resources;
use crate::task::{Regions, RenderTask};
use crate::update::{UpdateEvent, UpdateJob, UpdateStats, update_map};

/// The maps a worker can update; maps may be added while it runs (e.g. once their world appears).
#[derive(Default)]
pub struct LoadedMaps(RwLock<BTreeMap<String, Arc<MapContext>>>);

impl LoadedMaps {
    pub fn insert(&self, map: MapContext) -> Arc<MapContext> {
        let map = Arc::new(map);
        self.0.write().unwrap_or_else(PoisonError::into_inner).insert(map.id.clone(), map.clone());
        map
    }

    pub fn get(&self, id: &str) -> Option<Arc<MapContext>> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).get(id).cloned()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.read().unwrap_or_else(PoisonError::into_inner).contains_key(id)
    }

    pub fn all(&self) -> Vec<Arc<MapContext>> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).values().cloned().collect()
    }
}

pub enum TaskEvent<'a> {
    Started(&'a RenderTask),
    Update(&'a RenderTask, UpdateEvent<'a>),
    Finished(&'a RenderTask, &'a Result<UpdateStats>),
}

/// Works off `queue` until it is stopped, or until it runs empty when `exit_when_idle`.
pub fn run_queue(
    queue: &RenderQueue,
    maps: &LoadedMaps,
    resources: &Resources,
    exit_when_idle: bool,
    on_event: &mut dyn FnMut(TaskEvent),
) {
    while let Some(next) = queue.take(exit_when_idle) {
        let taken = match next {
            Next::Task(taken) => taken,
            Next::Job(job, cancel) => {
                // a job reports its own errors; a panic must not take the worker down
                let _ = catch_unwind(AssertUnwindSafe(|| (job.work)(&JobControl { queue, cancel: &cancel })));
                continue;
            }
        };
        let task = &taken.task;
        let map = maps.get(&task.map);
        let regions = match &map {
            Some(map) => task.remaining(|| map.world.regions()),
            None => Ok(task.regions.clone()),
        };
        if matches!(&regions, Ok(Regions::Only(left)) if left.is_empty()) && !task.done.is_empty() {
            continue;
        }
        on_event(TaskEvent::Started(task));
        let result = match (map, regions) {
            (None, _) => Err(Error::Invalid(format!("map '{}' is not loaded", task.map))),
            (_, Err(e)) => Err(e.into()),
            (Some(map), Ok(regions)) => {
                let job = UpdateJob { strategy: task.strategy, regions: &regions, cancel: &taken.cancel };
                let mut forward = |event: UpdateEvent| {
                    if let UpdateEvent::Progress(s) = &event {
                        queue.set_progress(s.regions_done as f64 / s.regions.max(1) as f64);
                        if let Some(region) = s.last_region {
                            queue.region_done(region);
                        }
                    }
                    on_event(TaskEvent::Update(task, event));
                };
                catch_unwind(AssertUnwindSafe(|| update_map(&map, resources, job, &mut forward))).unwrap_or_else(
                    |panic| Err(Error::Invalid(format!("render task panicked: {}", panic_message(&panic)))),
                )
            }
        };
        on_event(TaskEvent::Finished(task, &result));
    }
    queue.finish();
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown cause".to_owned())
}
