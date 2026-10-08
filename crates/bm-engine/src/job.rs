//! Render-queue work that isn't a map update, e.g. `StorageDeleteTask`: it runs on the worker ahead of queued
//! map updates, shows up as the running task (description, progress) and can be cancelled like one.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::queue::RenderQueue;

pub struct Job {
    pub(crate) key: String,
    pub(crate) description: String,
    pub(crate) work: Box<dyn FnOnce(&JobControl) + Send>,
}

impl Job {
    /// `key` identifies the work: a queued job with the same key makes a new one redundant (`contains`).
    pub fn new(
        key: impl Into<String>,
        description: impl Into<String>,
        work: impl FnOnce(&JobControl) + Send + 'static,
    ) -> Self {
        Self { key: key.into(), description: description.into(), work: Box::new(work) }
    }
}

/// What a running [`Job`] sees of the queue.
pub struct JobControl<'a> {
    pub(crate) queue: &'a RenderQueue,
    pub(crate) cancel: &'a AtomicBool,
}

impl JobControl<'_> {
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Progress (0..1) shown for the running task.
    pub fn set_progress(&self, progress: f64) {
        self.queue.set_progress(progress);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::channel;

    use super::*;
    use crate::queue::Next;
    use crate::task::RenderTask;

    #[test]
    fn jobs_run_first_dedup_by_key_and_report_progress() {
        let q = RenderQueue::new();
        q.schedule(RenderTask::region("w", (0, 0)));
        let (tx, rx) = channel();
        let tx2 = tx.clone();
        assert!(q.schedule_job(Job::new("delete:s/a", "deleting map 'a'", move |c| {
            c.set_progress(0.5);
            tx.send(c.is_cancelled()).unwrap();
        })));
        assert!(!q.schedule_job(Job::new("delete:s/a", "again", move |_| tx2.send(true).unwrap())));
        let Some(Next::Job(job, cancel)) = q.take(true) else { panic!("job first") };
        assert_eq!(q.current(), Some(("deleting map 'a'".to_owned(), 0.0)));
        assert_eq!(q.current_task(), None);
        (job.work)(&JobControl { queue: &q, cancel: &cancel });
        assert_eq!(q.current(), Some(("deleting map 'a'".to_owned(), 0.5)));
        assert!(!rx.recv().unwrap(), "not cancelled");
        assert!(matches!(q.take(true), Some(Next::Task(_))));
        assert!(q.take(true).is_none());
        assert!(rx.try_recv().is_err(), "the redundant job never ran");
    }

    #[test]
    fn stop_drops_and_cancels_jobs() {
        let q = Arc::new(RenderQueue::new());
        q.schedule_job(Job::new("a", "a", |_| {}));
        q.schedule_job(Job::new("b", "b", |_| {}));
        let Some(Next::Job(_, cancel)) = q.take(true) else { panic!("job") };
        q.stop();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(q.take(true).is_none());
        assert!(!q.schedule_job(Job::new("c", "c", |_| {})));
    }
}
