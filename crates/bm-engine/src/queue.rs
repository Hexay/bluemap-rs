//! `RenderManager`'s task list: one running task plus a deduplicated queue. Producers (file watchers, timers,
//! commands) schedule from any thread; [`crate::run_queue`] works it off. [`Job`]s run ahead of queued tasks.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use bm_format::grid::Tile;

use crate::job::Job;
use crate::task::{Regions, RenderTask};

mod pause;
pub use pause::{PauseReason, PauseReasons};

#[derive(Default)]
pub struct RenderQueue {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    pending: VecDeque<RenderTask>,
    jobs: VecDeque<Job>,
    current: Option<Running>,
    stopped: bool,
    paused: PauseReasons,
    /// Tasks taken so far; numbers the running one for [`RenderQueue::current_run`].
    runs: u64,
    /// What [`RenderQueue::stop`] interrupted: the running task (with its done regions) first, then the queue.
    abandoned: Vec<RenderTask>,
    /// When the last task or job ended (`RenderManager.getLastTimeBusy`).
    last_busy: Option<Instant>,
    /// Descriptions of the last finished (or cancelled) tasks and jobs, oldest first (`getCompletedTasks`).
    completed: VecDeque<String>,
}

/// `RenderManager.completedTasks` keeps the last 10.
const COMPLETED_KEPT: usize = 10;

struct Running {
    /// `None` while a [`Job`] runs.
    task: Option<RenderTask>,
    description: String,
    cancel: Arc<AtomicBool>,
    progress: f64,
    /// Paused: queue the task again in front once it has wound down (with every region it finished).
    requeue: bool,
}

/// What [`RenderQueue::take`] hands the worker.
pub(crate) struct Taken {
    pub task: RenderTask,
    pub cancel: Arc<AtomicBool>,
}

pub(crate) enum Next {
    Task(Taken),
    Job(Job, Arc<AtomicBool>),
}

#[cfg(test)]
impl Next {
    pub(crate) fn into_task(self) -> Taken {
        match self {
            Next::Task(t) => t,
            Next::Job(..) => panic!("expected a task, got a job"),
        }
    }
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

    /// `scheduleRenderTaskNext` of a non-update task: runs after the queued jobs, before any queued task. False
    /// when a queued job has the same key or the queue is stopped.
    pub fn schedule_job(&self, job: Job) -> bool {
        let mut s = self.lock();
        if s.stopped || s.jobs.iter().any(|j| j.key == job.key) {
            return false;
        }
        s.jobs.push_back(job);
        self.changed.notify_all();
        true
    }

    fn insert(&self, task: RenderTask, front: bool) -> bool {
        let mut s = self.lock();
        if s.stopped || s.pending.iter().any(|t| t.contains(&task)) {
            return false;
        }
        s.pending.retain(|t| !task.contains(t));
        if let Some(running) = &s.current
            && running.task.as_ref().is_some_and(|t| task.contains(t))
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
        let pending: Vec<RenderTask> = s.pending.drain(..).collect();
        s.abandoned.extend(pending);
        s.jobs.clear();
        if let Some(running) = &s.current {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.changed.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    /// After [`RenderQueue::stop`] and the worker's return: the tasks it cut short, to persist for the next run.
    pub fn abandoned_tasks(&self) -> Vec<RenderTask> {
        self.lock().abandoned.clone()
    }

    /// `RenderManager.stop()` of a plugin, for `reason`: nothing is taken until every reason is resumed. The first
    /// reason cancels the running task and queues it again in front, minus its finished regions. A running job
    /// finishes.
    pub fn pause(&self, reason: PauseReason) {
        let mut s = self.lock();
        let first = s.paused.is_empty();
        s.paused.insert(reason);
        if !first {
            return;
        }
        if let Some(running) = &mut s.current
            && running.task.is_some()
        {
            running.cancel.store(true, Ordering::Relaxed);
            running.requeue = true;
        }
        self.changed.notify_all();
    }

    /// Drops `reason`; the queue runs again once no reason is left.
    pub fn resume(&self, reason: PauseReason) {
        self.lock().paused.remove(reason);
        self.changed.notify_all();
    }

    pub fn pause_reasons(&self) -> PauseReasons {
        self.lock().paused
    }

    pub fn is_paused(&self) -> bool {
        !self.lock().paused.is_empty()
    }

    /// The queued tasks (without the running one and jobs), in order.
    pub fn pending_tasks(&self) -> Vec<RenderTask> {
        self.lock().pending.iter().cloned().collect()
    }

    /// The running task, if any (not a job).
    pub fn current_task(&self) -> Option<RenderTask> {
        self.lock().current.as_ref().and_then(|r| r.task.clone())
    }

    /// Drops queued tasks matching `filter` and cancels a matching running one; returns how many were affected.
    pub fn remove_where(&self, filter: impl Fn(&RenderTask) -> bool) -> usize {
        let mut s = self.lock();
        let before = s.pending.len();
        s.pending.retain(|t| !filter(t));
        let mut removed = before - s.pending.len();
        if let Some(running) = &s.current
            && running.task.as_ref().is_some_and(&filter)
        {
            running.cancel.store(true, Ordering::Relaxed);
            removed += 1;
        }
        self.changed.notify_all();
        removed
    }

    /// The running task's (or job's) description and estimated progress (0..1).
    pub fn current(&self) -> Option<(String, f64)> {
        self.lock().current.as_ref().map(|r| (r.description.clone(), r.progress))
    }

    /// The running task's sequence number (new for every task taken, so an ETA tracker can tell tasks apart)
    /// and its estimated progress.
    pub fn current_run(&self) -> Option<(u64, f64)> {
        let s = self.lock();
        s.current.as_ref().map(|r| (s.runs, r.progress))
    }

    /// Descriptions of the last finished tasks and jobs, oldest first.
    pub fn completed(&self) -> Vec<String> {
        self.lock().completed.iter().cloned().collect()
    }

    /// When a task or job last ended; `None` if none ever ran.
    pub fn last_busy(&self) -> Option<Instant> {
        self.lock().last_busy
    }

    pub fn pending(&self) -> usize {
        let s = self.lock();
        s.pending.len() + s.jobs.len()
    }

    pub fn is_idle(&self) -> bool {
        let s = self.lock();
        s.current.is_none() && s.pending.is_empty() && s.jobs.is_empty()
    }

    /// Next job, else next task merged with every queued region update of the same map and strategy (one pass
    /// over shared tiles instead of one per region). Blocks while the queue is empty or paused, unless
    /// `exit_when_idle` and nothing is queued; `None` once stopped (or idle with `exit_when_idle`).
    pub(crate) fn take(&self, exit_when_idle: bool) -> Option<Next> {
        let mut s = self.lock();
        retire_current(&mut s);
        self.changed.notify_all();
        loop {
            if s.stopped {
                return None;
            }
            if s.paused.is_empty() {
                let cancel = Arc::new(AtomicBool::new(false));
                if let Some(job) = s.jobs.pop_front() {
                    s.runs += 1;
                    s.current = Some(Running {
                        task: None,
                        description: job.description.clone(),
                        cancel: cancel.clone(),
                        progress: 0.0,
                        requeue: false,
                    });
                    return Some(Next::Job(job, cancel));
                }
                if let Some(mut task) = s.pending.pop_front() {
                    if matches!(task.regions, Regions::Only(_)) {
                        s.pending.retain(|t| !task.absorb(t));
                    }
                    s.runs += 1;
                    s.current = Some(Running {
                        task: Some(task.clone()),
                        description: task.description(),
                        cancel: cancel.clone(),
                        progress: 0.0,
                        requeue: false,
                    });
                    return Some(Next::Task(Taken { task, cancel }));
                }
            }
            // a paused queue with work left isn't idle: wait for the resume
            if exit_when_idle && s.pending.is_empty() && s.jobs.is_empty() {
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

    /// Records a finished region of the running task, so a pause or stop requeues only the rest.
    pub(crate) fn region_done(&self, region: Tile) {
        if let Some(task) = self.lock().current.as_mut().and_then(|r| r.task.as_mut()) {
            task.done.insert(region);
        }
    }

    /// Marks the running task done without taking another.
    pub(crate) fn finish(&self) {
        retire_current(&mut self.lock());
        self.changed.notify_all();
    }
}

fn retire_current(s: &mut State) {
    let Some(running) = s.current.take() else { return };
    s.last_busy = Some(Instant::now());
    match running.task {
        Some(task) if s.stopped => s.abandoned.insert(0, task),
        Some(task) if running.requeue => {
            if !s.pending.iter().any(|t| t.contains(&task)) {
                s.pending.push_front(task);
            }
        }
        _ => {
            if s.completed.len() == COMPLETED_KEPT {
                s.completed.pop_front();
            }
            s.completed.push_back(running.description);
        }
    }
}

#[cfg(test)]
mod tests;
