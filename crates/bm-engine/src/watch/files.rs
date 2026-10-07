//! The file side of a map's update service: an OS watcher on the region folder (polling where the OS one can't
//! start) and fingerprints of the region files for rescans.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use bm_format::grid::Tile;
use bm_world::region::parse_region_file_name;
use notify::{Config, Event, EventKind, PollWatcher, RecursiveMode, Watcher};

/// What the watcher thread reports to the service.
pub(super) enum Msg {
    Changed(Vec<Tile>),
    /// Events were lost (queue overflow): compare fingerprints.
    Rescan,
    Error(String),
    Close,
}

/// How often the polling fallback looks at the folder.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub(super) type BoxedWatcher = Box<dyn Watcher + Send>;

/// Watches `dir` (non-recursively, like Java's `WatchService` on the region folder). Falls back to polling when the
/// OS watcher can't be set up; the error says why the OS watcher failed.
pub(super) fn watch(dir: &Path, tx: &Sender<Msg>) -> (notify::Result<BoxedWatcher>, Option<notify::Error>) {
    let native = notify::recommended_watcher(handler(tx.clone())).and_then(|mut w| {
        w.watch(dir, RecursiveMode::NonRecursive)?;
        Ok(Box::new(w) as BoxedWatcher)
    });
    match native {
        Ok(w) => (Ok(w), None),
        Err(e) => {
            let polling = PollWatcher::new(handler(tx.clone()), Config::default().with_poll_interval(POLL_INTERVAL))
                .and_then(|mut w| {
                    w.watch(dir, RecursiveMode::NonRecursive)?;
                    Ok(Box::new(w) as BoxedWatcher)
                });
            (polling, Some(e))
        }
    }
}

fn handler(tx: Sender<Msg>) -> impl Fn(notify::Result<Event>) + Send + 'static {
    move |res| {
        let msg = match res {
            Ok(e) if e.need_rescan() => Msg::Rescan,
            // our own reads must not trigger updates
            Ok(Event { kind: EventKind::Access(_), .. }) => return,
            Ok(e) => Msg::Changed(e.paths.iter().filter_map(|p| region_of(p)).collect()),
            Err(e) => Msg::Error(e.to_string()),
        };
        let _ = tx.send(msg);
    }
}

/// `RegionType.regionForFileName`.
fn region_of(path: &Path) -> Option<Tile> {
    parse_region_file_name(path.file_name()?.to_str()?)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fingerprint {
    modified: Option<SystemTime>,
    len: u64,
}

/// Last seen fingerprint of every region file in a folder.
#[derive(Default)]
pub(super) struct Fingerprints(HashMap<Tile, Fingerprint>);

impl Fingerprints {
    /// Re-reads `dir`; returns the regions whose file appeared, changed or disappeared. A missing folder counts as
    /// empty.
    pub fn rescan(&mut self, dir: &Path) -> std::io::Result<Vec<Tile>> {
        let now = read(dir)?;
        let mut changed: Vec<Tile> = now.iter().filter(|(r, f)| self.0.get(r) != Some(f)).map(|(r, _)| *r).collect();
        changed.extend(self.0.keys().filter(|r| !now.contains_key(r)));
        changed.sort_unstable();
        self.0 = now;
        Ok(changed)
    }
}

fn read(dir: &Path) -> std::io::Result<HashMap<Tile, Fingerprint>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e),
    };
    let mut out = HashMap::new();
    for entry in entries {
        let entry = entry?;
        let Some(region) = entry.file_name().to_str().and_then(parse_region_file_name) else { continue };
        // a file deleted between listing and stat simply drops out
        if let Ok(meta) = entry.metadata() {
            out.insert(region, Fingerprint { modified: meta.modified().ok(), len: meta.len() });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescan_reports_new_changed_and_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut prints = Fingerprints::default();
        assert!(prints.rescan(&dir.path().join("missing")).unwrap().is_empty());
        std::fs::write(dir.path().join("r.0.0.mca"), b"a").unwrap();
        std::fs::write(dir.path().join("r.-1.2.mca"), b"a").unwrap();
        std::fs::write(dir.path().join("level.dat"), b"a").unwrap();
        assert_eq!(prints.rescan(dir.path()).unwrap(), vec![(-1, 2), (0, 0)]);
        assert!(prints.rescan(dir.path()).unwrap().is_empty());
        std::fs::write(dir.path().join("r.0.0.mca"), b"ab").unwrap();
        std::fs::remove_file(dir.path().join("r.-1.2.mca")).unwrap();
        assert_eq!(prints.rescan(dir.path()).unwrap(), vec![(-1, 2), (0, 0)]);
    }
}
