//! `RenderManager`'s task list: one running task plus a deduplicated queue. Producers (file watchers, timers,
//! commands) schedule from any thread; [`crate::run_queue`] works it off.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::task::{Regions, RenderTask};

#[derive(Default)]
pub struct RenderQueue {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    pending: VecDeque<RenderTask>,
    current: Option<Running>,
    stopped: bool,
}

struct Running {
    task: RenderTask,
    cancel: Arc<AtomicBool>,
    progress: f64,
}

/// What [`RenderQueue::take`] hands the worker.
pub(crate) struct Taken {
    pub task: RenderTask,
    pub cancel: Arc<AtomicBool>,
}

impl RenderQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `scheduleRenderTask`: appended unless a queued task already contains it; queued tasks it contains are
    /// dropped and a running one it contains is cancelled. False when it was redundant or the queue is stopped.
    pub fn schedule(&self, task: RenderTask) -> bool {
        self.insert(task, false)
    }

    /// `scheduleRenderTaskNext`: like [`RenderQueue::schedule`], but ahead of every queued task.
    pub fn schedule_next(&self, task: RenderTask) -> bool {
        self.insert(task, true)
    }

    fn insert(&self, task: RenderTask, front: bool) -> bool {
        let mut s = self.lock();
        if s.stopped || s.pending.iter().any(|t| t.contains(&task)) {
            return false;
        }
        s.pending.retain(|t| !task.contains(t));
        if let Some(running) = &s.current
            && task.contains(&running.task)
        {
            running.cancel.store(true, Ordering::Relaxed);
        }
        if front {
            s.pending.push_front(task)
        } else {
            s.pending.push_back(task)
        }
        self.changed.notify_all();
        true
    }

    /// `removeAllRenderTasks` + `stop`: drops the queue, cancels the running task and makes the worker return
    /// once it has wound down. Later schedules are ignored.
    pub fn stop(&self) {
        let mut s = self.lock();
        s.stopped = true;
        s.pending.clear();
        if let Some(running) = &s.current {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.changed.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    /// The running task's description and estimated progress (0..1).
    pub fn current(&self) -> Option<(String, f64)> {
        self.lock().current.as_ref().map(|r| (r.task.description(), r.progress))
    }

    pub fn pending(&self) -> usize {
        self.lock().pending.len()
    }

    pub fn is_idle(&self) -> bool {
        let s = self.lock();
        s.current.is_none() && s.pending.is_empty()
    }

    /// Next task, merged with every queued region update of the same map and strategy (one pass over shared
    /// tiles instead of one per region). Blocks while the queue is empty, unless `exit_when_idle`; `None` once
    /// stopped (or idle with `exit_when_idle`).
    pub(crate) fn take(&self, exit_when_idle: bool) -> Option<Taken> {
        let mut s = self.lock();
        s.current = None;
        self.changed.notify_all();
        loop {
            if s.stopped {
                return None;
            }
            if let Some(mut task) = s.pending.pop_front() {
                if matches!(task.regions, Regions::Only(_)) {
                    s.pending.retain(|t| !task.absorb(t));
                }
                let cancel = Arc::new(AtomicBool::new(false));
                s.current = Some(Running { task: task.clone(), cancel: cancel.clone(), progress: 0.0 });
                return Some(Taken { task, cancel });
            }
            if exit_when_idle {
                return None;
            }
            s = self.changed.wait(s).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(crate) fn set_progress(&self, progress: f64) {
        if let Some(r) = &mut self.lock().current {
            r.progress = progress;
        }
    }

    /// Marks the running task done without taking another.
    pub(crate) fn finish(&self) {
        self.lock().current = None;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use bm_map::renderstate::TileUpdateStrategy::ForceNone;

    use super::*;

    #[test]
    fn dedups_and_merges_like_render_manager() {
        let q = RenderQueue::new();
        assert!(q.schedule(RenderTask::region("w", (0, 0))));
        assert!(!q.schedule(RenderTask::region("w", (0, 0))));
        assert!(q.schedule(RenderTask::region("w", (1, 0))));
        assert!(q.schedule(RenderTask::region("n", (0, 0))));
        let first = q.take(true).unwrap();
        assert_eq!(first.task.description(), "updating 2 regions of map 'w'");
        // a full update swallows queued region updates of its map and cancels a running one
        assert!(q.schedule(RenderTask::region("w", (5, 5))));
        let full = RenderTask::full("w", ForceNone);
        assert!(q.schedule_next(full.clone()));
        assert!(first.cancel.load(Ordering::Relaxed));
        assert_eq!(q.pending(), 2);
        assert_eq!(q.take(true).unwrap().task, full);
        assert_eq!(q.take(true).unwrap().task, RenderTask::region("n", (0, 0)));
        assert!(q.take(true).is_none());
        assert!(q.is_idle());
    }

    #[test]
    fn stop_cancels_and_wakes() {
        let q = Arc::new(RenderQueue::new());
        q.schedule(RenderTask::region("w", (0, 0)));
        let running = q.take(false).unwrap();
        let waiter = {
            let q = q.clone();
            std::thread::spawn(move || q.take(false).is_none())
        };
        q.stop();
        assert!(running.cancel.load(Ordering::Relaxed));
        assert!(waiter.join().unwrap());
        assert!(!q.schedule(RenderTask::region("w", (0, 0))));
    }
}
