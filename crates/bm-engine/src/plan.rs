//! `MapUpdatePreparationTask`: which regions a map update visits and in what order, plus what we add on top —
//! a single owner region per hires tile (BlueMap renders region-boundary tiles twice, docs/10 §9) and, per lowres
//! tile, how many pending regions still write to it (so each one is saved once, when it is final).

use bm_format::grid::{Grid, Tile};
use rustc_hash::FxHashMap;

/// Region files are 512×512 blocks.
pub const REGION_GRID: Grid = Grid { size: [512, 512], offset: [0, 0] };

pub struct Plan {
    /// Processing order.
    pub regions: Vec<Tile>,
    index: FxHashMap<Tile, usize>,
    hires: Grid,
    lowres: LowresTouch,
}

struct LowresTouch {
    grid: Grid,
    lod_count: u32,
    factor: i32,
    /// `(lod, tile)` → regions not yet completed that may write to it.
    pending: FxHashMap<(u32, Tile), u32>,
}

impl Plan {
    /// `regions` with their last update time: sorted oldest first, then by distance to region 0,0, like
    /// `regionLastUpdatedComparator(defaultComparator(ZERO))` (ties broken by position, not `HashSet` order).
    pub fn new(mut regions: Vec<(Tile, i32)>, hires: Grid, lowres: Grid, lod_count: u32, factor: i32) -> Self {
        regions.sort_by_key(|&((x, z), updated)| (updated, i64::from(x).pow(2) + i64::from(z).pow(2), x, z));
        let regions: Vec<Tile> = regions.into_iter().map(|(r, _)| r).collect();
        let index = regions.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        let mut lowres = LowresTouch { grid: lowres, lod_count, factor, pending: FxHashMap::default() };
        for &r in &regions {
            for key in lowres.touched(hires, r) {
                *lowres.pending.entry(key).or_default() += 1;
            }
        }
        Self { regions, index, hires, lowres }
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

    /// Marks `region` done; returns the lowres tiles no remaining region writes to, lowest LOD first.
    pub fn complete(&mut self, region: Tile) -> Vec<(u32, Tile)> {
        let mut done: Vec<(u32, Tile)> = Vec::new();
        for key in self.lowres.touched(self.hires, region) {
            if let Some(n) = self.lowres.pending.get_mut(&key) {
                *n -= 1;
                if *n == 0 {
                    self.lowres.pending.remove(&key);
                    done.push(key);
                }
            }
        }
        done.sort_unstable();
        done
    }
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
