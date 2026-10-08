use bm_engine::Regions;
use bm_engine::TileUpdateStrategy::{ForceAll, ForceNone};

use super::*;

fn lister(_: &str) -> Option<Vec<(i32, i32)>> {
    Some(vec![(0, 0), (1, 0), (2, 0)])
}

/// A `-f` run on `maps` that a shutdown cuts short with `done` regions of the first map finished.
fn interrupted(data: &Path, maps: &[&str], done: &[(i32, i32)]) {
    let mut plan = RenderPlan::new(ForceAll, data, false);
    let queue = RenderQueue::new();
    for (i, map) in maps.iter().enumerate() {
        for mut task in plan.tasks(map) {
            if i == 0 {
                task.done.extend(done);
            }
            queue.schedule(task);
        }
    }
    queue.schedule(RenderTask::region("w", (9, 9))); // a watcher update: not resumed
    queue.stop();
    plan.save(&queue, &lister);
}

#[test]
fn an_interrupted_forced_render_resumes_once() {
    let dir = tempfile::tempdir().unwrap();
    interrupted(dir.path(), &["w", "n"], &[(0, 0)]);

    let mut plan = RenderPlan::new(ForceAll, dir.path(), false);
    let only = |r: &[(i32, i32)]| Regions::Only(r.iter().copied().collect());
    assert_eq!(plan.tasks("w"), [RenderTask::new("w", only(&[(1, 0), (2, 0)]), ForceAll)]);
    assert_eq!(plan.tasks("n"), [RenderTask::new("n", only(&[(0, 0), (1, 0), (2, 0)]), ForceAll)]);
    plan.save(&RenderQueue::new(), &lister);
    assert!(!tasks_dat::file(dir.path()).exists(), "a completed run leaves nothing behind");
}

#[test]
fn restart_and_other_strategies_start_fresh_but_keep_other_maps() {
    let dir = tempfile::tempdir().unwrap();
    interrupted(dir.path(), &["w", "n"], &[(0, 0)]);

    let mut plan = RenderPlan::new(ForceAll, dir.path(), true);
    assert_eq!(plan.tasks("w"), [RenderTask::full("w", ForceAll)]);
    plan.save(&RenderQueue::new(), &lister);
    let left = RenderPlan::new(ForceAll, dir.path(), false).tasks("n");
    assert_eq!(left.len(), 1, "map 'n' wasn't rendered, so its progress is kept");
    assert!(matches!(left[0].regions, Regions::Only(_)));

    let mut plain = RenderPlan::new(ForceNone, dir.path(), false);
    assert_eq!(plain.tasks("n"), [RenderTask::full("n", ForceNone)]);
    plain.save(&RenderQueue::new(), &lister);
    assert!(tasks_dat::file(dir.path()).exists(), "a plain -r leaves tasks.dat alone");
}
