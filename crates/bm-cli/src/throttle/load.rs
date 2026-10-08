//! Server tick time (`ServerLoad` from the shim, docs/15): a 10 s rolling average of the reported MSPT with
//! hysteresis between `render-pause-mspt` and `render-resume-mspt`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(10);
/// One slow tick right after a (re)start must not pause rendering.
const MIN_SAMPLES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transition {
    /// The average MSPT that crossed `render-pause-mspt`.
    Pause(f64),
    Resume(f64),
}

#[derive(Default)]
pub struct LoadMonitor {
    samples: VecDeque<(Instant, f64)>,
    lagging: bool,
    announced: bool,
}

impl LoadMonitor {
    /// True the first time only: the shim reports load.
    pub fn first_report(&mut self) -> bool {
        !std::mem::replace(&mut self.announced, true)
    }

    pub fn is_lagging(&self) -> bool {
        self.lagging
    }

    pub fn average(&self) -> Option<f64> {
        let n = self.samples.len();
        (n > 0).then(|| self.samples.iter().map(|(_, v)| v).sum::<f64>() / n as f64)
    }

    /// Records one sample. `pause_at <= 0` disables pausing; a `resume_at` outside `(0, pause_at]` means `pause_at`.
    pub fn sample(&mut self, mspt: f64, now: Instant, pause_at: f64, resume_at: f64) -> Option<Transition> {
        if mspt.is_finite() && mspt >= 0.0 {
            self.samples.push_back((now, mspt));
        }
        while self.samples.front().is_some_and(|(t, _)| now.duration_since(*t) > WINDOW) {
            self.samples.pop_front();
        }
        let avg = self.average()?;
        let resume_at = if resume_at > 0.0 && resume_at <= pause_at { resume_at } else { pause_at };
        if self.lagging && (pause_at <= 0.0 || avg < resume_at) {
            self.lagging = false;
            return Some(Transition::Resume(avg));
        }
        if !self.lagging && pause_at > 0.0 && avg > pause_at && self.samples.len() >= MIN_SAMPLES {
            self.lagging = true;
            return Some(Transition::Pause(avg));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(m: &mut LoadMonitor, t0: Instant, from: u64, values: &[f64]) -> Vec<Transition> {
        let at = |i: usize| t0 + Duration::from_secs(from + i as u64);
        values.iter().enumerate().filter_map(|(i, v)| m.sample(*v, at(i), 45.0, 40.0)).collect()
    }

    #[test]
    fn pauses_on_the_average_with_hysteresis() {
        let (mut m, t0) = (LoadMonitor::default(), Instant::now());
        assert_eq!(feed(&mut m, t0, 0, &[200.0, 20.0, 20.0, 20.0]), [], "one spike is averaged away");
        assert_eq!(feed(&mut m, t0, 20, &[60.0; 4]), [], "fewer than 5 samples in the window");
        assert_eq!(feed(&mut m, t0, 24, &[60.0]), [Transition::Pause(60.0)]);
        assert!(m.is_lagging());
        assert_eq!(feed(&mut m, t0, 25, &[42.0; 10]), [], "between the thresholds: still paused");
        let resumed = feed(&mut m, t0, 35, &[30.0; 10]);
        assert!(matches!(resumed.as_slice(), [Transition::Resume(avg)] if *avg < 40.0), "{resumed:?}");
        assert!(!m.is_lagging());
    }

    #[test]
    fn zero_disables_and_releases() {
        let (mut m, t0) = (LoadMonitor::default(), Instant::now());
        feed(&mut m, t0, 0, &[100.0; 5]);
        assert!(m.is_lagging());
        assert!(matches!(m.sample(100.0, t0 + Duration::from_secs(5), 0.0, 0.0), Some(Transition::Resume(_))));
        assert_eq!(m.sample(100.0, t0 + Duration::from_secs(6), 0.0, 0.0), None);
        assert!(!m.is_lagging());
    }
}
