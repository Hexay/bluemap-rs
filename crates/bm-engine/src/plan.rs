//! `MapUpdatePreparationTask`: which regions a map update visits and in what order, plus what we add on top —
//! a single owner region per hires tile (BlueMap renders region-boundary tiles twice, docs/10 §9) and, per lowres
//! tile and per tile/chunk render-state cell, how many pending regions still touch it (so each one is saved once,
//! when it is final, and leaves memory: what an update holds stays proportional to its front, docs/22).

use std::hash::Hash;

use bm_format::grid::{Grid, Tile};
use bm_map::renderstate::CellKind;
use rustc_hash::FxHashMap;

/// Region files are 512×512 blocks.
pub const REGION_GRID: Grid = Grid { size: [512, 512], offset: [0, 0] };

pub struct Plan {
    /// Processing order.
    pub regions: Vec<Tile>,
    index: FxHashMap<Tile, usize>,
    hires: Grid,
    lowres: LowresTouch,
    pending_lowres: Pending<(u32, Tile)>,
    pending_tiles: Pending<Tile>,
    pending_chunks: Pending<Tile>,
}

/// What no remaining region touches any more once a region completes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Finished {
    /// `(lod, tile)`, lowest LOD first.
    pub lowres: Vec<(u32, Tile)>,
    /// [`CellKind::Tiles`] cells.
    pub tile_cells: Vec<Tile>,
    /// [`CellKind::Chunks`] cells.
    pub chunk_cells: Vec<Tile>,
}

struct LowresTouch {
    grid: Grid,
    lod_count: u32,
    factor: i32,
}

/// Key → regions not yet completed that touch it.
struct Pending<K>(FxHashMap<K, u32>);

impl<K: Hash + Eq + Copy + Ord> Pending<K> {
    fn new() -> Self {
        Self(FxHashMap::default())
    }

    fn add(&mut self, keys: impl IntoIterator<Item = K>) {
        keys.into_iter().for_each(|k| *self.0.entry(k).or_default() += 1);
    }

    fn complete(&mut self, keys: impl IntoIterator<Item = K>) -> Vec<K> {
        let mut done: Vec<K> = keys
            .into_iter()
            .filter(|key| match self.0.get_mut(key) {
                Some(1) => self.0.remove(key).is_some(),
                Some(n) => {
                    *n -= 1;
                    false
                }
                None => false,
            })
            .collect();
        done.sort_unstable();
        done
    }
}

impl Plan {
    /// `regions` with their last update time: sorted oldest first, then by distance to region 0,0, like
    /// `regionLastUpdatedComparator(defaultComparator(ZERO))` (ties broken by position, not `HashSet` order).
    pub fn new(mut regions: Vec<(Tile, i32)>, hires: Grid, lowres: Grid, lod_count: u32, factor: i32) -> Self {
        regions.sort_by_key(|&((x, z), updated)| (updated, i64::from(x).pow(2) + i64::from(z).pow(2), x, z));
        let regions: Vec<Tile> = regions.into_iter().map(|(r, _)| r).collect();
        let index = regions.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        let lowres = LowresTouch { grid: lowres, lod_count, factor };
        let mut plan = Self {
            regions,
            index,
            hires,
            lowres,
            pending_lowres: Pending::new(),
            pending_tiles: Pending::new(),
            pending_chunks: Pending::new(),
        };
        for i in 0..plan.regions.len() {
            let r = plan.regions[i];
            plan.pending_lowres.add(plan.lowres.touched(hires, r));
            let [tiles, chunks] = plan.touched_cells(r);
            plan.pending_tiles.add(tiles);
            plan.pending_chunks.add(chunks);
        }
        plan
    }

    pub fn contains(&self, region: Tile) -> bool {
        self.index.contains_key(&region)
    }

    /// The hires tiles intersecting `region` (17×17 for the default grid).
    pub fn region_tiles(&self, region: Tile) -> impl Iterator<Item = Tile> + '_ {
        let (x0, z0) = REGION_GRID.tile_min(region);
        self.hires.tiles_in(x0, z0, REGION_GRID.size[0], REGION_GRID.size[1])
    }

    /// The planned regions a hires tile's blocks lie in.
    pub fn covering_regions(&self, tile: Tile) -> impl Iterator<Item = Tile> + '_ {
        let (x0, z0) = self.hires.tile_min(tile);
        REGION_GRID.tiles_in(x0, z0, self.hires.size[0], self.hires.size[1]).filter(|r| self.contains(*r))
    }

    /// The region that processes `tile`: the first one in processing order that covers it.
    pub fn owner(&self, tile: Tile) -> Option<Tile> {
        self.covering_regions(tile).min_by_key(|r| self.index[r])
    }

    /// Marks `region` done; returns what no remaining region touches.
    pub fn complete(&mut self, region: Tile) -> Finished {
        let [tiles, chunks] = self.touched_cells(region);
        Finished {
            lowres: self.pending_lowres.complete(self.lowres.touched(self.hires, region)),
            tile_cells: self.pending_tiles.complete(tiles),
            chunk_cells: self.pending_chunks.complete(chunks),
        }
    }

    /// The tile and chunk render-state cells a region's tasks read or write: the states of the hires tiles
    /// intersecting it and the chunk hashes under those tiles (which reach into neighbouring regions).
    fn touched_cells(&self, region: Tile) -> [impl Iterator<Item = Tile> + use<>; 2] {
        let (x0, z0) = REGION_GRID.tile_min(region);
        let [w, d] = REGION_GRID.size;
        let (first, last) = (self.hires.tile_of(x0, z0), self.hires.tile_of(x0 + w - 1, z0 + d - 1));
        let (bx0, bz0) = self.hires.tile_min(first);
        let (bx1, bz1) = self.hires.tile_min(last);
        let (bx1, bz1) = (bx1 + self.hires.size[0] - 1, bz1 + self.hires.size[1] - 1);
        [
            cell_rect(CellKind::Tiles, first, last),
            cell_rect(CellKind::Chunks, (bx0 >> 4, bz0 >> 4), (bx1 >> 4, bz1 >> 4)),
        ]
    }
}

/// The cells of `kind` holding entries `min..=max`.
fn cell_rect(kind: CellKind, min: Tile, max: Tile) -> impl Iterator<Item = Tile> + use<> {
    let ((x0, z0), (x1, z1)) = (kind.cell_of(min.0, min.1), kind.cell_of(max.0, max.1));
    (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| (x, z)))
}

impl LowresTouch {
    /// Every `(lod, tile)` the hires tiles of `region` can write to: LOD 1 under their columns, one tile further
    /// towards -x/-z for the seam copies, and the same cascaded through each coarser LOD. A superset is harmless.
    fn touched(&self, hires: Grid, region: Tile) -> Vec<(u32, Tile)> {
        let (rx, rz) = REGION_GRID.tile_min(region);
        let first = hires.tile_min(hires.tile_of(rx, rz));
        let last = hires.tile_min(hires.tile_of(rx + REGION_GRID.size[0] - 1, rz + REGION_GRID.size[1] - 1));
        let (min, max) = ((first.0 - 1, first.1 - 1), (last.0 + hires.size[0] - 1, last.1 + hires.size[1] - 1));
        let (mut lo, mut hi) = (self.grid.tile_of(min.0, min.1), self.grid.tile_of(max.0, max.1));
        let mut out = Vec::new();
        for lod in 1..=self.lod_count {
            for x in lo.0..=hi.0 {
                out.extend((lo.1..=hi.1).map(|z| (lod, (x, z))));
            }
            let f = self.factor;
            lo = (lo.0.div_euclid(f) - 1, lo.1.div_euclid(f) - 1);
            hi = (hi.0.div_euclid(f), hi.1.div_euclid(f));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn everything_finishes_once_after_its_last_toucher() {
        let hires = Grid { size: [32, 32], offset: [2, 2] };
        let lowres = Grid { size: [500, 500], offset: [0, 0] };
        let regions: Vec<(Tile, i32)> = (-3..3).flat_map(|x| (-3..3).map(move |z| ((x, z), 0))).collect();
        let mut plan = Plan::new(regions, hires, lowres, 3, 5);
        let order = plan.regions.clone();
        let touched = |plan: &Plan, r: Tile| {
            let [t, c] = plan.touched_cells(r);
            (plan.lowres.touched(hires, r), t.collect::<Vec<_>>(), c.collect::<Vec<_>>())
        };
        let (mut lowres_seen, mut tiles_seen, mut chunks_seen) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for (i, &r) in order.iter().enumerate() {
            let done = plan.complete(r);
            for &later in &order[i + 1..] {
                let (l, t, c) = touched(&plan, later);
                assert!(done.lowres.iter().all(|k| !l.contains(k)), "lowres finished early");
                assert!(done.tile_cells.iter().all(|k| !t.contains(k)), "tile cell finished early");
                assert!(done.chunk_cells.iter().all(|k| !c.contains(k)), "chunk cell finished early");
            }
            assert!(done.lowres.iter().all(|&k| lowres_seen.insert(k)));
            assert!(done.tile_cells.iter().all(|&k| tiles_seen.insert(k)));
            assert!(done.chunk_cells.iter().all(|&k| chunks_seen.insert(k)));
        }
        let all = order.iter().map(|&r| touched(&plan, r));
        let (mut l, mut t, mut c) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for (a, b, d) in all {
            l.extend(a);
            t.extend(b);
            c.extend(d);
        }
        assert_eq!((lowres_seen, tiles_seen, chunks_seen), (l, t, c));
    }
}
