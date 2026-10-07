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
}

impl RenderTask {
    pub fn full(map: impl Into<String>, strategy: TileUpdateStrategy) -> Self {
        Self { map: map.into(), regions: Regions::All, strategy }
    }

    /// What the file watcher schedules for a changed region file.
    pub fn region(map: impl Into<String>, region: Tile) -> Self {
        Self { map: map.into(), regions: Regions::Only([region].into()), strategy: TileUpdateStrategy::ForceNone }
    }

    /// `RenderTask.contains`: running `self` does everything `other` would. Like Java, tasks of different
    /// strategies never contain each other.
    pub fn contains(&self, other: &RenderTask) -> bool {
        self.map == other.map
            && self.strategy == other.strategy
            && match (&self.regions, &other.regions) {
                (Regions::All, _) => true,
                (Regions::Only(_), Regions::All) => false,
                (Regions::Only(mine), Regions::Only(theirs)) => theirs.is_subset(mine),
            }
    }

    /// Adds `other`'s regions when both are region updates of the same map and strategy.
    pub(crate) fn absorb(&mut self, other: &RenderTask) -> bool {
        if self.map != other.map || self.strategy != other.strategy {
            return false;
        }
        match (&mut self.regions, &other.regions) {
            (Regions::Only(mine), Regions::Only(theirs)) => {
                mine.extend(theirs);
                true
            }
            _ => false,
        }
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
}
