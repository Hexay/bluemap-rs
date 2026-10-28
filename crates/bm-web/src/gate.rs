//! A counting semaphore for blocking-pool threads (the transcode memory gate).

use std::sync::{Condvar, Mutex, MutexGuard};

pub(crate) struct Gate {
    busy: Mutex<usize>,
    freed: Condvar,
    limit: usize,
}

pub(crate) struct Permit<'a>(&'a Gate);

impl Gate {
    pub(crate) fn new(limit: usize) -> Self {
        Self { busy: Mutex::new(0), freed: Condvar::new(), limit }
    }

    fn lock(&self) -> MutexGuard<'_, usize> {
        self.busy.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn acquire(&self) -> Permit<'_> {
        let mut busy = self.lock();
        while *busy >= self.limit {
            busy = self.freed.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        *busy += 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.lock() -= 1;
        self.0.freed.notify_one();
    }
}
