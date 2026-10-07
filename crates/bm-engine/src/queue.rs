//! `RenderManager`'s task list: one running task plus a deduplicated queue. Producers (file watchers, timers,
//! commands) schedule from any thread; [`crate::run_queue`] works it off. [`Job`]s run ahead of queued tasks.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::job::Job;
use crate::task::{Regions, RenderTask};

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
    paused: bool,
    /// Tasks taken so far; numbers the running one for [`RenderQueue::current_run`].
    runs: u64,
}

struct Running {
    /// `None` while a [`Job`] runs.
    task: Option<RenderTask>,
    description: String,
    cancel: Arc<AtomicBool>,
    progress: f64,
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
        s.pending.clear();
        s.jobs.clear();
        if let Some(running) = &s.current {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.changed.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    /// `RenderManager.stop()` of a plugin (`/bluemap stop`, player render limit): the running task is cancelled
    /// and queued again in front, and nothing is taken until [`RenderQueue::resume`]. Finished regions stay done.
    /// A running job finishes.
    pub fn pause(&self) {
        let mut s = self.lock();
        if s.paused {
            return;
        }
        s.paused = true;
        if let Some(running) = &s.current
            && let Some(task) = running.task.clone()
        {
            running.cancel.store(true, Ordering::Relaxed);
            if !s.pending.iter().any(|t| t.contains(&task)) {
                s.pending.push_front(task);
            }
        }
        self.changed.notify_all();
    }

    pub fn resume(&self) {
        self.lock().paused = false;
        self.changed.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
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

    pub fn pending(&self) -> usize {
        let s = self.lock();
        s.pending.len() + s.jobs.len()
    }

    pub fn is_idle(&self) -> bool {
        let s = self.lock();
        s.current.is_none() && s.pending.is_empty() && s.jobs.is_empty()
    }

    /// Next job, else next task merged with every queued region update of the same map and strategy (one pass
    /// over shared tiles instead of one per region). Blocks while the queue is empty, unless `exit_when_idle`;
    /// `None` once stopped (or idle with `exit_when_idle`).
    pub(crate) fn take(&self, exit_when_idle: bool) -> Option<Next> {
        let mut s = self.lock();
        s.current = None;
        self.changed.notify_all();
        loop {
            if s.stopped {
                return None;
            }
            if !s.paused {
                let cancel = Arc::new(AtomicBool::new(false));
                if let Some(job) = s.jobs.pop_front() {
                    s.runs += 1;
                    s.current = Some(Running {
                        task: None,
                        description: job.description.clone(),
                        cancel: cancel.clone(),
                        progress: 0.0,
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
                    });
                    return Some(Next::Task(Taken { task, cancel }));
                }
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
mod tests;
