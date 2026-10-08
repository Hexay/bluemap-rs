//! What a CLI render starts with, and resuming `-f` across runs (docs/15 §4): a forced render cut short by a
//! shutdown is saved to `<data>/tasks.dat`; the next `-f` on that map continues past its done regions, and
//! `--restart` starts over. Entries for other maps (or a plugin's) are carried over untouched.

use std::path::{Path, PathBuf};

use bm_engine::{Regions, RenderQueue, RenderTask, TileUpdateStrategy};

use crate::log;
use crate::tasks_dat::{self, Loaded, RegionLister};

pub struct RenderPlan {
    strategy: TileUpdateStrategy,
    /// `Some` for `-f`.
    resume: Option<Resume>,
}

struct Resume {
    data: PathBuf,
    restart: bool,
    saved: Loaded,
}

impl RenderPlan {
    pub fn new(strategy: TileUpdateStrategy, data: &Path, restart: bool) -> Self {
        let resume = (strategy == TileUpdateStrategy::ForceAll).then(|| Resume {
            data: data.to_owned(),
            restart,
            saved: read(data),
        });
        Self { strategy, resume }
    }

    /// One task per map: the saved remainder of an interrupted `-f`, else a full update.
    pub fn tasks(&mut self, map: &str) -> Vec<RenderTask> {
        let full = || vec![RenderTask::full(map, self.strategy)];
        let Some(resume) = &mut self.resume else { return full() };
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut resume.saved.tasks)
            .into_iter()
            .partition(|t| t.map == map && t.strategy == TileUpdateStrategy::ForceAll);
        resume.saved.tasks = rest;
        if mine.is_empty() || resume.restart {
            return full();
        }
        let left: usize = mine.iter().map(region_count).sum();
        log::info(&format!(
            "Resuming the interrupted forced render of map '{map}': {left} regions left (--restart starts over)"
        ));
        mine
    }

    /// After the run: saves what a shutdown cut short of the forced render (plus carried entries), or removes the
    /// file once nothing is left.
    pub fn save(self, queue: &RenderQueue, regions: RegionLister) {
        let Some(resume) = self.resume else { return };
        let mut tasks: Vec<RenderTask> =
            queue.abandoned_tasks().into_iter().filter(|t| t.strategy == TileUpdateStrategy::ForceAll).collect();
        let interrupted = !tasks.is_empty();
        tasks.extend(resume.saved.tasks);
        let path = tasks_dat::file(&resume.data);
        let result = if tasks.is_empty() && resume.saved.purges.is_empty() {
            std::fs::remove_file(&path)
                .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        } else {
            tasks_dat::write(&resume.data, &tasks_dat::encode(&tasks, &resume.saved.purges, regions))
        };
        match result {
            Err(e) => log::error(&format!("Failed to save tasks.dat! {e}")),
            Ok(()) if interrupted => log::info("Saved the forced render's progress; run with -f again to resume it."),
            Ok(()) => {}
        }
    }
}

fn region_count(task: &RenderTask) -> usize {
    match &task.regions {
        Regions::Only(set) => set.len(),
        Regions::All => 0,
    }
}

fn read(data: &Path) -> Loaded {
    let Ok(bytes) = std::fs::read(tasks_dat::file(data)) else { return Loaded::default() };
    tasks_dat::decode(&bytes, &|_| true).unwrap_or_else(|e| {
        log::warn(&format!("Ignoring an unreadable tasks.dat: {e}"));
        Loaded::default()
    })
}

#[cfg(test)]
mod tests;
