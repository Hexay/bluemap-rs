use std::time::{Duration, Instant};

use super::*;
use crate::task::Regions;

fn settings() -> WatchSettings {
    WatchSettings {
        debounce: Duration::from_millis(100),
        update_cooldown: Duration::ZERO,
        full_update_interval: Duration::ZERO,
        scan_interval: Duration::ZERO,
    }
}

fn quiet() -> LogFn {
    Arc::new(|_, _| {})
}

/// The next task scheduled within `timeout`.
fn next_task(queue: &RenderQueue, timeout: Duration) -> Option<RenderTask> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if let Some(t) = queue.take(true) {
            return Some(t.task);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

fn regions(task: &RenderTask) -> Vec<Tile> {
    match &task.regions {
        Regions::Only(r) => r.iter().copied().collect(),
        Regions::All => panic!("expected a region update, got {task:?}"),
    }
}

#[test]
fn changed_region_files_are_debounced_into_one_update() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("r.0.0.mca"), b"0").unwrap();
    let queue = Arc::new(RenderQueue::new());
    let service = MapUpdateService::start("w", dir.path().to_owned(), queue.clone(), settings(), quiet()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(queue.take(true).is_none(), "existing files are covered by the initial full update");
    for i in 0..5 {
        std::fs::write(dir.path().join("r.0.-1.mca"), [i]).unwrap();
        std::fs::write(dir.path().join("level.dat"), [i]).unwrap();
    }
    let task = next_task(&queue, Duration::from_secs(10)).expect("region update");
    assert_eq!(regions(&task), vec![(0, -1)]);
    std::thread::sleep(Duration::from_millis(300));
    assert!(queue.take(true).is_none(), "one update per burst of changes");
    service.close();
}

#[test]
fn region_folder_created_later_is_picked_up() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("region");
    let queue = Arc::new(RenderQueue::new());
    let service = MapUpdateService::start("w", dir.clone(), queue.clone(), settings(), quiet()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("r.3.4.mca"), b"0").unwrap();
    let task = next_task(&queue, RETRY_INTERVAL * 3).expect("region update after the folder appeared");
    assert_eq!(regions(&task), vec![(3, 4)]);
    service.close();
}

#[test]
fn full_updates_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let queue = Arc::new(RenderQueue::new());
    let s = WatchSettings { full_update_interval: Duration::from_millis(400), ..settings() };
    let service = MapUpdateService::start("w", dir.path().to_owned(), queue.clone(), s, quiet()).unwrap();
    let task = next_task(&queue, Duration::from_secs(5)).expect("full update");
    assert_eq!(task, RenderTask::full("w", TileUpdateStrategy::ForceNone));
    let again = next_task(&queue, Duration::from_secs(5)).expect("second full update");
    assert_eq!(again.regions, Regions::All);
    service.close();
}

#[test]
fn periodic_rescan_catches_changes_the_watcher_missed() {
    let dir = tempfile::tempdir().unwrap();
    let queue = Arc::new(RenderQueue::new());
    let s = WatchSettings { scan_interval: Duration::from_millis(200), ..settings() };
    let mut service = Service {
        map: "w".into(),
        dir: dir.path().to_owned(),
        queue: queue.clone(),
        settings: s,
        log: quiet(),
        tx: channel().0,
        watcher: None,
        files: Fingerprints::default(),
        armed: HashMap::new(),
        last_scheduled: HashMap::new(),
        next_full: None,
        next_scan: Some(Instant::now()),
        next_retry: Instant::now() + Duration::from_secs(3600),
    };
    std::fs::write(dir.path().join("r.1.1.mca"), b"0").unwrap();
    service.on_time(Instant::now());
    service.on_time(Instant::now() + Duration::from_secs(1));
    assert_eq!(regions(&queue.take(true).unwrap().task), vec![(1, 1)]);
}
