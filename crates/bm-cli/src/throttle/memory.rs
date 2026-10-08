//! `memory-limit` (docs/15): fewer render threads at load so the estimated peak fits, and a 1 Hz guard that
//! pauses the render queue while the core's RSS is above the limit.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use bm_engine::{PauseReason, RenderQueue, RenderTask, TileUpdateStrategy};

use crate::log;

/// Measured peak RSS on fixture `structures`, rounded up (docs/15 §memory: command and numbers).
pub const BASE_BYTES: u64 = 160 << 20;
pub const PER_THREAD_BYTES: u64 = 24 << 20;

pub const CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Paused, nothing running and still above the limit this long: the limit is below the idle footprint.
const IDLE_GIVE_UP: Duration = Duration::from_secs(60);

/// `min(resolved, max(1, (limit - base) / per_thread))`; at least 1 thread even when the limit is below the base.
pub fn fit_threads(resolved: usize, limit: Option<u64>) -> usize {
    let Some(limit) = limit else { return resolved };
    let fitting = limit.saturating_sub(BASE_BYTES) / PER_THREAD_BYTES.max(1);
    resolved.min((fitting as usize).max(1))
}

pub fn mib(bytes: u64) -> u64 {
    bytes >> 20
}

/// The pause/resume state machine; [`MemoryGuard::check`] is fed one RSS sample per [`CHECK_INTERVAL`].
pub struct MemoryGuard {
    limit: u64,
    paused: bool,
    /// Since when the queue sits paused with nothing running while RSS stays above the limit.
    idle_over_since: Option<Instant>,
    warn_only: bool,
    /// A forced task restarts from scratch when paused; one interrupted twice is let finish instead.
    forced_interrupted: Option<RenderTask>,
    forced_warned: bool,
}

impl MemoryGuard {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            paused: false,
            idle_over_since: None,
            warn_only: false,
            forced_interrupted: None,
            forced_warned: false,
        }
    }

    pub fn check(&mut self, rss: u64, queue: &RenderQueue, now: Instant) {
        if self.warn_only {
            return;
        }
        let (rss_mib, limit_mib) = (mib(rss), mib(self.limit));
        if !self.paused {
            if rss <= self.limit {
                return;
            }
            let forced = queue.current_task().filter(|t| t.strategy == TileUpdateStrategy::ForceAll);
            if forced.is_some() && forced == self.forced_interrupted {
                if !self.forced_warned {
                    self.forced_warned = true;
                    log::warn(&format!(
                        "Core memory ({rss_mib} MiB) is above the memory-limit ({limit_mib} MiB) again; letting the \
                         forced render finish instead of restarting it"
                    ));
                }
                return;
            }
            self.forced_interrupted = forced;
            self.forced_warned = false;
            self.paused = true;
            self.idle_over_since = None;
            queue.pause(PauseReason::Memory);
            log::warn(&format!(
                "Core memory ({rss_mib} MiB) is above the memory-limit ({limit_mib} MiB): pausing rendering"
            ));
            return;
        }
        if rss < self.limit / 10 * 9 {
            self.paused = false;
            queue.resume(PauseReason::Memory);
            log::info(&format!("Core memory is down to {rss_mib} MiB: resuming rendering"));
            return;
        }
        if queue.current().is_some() {
            self.idle_over_since = None;
            return;
        }
        let since = *self.idle_over_since.get_or_insert(now);
        if now.duration_since(since) >= IDLE_GIVE_UP {
            self.warn_only = true;
            self.paused = false;
            queue.resume(PauseReason::Memory);
            log::warn(&format!(
                "The memory-limit ({limit_mib} MiB) is below what the core needs while idle ({rss_mib} MiB); resuming \
                 rendering and ignoring the limit until the next start"
            ));
        }
    }
}

/// The CLI's guard thread: samples RSS until `stop`'s sender is dropped.
pub fn monitor(queue: &RenderQueue, limit: u64, stop: Receiver<()>) {
    let mut guard = MemoryGuard::new(limit);
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(CHECK_INTERVAL) {
        if let Some(rss) = bm_ipc::resident_memory() {
            guard.check(rss, queue, Instant::now());
        }
    }
}

#[cfg(test)]
mod tests;
