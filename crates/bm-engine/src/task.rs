//! Render tasks as `RenderManager` sees them: a full map update (`MapUpdatePreparationTask` → `MapUpdateTask`) or
//! an update of some regions (`WorldRegionUpdateTask`s), with BlueMap's equality/containment for deduplication.

use std::collections::BTreeSet;

use bm_format::grid::Tile;
use bm_map::renderstate::TileUpdateStrategy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Regions {
    /// Every region file of the world, listed when the task starts.
    All,
    Only(BTreeSet<Tile>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderTask {
    pub map: String,
    pub regions: Regions,
    pub strategy: TileUpdateStrategy,
    /// Regions an interrupted run already finished; skipped when the task runs again (docs/15 §4).
    pub done: BTreeSet<Tile>,
}

impl RenderTask {
    pub fn new(map: impl Into<String>, regions: Regions, strategy: TileUpdateStrategy) -> Self {
        Self { map: map.into(), regions, strategy, done: BTreeSet::new() }
    }

    pub fn full(map: impl Into<String>, strategy: TileUpdateStrategy) -> Self {
        Self::new(map, Regions::All, strategy)
    }

    /// What the file watcher schedules for a changed region file.
    pub fn region(map: impl Into<String>, region: Tile) -> Self {
        Self::new(map, Regions::Only([region].into()), TileUpdateStrategy::ForceNone)
    }

    /// `RenderTask.contains`: running `self` does everything `other` would. Like Java, tasks of different
    /// strategies never contain each other; a partly done task doesn't contain one with fewer regions done.
    pub fn contains(&self, other: &RenderTask) -> bool {
        self.map == other.map
            && self.strategy == other.strategy
            && self.done.is_subset(&other.done)
            && match (&self.regions, &other.regions) {
                (Regions::All, _) => true,
                (Regions::Only(_), Regions::All) => false,
                (Regions::Only(mine), Regions::Only(theirs)) => theirs.is_subset(mine),
            }
    }

    /// Adds `other`'s regions when both are region updates of the same map and strategy; regions it adds are no
    /// longer done.
    pub(crate) fn absorb(&mut self, other: &RenderTask) -> bool {
        if self.map != other.map || self.strategy != other.strategy || !other.done.is_empty() {
            return false;
        }
        match (&mut self.regions, &other.regions) {
            (Regions::Only(mine), Regions::Only(theirs)) => {
                mine.extend(theirs);
                self.done.retain(|r| !theirs.contains(r));
                true
            }
            _ => false,
        }
    }

    /// The regions still to run: `all` lists a whole map's regions, called only when some are done.
    pub fn remaining<E>(&self, all: impl FnOnce() -> Result<Vec<Tile>, E>) -> Result<Regions, E> {
        if self.done.is_empty() {
            return Ok(self.regions.clone());
        }
        let left = match &self.regions {
            Regions::All => all()?.into_iter().filter(|r| !self.done.contains(r)).collect(),
            Regions::Only(set) => set.difference(&self.done).copied().collect(),
        };
        Ok(Regions::Only(left))
    }

    /// `getDescription()`; region positions print like flow-math's `Vector2i`.
    pub fn description(&self) -> String {
        match &self.regions {
            Regions::All => format!("updating map '{}'", self.map),
            Regions::Only(r) if r.len() == 1 => {
                let (x, z) = r.first().copied().unwrap_or_default();
                format!("updating region ({x}, {z})")
            }
            Regions::Only(r) => format!("updating {} regions of map '{}'", r.len(), self.map),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_follows_java() {
        let full = RenderTask::full("w", TileUpdateStrategy::ForceNone);
        let r = RenderTask::region("w", (1, 2));
        assert!(full.contains(&r));
        assert!(!r.contains(&full));
        assert!(!RenderTask::full("w", TileUpdateStrategy::ForceAll).contains(&r));
        assert!(!RenderTask::full("other", TileUpdateStrategy::ForceNone).contains(&r));
        let mut both = RenderTask::region("w", (0, 0));
        assert!(both.absorb(&r));
        assert!(both.contains(&r) && both.contains(&RenderTask::region("w", (0, 0))));
        assert_eq!(both.description(), "updating 2 regions of map 'w'");
        assert_eq!(r.description(), "updating region (1, 2)");
    }

    #[test]
    fn done_regions_are_skipped_and_limit_containment() {
        let mut resumed = RenderTask::full("w", TileUpdateStrategy::ForceAll);
        resumed.done = [(0, 0)].into();
        let fresh = RenderTask::full("w", TileUpdateStrategy::ForceAll);
        assert!(fresh.contains(&resumed) && !resumed.contains(&fresh));
        let left = resumed.remaining(|| Ok::<_, ()>(vec![(0, 0), (1, 0)])).unwrap();
        assert_eq!(left, Regions::Only([(1, 0)].into()));
        assert_eq!(fresh.remaining(|| Err(())), Ok(Regions::All));

        let mut part = RenderTask::region("w", (0, 0));
        part.absorb(&RenderTask::region("w", (1, 0)));
        part.done = [(0, 0), (1, 0)].into();
        assert!(part.absorb(&RenderTask::region("w", (1, 0))));
        assert_eq!(part.done, [(0, 0)].into());
        assert!(!RenderTask::region("w", (2, 0)).absorb(&part));
    }
}
