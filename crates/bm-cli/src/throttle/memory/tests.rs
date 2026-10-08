use super::*;

const MIB: u64 = 1 << 20;

#[test]
fn threads_fit_the_limit() {
    assert_eq!(fit_threads(8, None), 8);
    assert_eq!(fit_threads(8, Some(BASE_BYTES + 2 * PER_THREAD_BYTES)), 2);
    assert_eq!(fit_threads(8, Some(BASE_BYTES + 2 * PER_THREAD_BYTES - 1)), 1);
    assert_eq!(fit_threads(2, Some(BASE_BYTES + 100 * PER_THREAD_BYTES)), 2, "never more than configured");
    assert_eq!(fit_threads(8, Some(BASE_BYTES / 2)), 1, "below the base: 1 thread plus the runtime guard");
}

#[test]
fn pauses_above_and_resumes_below_ninety_percent() {
    let q = RenderQueue::new();
    let mut guard = MemoryGuard::new(1000 * MIB);
    let t0 = Instant::now();
    guard.check(900 * MIB, &q, t0);
    assert!(!q.is_paused());
    guard.check(1001 * MIB, &q, t0);
    assert!(q.pause_reasons().contains(PauseReason::Memory));
    guard.check(950 * MIB, &q, t0 + Duration::from_secs(1));
    assert!(q.is_paused(), "hysteresis: still above 90%");
    guard.check(899 * MIB, &q, t0 + Duration::from_secs(2));
    assert!(!q.is_paused());
}

#[test]
fn gives_up_when_the_idle_core_stays_above_the_limit() {
    let q = RenderQueue::new();
    let mut guard = MemoryGuard::new(100 * MIB);
    let t0 = Instant::now();
    guard.check(200 * MIB, &q, t0);
    assert!(q.is_paused());
    guard.check(200 * MIB, &q, t0 + Duration::from_secs(1));
    guard.check(200 * MIB, &q, t0 + Duration::from_secs(60));
    assert!(q.is_paused(), "not yet 60 s since it went idle");
    guard.check(200 * MIB, &q, t0 + Duration::from_secs(61));
    assert!(!q.is_paused(), "degrades to warn-only");
    guard.check(500 * MIB, &q, t0 + Duration::from_secs(62));
    assert!(!q.is_paused());
}

#[test]
fn leaves_other_reasons_alone() {
    let q = RenderQueue::new();
    q.pause(PauseReason::Stopped);
    let mut guard = MemoryGuard::new(100 * MIB);
    guard.check(200 * MIB, &q, Instant::now());
    guard.check(10 * MIB, &q, Instant::now());
    let reasons: Vec<_> = q.pause_reasons().iter().collect();
    assert_eq!(reasons, [PauseReason::Stopped]);
}
