use bm_map::renderstate::TileUpdateStrategy::ForceNone;

use super::*;

fn task(next: Option<Next>) -> Option<Taken> {
    next.map(Next::into_task)
}

#[test]
fn dedups_and_merges_like_render_manager() {
    let q = RenderQueue::new();
    assert!(q.schedule(RenderTask::region("w", (0, 0))));
    assert!(!q.schedule(RenderTask::region("w", (0, 0))));
    assert!(q.schedule(RenderTask::region("w", (1, 0))));
    assert!(q.schedule(RenderTask::region("n", (0, 0))));
    let first = task(q.take(true)).unwrap();
    assert_eq!(first.task.description(), "updating 2 regions of map 'w'");
    // a full update swallows queued region updates of its map and cancels a running one
    assert!(q.schedule(RenderTask::region("w", (5, 5))));
    let full = RenderTask::full("w", ForceNone);
    assert!(q.schedule_next(full.clone()));
    assert!(first.cancel.load(Ordering::Relaxed));
    assert_eq!(q.pending(), 2);
    assert_eq!(task(q.take(true)).unwrap().task, full);
    assert_eq!(task(q.take(true)).unwrap().task, RenderTask::region("n", (0, 0)));
    assert!(q.take(true).is_none());
    assert!(q.is_idle());
}

#[test]
fn pause_requeues_running_task_and_blocks_takes() {
    let q = Arc::new(RenderQueue::new());
    q.schedule(RenderTask::region("w", (0, 0)));
    q.schedule(RenderTask::region("n", (0, 0)));
    let running = task(q.take(false)).unwrap();
    q.pause(PauseReason::Stopped);
    assert!(running.cancel.load(Ordering::Relaxed));
    assert_eq!(q.pending_tasks(), vec![RenderTask::region("w", (0, 0)), RenderTask::region("n", (0, 0))]);
    let waiter = {
        let q = q.clone();
        std::thread::spawn(move || task(q.take(false)).map(|t| t.task))
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!waiter.is_finished());
    q.resume(PauseReason::Stopped);
    assert_eq!(waiter.join().unwrap(), Some(RenderTask::region("w", (0, 0))));
    assert_eq!(q.remove_where(|t| t.map == "n"), 1);
    assert!(q.pending_tasks().is_empty());
}

#[test]
fn reasons_pause_and_resume_independently() {
    let q = RenderQueue::new();
    q.schedule(RenderTask::region("w", (0, 0)));
    q.schedule(RenderTask::region("n", (0, 0)));
    let first = task(q.take(true)).unwrap();
    q.pause(PauseReason::Memory);
    assert!(first.cancel.load(Ordering::Relaxed));
    assert_eq!(q.pending(), 2, "the running task is queued again once");

    // a second reason doesn't requeue again
    q.pause(PauseReason::ServerLoad);
    q.pause(PauseReason::Memory);
    assert_eq!(q.pending(), 2);
    let reasons: Vec<_> = q.pause_reasons().iter().collect();
    assert_eq!(reasons, [PauseReason::Memory, PauseReason::ServerLoad]);

    q.resume(PauseReason::Memory);
    assert!(q.is_paused());
    q.resume(PauseReason::Stopped);
    assert!(q.is_paused(), "resuming an absent reason changes nothing");
    q.resume(PauseReason::ServerLoad);
    assert!(!q.is_paused() && q.pause_reasons().is_empty());
    let next = task(q.take(true)).unwrap();
    assert_eq!(next.task, RenderTask::region("w", (0, 0)));
    assert!(!next.cancel.load(Ordering::Relaxed));
}

#[test]
fn exit_when_idle_waits_out_a_pause() {
    let q = Arc::new(RenderQueue::new());
    q.pause(PauseReason::Memory);
    assert!(q.take(true).is_none(), "nothing queued: idle even while paused");
    assert!(q.schedule(RenderTask::region("w", (0, 0))), "scheduling still works while paused");
    let waiter = {
        let q = q.clone();
        std::thread::spawn(move || task(q.take(true)).map(|t| t.task))
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!waiter.is_finished(), "a paused queue with work left blocks instead of ending the run");
    q.resume(PauseReason::Memory);
    assert_eq!(waiter.join().unwrap(), Some(RenderTask::region("w", (0, 0))));
}

#[test]
fn stop_cancels_and_wakes() {
    let q = Arc::new(RenderQueue::new());
    q.schedule(RenderTask::region("w", (0, 0)));
    let running = task(q.take(false)).unwrap();
    let waiter = {
        let q = q.clone();
        std::thread::spawn(move || q.take(false).is_none())
    };
    q.stop();
    assert!(running.cancel.load(Ordering::Relaxed));
    assert!(waiter.join().unwrap());
    assert!(!q.schedule(RenderTask::region("w", (0, 0))));
}

#[test]
fn runs_number_each_taken_task() {
    let q = RenderQueue::new();
    assert_eq!(q.current_run(), None);
    q.schedule(RenderTask::region("w", (0, 0)));
    q.schedule(RenderTask::region("n", (0, 0)));
    task(q.take(true));
    q.set_progress(0.25);
    assert_eq!(q.current_run(), Some((1, 0.25)));
    task(q.take(true));
    assert_eq!(q.current_run(), Some((2, 0.0)));
}
