//! A counting semaphore for blocking-pool threads, optionally with a bounded wait (`server-unpack` slots, docs/18).

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub(crate) struct Gate {
    state: Mutex<State>,
    freed: Condvar,
    limit: usize,
    /// Most callers [`Gate::acquire_bounded`] lets wait at once.
    queue: usize,
    wait: Duration,
}

#[derive(Default)]
struct State {
    busy: usize,
    waiting: usize,
}

pub(crate) struct Permit<'a>(&'a Gate);

impl Gate {
    /// `limit` permits; [`Gate::acquire`] waits as long as it takes.
    pub(crate) fn new(limit: usize) -> Self {
        Self::with_queue(limit, usize::MAX, Duration::MAX)
    }

    /// `limit` permits; [`Gate::acquire_bounded`] lets up to `queue` callers wait up to `wait` each.
    pub(crate) fn with_queue(limit: usize, queue: usize, wait: Duration) -> Self {
        Self { state: Mutex::default(), freed: Condvar::new(), limit, queue, wait }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn acquire(&self) -> Permit<'_> {
        let mut state = self.lock();
        while state.busy >= self.limit {
            state = self.freed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state.busy += 1;
        Permit(self)
    }

    /// `None` at once when the queue is full, or once the wait runs out.
    pub(crate) fn acquire_bounded(&self) -> Option<Permit<'_>> {
        let mut state = self.lock();
        if state.busy >= self.limit {
            if state.waiting >= self.queue {
                return None;
            }
            state.waiting += 1;
            let deadline = Instant::now().checked_add(self.wait);
            while state.busy >= self.limit {
                let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
                if left == Some(Duration::ZERO) {
                    break;
                }
                state = match left {
                    Some(left) => self.freed.wait_timeout(state, left).unwrap_or_else(|e| e.into_inner()).0,
                    None => self.freed.wait(state).unwrap_or_else(|e| e.into_inner()),
                };
            }
            state.waiting -= 1;
            if state.busy >= self.limit {
                return None;
            }
        }
        state.busy += 1;
        Some(Permit(self))
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.lock().busy -= 1;
        self.0.freed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_waits_queue_then_give_up() {
        let gate = Gate::with_queue(1, 1, Duration::from_millis(50));
        let held = gate.acquire_bounded().expect("free");
        std::thread::scope(|s| {
            let waiter = s.spawn(|| gate.acquire_bounded().is_some());
            while gate.lock().waiting == 0 {
                std::thread::yield_now();
            }
            assert!(gate.acquire_bounded().is_none(), "queue full: refused at once");
            drop(held);
            assert!(waiter.join().unwrap(), "a freed permit goes to the waiter");
        });
        let _held = gate.acquire_bounded().unwrap();
        let t = Instant::now();
        assert!(gate.acquire_bounded().is_none());
        assert!(t.elapsed() >= Duration::from_millis(50), "waited out the deadline");
        assert!(Gate::with_queue(0, 0, Duration::from_secs(9)).acquire_bounded().is_none());
    }
}
