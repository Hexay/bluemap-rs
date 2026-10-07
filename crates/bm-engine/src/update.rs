//! A full map update (`MapUpdatePreparationTask` → `MapUpdateTask` of `WorldRegionUpdateTask`s): regions one after
//! another, each region's tiles in parallel; render state and lowres are persisted off the render threads.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bm_map::renderstate::{
    Action, CellIo, MapChunkState, MapRegionState, MapTileState, TileInfo, TileUpdateStrategy,
};
use bm_render::StateCache;
use bm_world::ChunkArea;
use rustc_hash::FxHashSet;

use crate::actions::{Headers, TileJob, complete_region, tile_jobs};
use crate::error::Result;
use crate::io::QueuedCells;
use crate::map::MapContext;
use crate::persist::{LowresSettings, Persister};
use crate::plan::{Plan, REGION_GRID};
use crate::render::{Outcome, RegionRender};
use crate::resources::Resources;

/// Render state is saved at least this often during an update, so an interrupted render resumes close to where it
/// stopped.
const SAVE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub struct UpdateStats {
    pub regions: usize,
    pub regions_done: usize,
    pub tiles_rendered: usize,
    /// Tiles a precondition (not generated, missing light, low inhabited time, chunk error) kept from rendering.
    pub tiles_skipped: usize,
    pub tiles_deleted: usize,
    pub tile_errors: usize,
    pub lowres_saves: usize,
}

pub enum UpdateEvent<'a> {
    /// Something BlueMap would log and carry on after.
    Warning(String),
    /// After each region.
    Progress(&'a UpdateStats),
}

struct States<S: CellIo> {
    tiles: MapTileState<S>,
    chunks: MapChunkState<S>,
    regions: MapRegionState<S>,
}

impl<S: CellIo> States<S> {
    fn save(&mut self, on_event: &mut dyn FnMut(UpdateEvent)) -> Result<()> {
        self.tiles.save()?;
        self.chunks.save()?;
        self.regions.save()?;
        let errors = [self.tiles.take_load_errors(), self.chunks.take_load_errors(), self.regions.take_load_errors()];
        errors.into_iter().flatten().for_each(|e| on_event(UpdateEvent::Warning(format!("render state: {e}"))));
        Ok(())
    }
}

/// Updates every region of the map that `strategy` or changed chunks call for.
pub fn update_map(
    ctx: &MapContext,
    resources: &Resources,
    strategy: TileUpdateStrategy,
    on_event: &mut dyn FnMut(UpdateEvent),
) -> Result<UpdateStats> {
    let c = &ctx.config;
    let lowres = LowresSettings { tile_size: c.lowres_tile_size, lod_count: c.lod_count.max(0) as u32, lod_factor: c.lod_factor };
    let persister = Persister::start(ctx.storage.clone(), lowres)?;
    let result = run(ctx, resources, strategy, &persister, on_event);
    let persisted = persister.finish();
    let mut stats = result?;
    stats.lowres_saves = persisted?.lowres_saves;
    Ok(stats)
}

fn run(
    ctx: &MapContext,
    resources: &Resources,
    strategy: TileUpdateStrategy,
    persister: &Persister,
    on_event: &mut dyn FnMut(UpdateEvent),
) -> Result<UpdateStats> {
    let cells = || QueuedCells { storage: ctx.storage.clone(), queue: persister.queue.clone() };
    let mut states =
        States { tiles: MapTileState::new(cells()), chunks: MapChunkState::new(cells()), regions: MapRegionState::new(cells()) };
    let mut stats = UpdateStats::default();

    let Some(mut plan) = plan(ctx, &mut states.regions, on_event)? else { return Ok(stats) };
    stats.regions = plan.regions.len();
    let mut cache = StateCache::new(&resources.pack, &ctx.gallery, &resources.states);
    let mut last_save = Instant::now();
    for region in plan.regions.clone() {
        let headers = Headers::load(&ctx.world, &plan, region);
        if headers.region(region).is_none() {
            // like a cancelled region task: no tiles, no render state written
            on_event(UpdateEvent::Warning(format!("Failed to load chunks for region {region:?}")));
            continue;
        }
        let jobs = tile_jobs(ctx, &plan, region, &headers, &mut states.tiles, &mut states.chunks, strategy);
        if !jobs.is_empty() {
            let area = load_area(ctx, &jobs);
            cache.update(&resources.states);
            let render = RegionRender { ctx, resources, cache: &cache, area: &area, queue: &persister.queue };
            let now = unix_now();
            for r in render.run(&jobs) {
                states.tiles.set(r.tile.0, r.tile.1, TileInfo { render_time: now, state: r.state });
                match (r.error, r.outcome) {
                    (Some(e), _) => {
                        stats.tile_errors += 1;
                        on_event(UpdateEvent::Warning(format!("Error while processing map-tile for map '{}': {e}", ctx.id)));
                    }
                    (None, Outcome::Rendered) => stats.tiles_rendered += 1,
                    (None, Outcome::Skipped) => stats.tiles_skipped += 1,
                    (None, Outcome::Deleted) => stats.tiles_deleted += 1,
                }
            }
        }
        complete_region(ctx, region, &headers, &mut states.chunks, &mut states.regions, unix_now());
        let finished = plan.complete(region);
        if !finished.is_empty() {
            send_flush(persister, finished)?;
        }
        if last_save.elapsed() >= SAVE_INTERVAL {
            states.save(on_event)?;
            last_save = Instant::now();
        }
        stats.regions_done += 1;
        on_event(UpdateEvent::Progress(&stats));
    }
    states.save(on_event)?;
    Ok(stats)
}

/// The planned regions, or `None` (with a warning) when the world has none: then nothing is touched, so a
/// misconfigured world never wipes a map.
fn plan<S: CellIo>(
    ctx: &MapContext,
    region_state: &mut MapRegionState<S>,
    on_event: &mut dyn FnMut(UpdateEvent),
) -> Result<Option<Plan>> {
    let mut regions: FxHashSet<(i32, i32)> =
        ctx.world.regions()?.into_iter().filter(|&r| ctx.mask.is_cell_inside(&REGION_GRID, r, true)).collect();
    if regions.is_empty() {
        let world = ctx.world.path.display();
        let msg = format!("No regions found in world '{world}', update-task for map '{}' will not be created.", ctx.id);
        on_event(UpdateEvent::Warning(msg));
        return Ok(None);
    }
    if ctx.config.check_for_removed_regions {
        region_state.for_each(|x, z, _| _ = regions.insert((x, z)))?;
    }
    let timed = regions.into_iter().map(|r| (r, region_state.get(r.0, r.1))).collect();
    let c = &ctx.config;
    let lowres = bm_format::grid::Grid { size: [c.lowres_tile_size; 2], offset: [0, 0] };
    Ok(Some(Plan::new(timed, ctx.hires_grid, lowres, c.lod_count.max(1) as u32, c.lod_factor.max(1))))
}

/// The chunks the rendered tiles read: their own, a two-block border (biome blending) and the
/// `min-inhabited-time-radius` ring. Tiles that are only deleted need none.
fn load_area(ctx: &MapContext, jobs: &[TileJob]) -> ChunkArea {
    let [w, d] = ctx.hires_grid.size;
    let mins = jobs.iter().filter(|j| j.action.action == Action::Render).map(|j| ctx.hires_grid.tile_min(j.tile));
    let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (x, z) in mins {
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x + w - 1), z1.max(z + d - 1));
    }
    if x0 > x1 {
        return ctx.world.load_area(0, 0, 0, 0);
    }
    let r = ctx.config.min_inhabited_time_radius.max(0);
    let (cx0, cz0) = (((x0 - 2) >> 4) - r, ((z0 - 2) >> 4) - r);
    let (cx1, cz1) = (((x1 + 2) >> 4) + r, ((z1 + 2) >> 4) + r);
    ctx.world.load_area(cx0, cz0, cx1 - cx0 + 1, cz1 - cz0 + 1)
}

fn send_flush(persister: &Persister, tiles: Vec<(u32, bm_format::grid::Tile)>) -> Result<()> {
    persister
        .queue
        .send(crate::persist::Msg::Flush(tiles))
        .map_err(|_| crate::error::Error::Lowres("the persistence thread has stopped".into()))
}

fn unix_now() -> i32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i32)
}
