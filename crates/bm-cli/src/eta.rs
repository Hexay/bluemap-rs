//! Remaining-time estimate of the running render task (`ProgressTracker` +
//! `RenderManager.estimateCurrentRenderTaskTimeRemaining`) and `TextFormat.duration` to print it.

use std::collections::VecDeque;
use std::time::Duration;

/// `new ProgressTracker(5000, 12)`: a sample every 5 s, averaged over the last minute.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
pub const AVERAGING: usize = 12;

#[derive(Default)]
pub struct ProgressTracker {
    /// Which task the samples belong to ([`bm_engine::RenderQueue::current_run`]).
    run: Option<u64>,
    last_time: i64,
    last_progress: f64,
    /// Milliseconds a whole task would take at each sample's pace.
    times_per_progress: VecDeque<i64>,
}

impl ProgressTracker {
    /// `update()`; a task not seen before restarts the tracker (`resetAndStart`).
    pub fn sample(&mut self, current: Option<(u64, f64)>, now_ms: i64) {
        let Some((run, progress)) = current else { return };
        if self.run != Some(run) {
            *self = Self { run: Some(run), last_time: now_ms, last_progress: progress, ..Self::default() };
            return;
        }
        let delta_progress = progress - self.last_progress;
        if delta_progress != 0.0 {
            self.times_per_progress.push_back(((now_ms - self.last_time) as f64 / delta_progress) as i64);
            while self.times_per_progress.len() > AVERAGING {
                self.times_per_progress.pop_front();
            }
            self.last_time = now_ms;
            self.last_progress = progress;
        }
    }

    /// `lastProgress` and `timesPerProgress` (`debug dump`).
    pub fn samples(&self) -> (f64, Vec<i64>) {
        (self.last_progress, self.times_per_progress.iter().copied().collect())
    }

    /// Whether the samples belong to task `run`.
    pub fn tracks(&self, run: u64) -> bool {
        self.run == Some(run)
    }

    /// `estimateCurrentRenderTaskTimeRemaining` for the running task; 0 before its first sample.
    pub fn remaining_of(&self, (run, progress): (u64, f64)) -> i64 {
        if self.tracks(run) { self.remaining_ms(progress) } else { 0 }
    }

    /// `estimateCurrentRenderTaskTimeRemaining` for a task at `progress`; 0 without samples.
    pub fn remaining_ms(&self, progress: f64) -> i64 {
        let n = self.times_per_progress.len();
        // Collectors.averagingLong(...).longValue()
        let average = if n == 0 { 0 } else { (self.times_per_progress.iter().sum::<i64>() as f64 / n as f64) as i64 };
        ((1.0 - progress) * average as f64) as i64
    }
}

/// `TextFormat.duration`: the largest unit above 1 (days … seconds), one decimal below 2.
pub fn duration(millis: i64) -> String {
    let units = [("days", 86_400_000i64), ("hours", 3_600_000), ("minutes", 60_000), ("seconds", 1000)];
    let (mut name, mut value) = ("seconds", 0.0);
    for (unit, ms) in units {
        (name, value) = (unit, millis as f64 / ms as f64);
        if value > 1.0 {
            break;
        }
    }
    // Java's %.Nf rounds half up; Rust's formatter rounds half to even
    if value < 2.0 && name != "seconds" {
        format!("{:.1} {name}", (value * 10.0).round() / 10.0)
    } else {
        format!("{:.0} {name}", value.round())
    }
}

/// `StatusCommand.taskETA`: the `remaining time` value, none without an estimate or below 0.1 % progress.
pub fn status_remaining(remaining_ms: i64, progress: f64) -> Option<String> {
    (remaining_ms != 0 && progress >= 0.001).then(|| duration(remaining_ms))
}

/// BlueMapCLI's `" (ETA: %s)"` suffix, empty when there is no estimate.
pub fn suffix(remaining_ms: i64) -> String {
    if remaining_ms > 0 { format!(" (ETA: {})", duration(remaining_ms)) } else { String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_the_pace_of_the_current_task() {
        let mut t = ProgressTracker::default();
        assert_eq!(t.remaining_ms(0.0), 0);
        t.sample(Some((1, 0.0)), 0);
        t.sample(Some((1, 0.1)), 5_000);
        t.sample(Some((1, 0.1)), 10_000); // no progress: no sample
        t.sample(Some((1, 0.2)), 20_000);
        // 50 s and 150 s per whole task
        assert_eq!(t.remaining_ms(0.2), 80_000);
        t.sample(None, 25_000);
        assert_eq!(t.remaining_ms(0.5), 50_000, "idle keeps the samples");
        assert_eq!(t.remaining_of((1, 0.5)), 50_000);
        assert_eq!(t.remaining_of((2, 0.5)), 0, "not yet sampled: no estimate");
        t.sample(Some((2, 0.0)), 30_000);
        assert_eq!(t.remaining_ms(0.0), 0, "a new task starts over");
    }

    #[test]
    fn keeps_one_minute_of_samples() {
        let mut t = ProgressTracker::default();
        let step = 1.0 / 1024.0;
        t.sample(Some((1, 0.0)), 0);
        for i in 1..=12 {
            t.sample(Some((1, f64::from(i) * step)), i64::from(i) * 5_000);
        }
        assert_eq!(t.remaining_ms(0.0), 5_120_000);
        for i in 13..=24 {
            t.sample(Some((1, f64::from(2 * i - 12) * step)), i64::from(i) * 5_000);
        }
        assert_eq!(t.remaining_ms(0.0), 2_560_000, "only the last 12 samples count");
    }

    #[test]
    fn formats_like_text_format() {
        assert_eq!(duration(500), "1 seconds");
        assert_eq!(duration(1_500), "2 seconds");
        assert_eq!(duration(59_000), "59 seconds");
        assert_eq!(duration(90_000), "1.5 minutes");
        assert_eq!(duration(150_000), "3 minutes");
        assert_eq!(duration(3_600_000), "60 minutes");
        assert_eq!(duration(5_400_000), "1.5 hours");
        assert_eq!(duration(3 * 86_400_000), "3 days");
        assert_eq!(status_remaining(90_000, 0.5).as_deref(), Some("1.5 minutes"));
        assert_eq!(status_remaining(0, 0.5), None);
        assert_eq!(status_remaining(90_000, 0.000_9), None);
        assert_eq!(suffix(0), "");
        assert_eq!(suffix(-5), "");
        assert_eq!(suffix(90_000), " (ETA: 1.5 minutes)");
    }
}
