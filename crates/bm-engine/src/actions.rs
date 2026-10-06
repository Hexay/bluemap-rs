//! `WorldRegionUpdateTask.init`: what to do with each hires tile a region owns, from its stored state, whether any
//! chunk under it changed (region-header timestamps vs. the stored "chunk hashes") and the map bounds.

use bm_format::grid::Tile;
use bm_map::renderstate::{
    Action, ActionAndNextState, BoundsSituation, CellIo, MapChunkState, MapTileState, TileUpdateStrategy,
};
use bm_world::World;
use bm_world::region::Region;
use rustc_hash::FxHashMap;

use crate::map::MapContext;
use crate::plan::{Plan, REGION_GRID};

/// Region headers around the region being processed; `None` where the file is missing or unreadable.
pub(crate) struct Headers(FxHashMap<Tile, Option<Region>>);

impl Headers {
    /// The planned regions in the 3×3 neighbourhood of `center`: tiles owned by it can reach into each of them.
    pub fn load(world: &World, plan: &Plan, center: Tile) -> Self {
        let mut map = FxHashMap::default();
        for dx in -1..=1 {
            for dz in -1..=1 {
                let r = (center.0 + dx, center.1 + dz);
                if plan.contains(r) {
                    map.insert(r, world.region(r.0, r.1).ok());
                }
            }
        }
        Self(map)
    }

    /// The chunk's current timestamp as BlueMap records it (0 when not listed), if its region is planned.
    pub fn hash(&self, cx: i32, cz: i32) -> Option<i32> {
        let region = self.0.get(&(cx >> 5, cz >> 5))?;
        let (lx, lz) = ((cx & 31) as usize, (cz & 31) as usize);
        Some(region.as_ref().and_then(|r| r.listed_timestamp(lx, lz)).unwrap_or(0))
    }

    pub fn region(&self, r: Tile) -> Option<&Region> {
        self.0.get(&r).and_then(Option::as_ref)
    }
}

pub(crate) struct TileJob {
    pub tile: Tile,
    pub action: ActionAndNextState,
}

/// The tiles `region` owns, each with its action; `None` actions (nothing to do) are left out.
pub(crate) fn tile_jobs<S: CellIo>(
    ctx: &MapContext,
    plan: &Plan,
    region: Tile,
    headers: &Headers,
    tiles: &mut MapTileState<S>,
    chunks: &mut MapChunkState<S>,
    strategy: TileUpdateStrategy,
) -> Vec<TileJob> {
    let mut jobs = Vec::new();
    for tile in plan.region_tiles(region).filter(|&t| plan.owner(t) == Some(region)) {
        let state = tiles.get(tile.0, tile.1).state;
        let changed = strategy.test(state) || chunks_changed(ctx, tile, headers, chunks);
        let action = state.find_action_and_next_state(changed, bounds(ctx, tile));
        if action.action != Action::None {
            jobs.push(TileJob { tile, action });
        }
    }
    jobs
}

/// Any chunk under the tile whose header timestamp differs from the stored one. Only chunks of planned regions
/// count, as each Java region task only compares its own chunks.
fn chunks_changed<S: CellIo>(ctx: &MapContext, tile: Tile, headers: &Headers, chunks: &mut MapChunkState<S>) -> bool {
    let ((x0, z0), (x1, z1)) = chunk_range(ctx, tile);
    (x0..=x1).any(|cx| (z0..=z1).any(|cz| headers.hash(cx, cz).is_some_and(|h| h != chunks.get(cx, cz))))
}

/// Chunks under a hires tile, inclusive.
pub(crate) fn chunk_range(ctx: &MapContext, tile: Tile) -> ((i32, i32), (i32, i32)) {
    let (x, z) = ctx.hires_grid.tile_min(tile);
    let [w, d] = ctx.hires_grid.size;
    ((x >> 4, z >> 4), ((x + w - 1) >> 4, (z + d - 1) >> 4))
}

fn bounds(ctx: &MapContext, tile: Tile) -> BoundsSituation {
    if !ctx.mask.is_cell_inside(&ctx.hires_grid, tile, true) {
        BoundsSituation::Outside
    } else if ctx.mask.is_cell_inside(&ctx.hires_grid, tile, false) {
        BoundsSituation::Inside
    } else {
        BoundsSituation::Edge
    }
}

/// `complete()`: record the region's chunk timestamps and its update time (or forget it when the file is gone).
pub(crate) fn complete_region<S: CellIo>(
    ctx: &MapContext,
    region: Tile,
    headers: &Headers,
    chunks: &mut MapChunkState<S>,
    regions: &mut bm_map::renderstate::MapRegionState<S>,
    now: i32,
) {
    let (x0, z0) = REGION_GRID.tile_min(region);
    let (cx0, cz0) = (x0 >> 4, z0 >> 4);
    let header = headers.region(region);
    for lx in 0..32 {
        for lz in 0..32 {
            let hash = header.and_then(|r| r.listed_timestamp(lx, lz)).unwrap_or(0);
            chunks.set(cx0 + lx as i32, cz0 + lz as i32, hash);
        }
    }
    if ctx.world.region_exists(region.0, region.1) {
        regions.set(region.0, region.1, now);
    } else {
        regions.delete(region.0, region.1);
    }
}
