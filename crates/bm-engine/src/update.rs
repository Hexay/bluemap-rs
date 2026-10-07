//! A map update (`MapUpdatePreparationTask` → `MapUpdateTask` of `WorldRegionUpdateTask`s, or just some regions'
//! tasks): regions in plan order, each region's tiles in parallel while the next region is prepared; render state
//! and lowres are persisted off the render threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ScopedJoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bm_map::renderstate::{
    Action, CellIo, MapChunkState, MapRegionState, MapTileState, TileInfo, TileUpdateStrategy,
};
use bm_format::grid::Tile;
use bm_render::StateCache;
use bm_world::ChunkArea;
use rustc_hash::FxHashSet;

use crate::actions::{Headers, TileJob, complete_region, tile_jobs};
use crate::error::Result;
use crate::io::QueuedCells;
use crate::map::MapContext;
use crate::persist::{LowresSettings, Persister};
use crate::plan::{Plan, REGION_GRID};
use crate::render::{Outcome, RegionRender, TileResult};
use crate::resources::Resources;
use crate::task::Regions;

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
    /// Stopped early by [`UpdateJob::cancel`]; finished regions are saved.
    pub cancelled: bool,
}

/// What a map update does: which regions, how forcefully, and a flag that stops it after the current region.
#[derive(Clone, Copy)]
pub struct UpdateJob<'a> {
    pub strategy: TileUpdateStrategy,
    pub regions: &'a Regions,
    pub cancel: &'a AtomicBool,
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

/// Updates every region of `job` that its strategy or changed chunks call for.
pub fn update_map(
    ctx: &MapContext,
    resources: &Resources,
    job: UpdateJob,
    on_event: &mut dyn FnMut(UpdateEvent),
) -> Result<UpdateStats> {
    let c = &ctx.config;
    let lowres = LowresSettings { tile_size: c.lowres_tile_size, lod_count: c.lod_count.max(0) as u32, lod_factor: c.lod_factor };
    let persister = Persister::start(ctx.storage.clone(), lowres, ctx.tile_listener.clone())?;
    let result = run(ctx, resources, job, &persister, on_event);
    let persisted = persister.finish();
    let mut stats = result?;
    stats.lowres_saves = persisted?.lowres_saves;
    Ok(stats)
}

/// Regions are prepared (tile jobs, chunk area) on this thread while the previous region still renders, so the
/// pool never drains at a region boundary. Overlap is safe: a region's owned tiles lie in no earlier region, so
/// its jobs don't depend on the render state an earlier region records, and no two regions share a lowres pixel.
fn run(
    ctx: &MapContext,
    resources: &Resources,
    job: UpdateJob,
    persister: &Persister,
    on_event: &mut dyn FnMut(UpdateEvent),
) -> Result<UpdateStats> {
    let cells = || QueuedCells { storage: ctx.storage.clone(), queue: persister.queue.clone() };
    let mut states =
        States { tiles: MapTileState::new(cells()), chunks: MapChunkState::new(cells()), regions: MapRegionState::new(cells()) };
    let mut stats = UpdateStats::default();

    let Some(plan) = plan(ctx, job.regions, &mut states.regions, on_event)? else { return Ok(stats) };
    stats.regions = plan.regions.len();
    let mut cache = Arc::new(StateCache::new(&resources.pack, &ctx.gallery, &resources.states));
    let mut done = Done { ctx, persister, states, plan, stats, last_save: Instant::now(), on_event };
    std::thread::scope(|s| -> Result<()> {
        let mut in_flight: Option<InFlight> = None;
        for region in done.plan.regions.clone() {
            if job.cancel.load(Ordering::Relaxed) {
                done.stats.cancelled = true;
                break;
            }
            let headers = Headers::load(&ctx.world, &done.plan, region);
            let jobs = match headers.region(region) {
                Some(_) => {
                    let States { tiles, chunks, .. } = &mut done.states;
                    tile_jobs(ctx, &done.plan, region, &headers, tiles, chunks, job.strategy)
                }
                None => Vec::new(),
            };
            let area = (!jobs.is_empty()).then(|| (load_area(ctx, &jobs), resources.states.snapshot()));
            if area.as_ref().is_some_and(|(_, registry)| registry.len() > cache.len()) {
                // the in-flight render reads the cache, so it has to end before new states are resolved
                if let Some(prev) = in_flight.take() {
                    done.finish(prev)?;
                }
                Arc::get_mut(&mut cache).expect("no render in flight").update(&resources.states);
            }
            let started = unix_now();
            let render = area.map(|(area, registry)| {
                let (cache, queue) = (cache.clone(), &persister.queue);
                s.spawn(move || {
                    RegionRender { ctx, resources, cache: &cache, area: &area, registry: &registry, queue }.run(&jobs)
                })
            });
            if let Some(prev) = in_flight.replace(InFlight { region, headers, started, render }) {
                done.finish(prev)?;
            }
        }
        in_flight.map_or(Ok(()), |last| done.finish(last))
    })?;
    done.states.save(done.on_event)?;
    Ok(done.stats)
}

/// A region whose tile jobs are planned; `render` is `None` when it has none.
struct InFlight<'s> {
    region: Tile,
    headers: Headers,
    started: i32,
    render: Option<ScopedJoinHandle<'s, Vec<TileResult>>>,
}

/// Bookkeeping of finished regions, in plan order.
struct Done<'a, S: CellIo> {
    ctx: &'a MapContext,
    persister: &'a Persister,
    states: States<S>,
    plan: Plan,
    stats: UpdateStats,
    last_save: Instant,
    on_event: &'a mut dyn FnMut(UpdateEvent),
}

impl<S: CellIo> Done<'_, S> {
    /// Waits for the region's render, then records its tiles, chunks and lowres flushes.
    fn finish(&mut self, f: InFlight) -> Result<()> {
        let results = match f.render {
            Some(handle) => handle.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            None => Vec::new(),
        };
        let (ctx, region) = (self.ctx, f.region);
        if f.headers.region(region).is_none() {
            // like a cancelled region task: no tiles, no render state written
            (self.on_event)(UpdateEvent::Warning(format!("Failed to load chunks for region {region:?}")));
            return Ok(());
        }
        for r in results {
            self.states.tiles.set(r.tile.0, r.tile.1, TileInfo { render_time: f.started, state: r.state });
            match (r.error, r.outcome) {
                (Some(e), _) => {
                    self.stats.tile_errors += 1;
                    let msg = format!("Error while processing map-tile for map '{}': {e}", ctx.id);
                    (self.on_event)(UpdateEvent::Warning(msg));
                }
                (None, Outcome::Rendered) => self.stats.tiles_rendered += 1,
                (None, Outcome::Skipped) => self.stats.tiles_skipped += 1,
                (None, Outcome::Deleted) => self.stats.tiles_deleted += 1,
            }
        }
        let States { chunks, regions, .. } = &mut self.states;
        complete_region(ctx, region, &f.headers, chunks, regions, unix_now());
        let finished = self.plan.complete(region);
        if !finished.is_empty() {
            send_flush(self.persister, finished)?;
        }
        if self.last_save.elapsed() >= SAVE_INTERVAL {
            self.states.save(self.on_event)?;
            self.last_save = Instant::now();
        }
        self.stats.regions_done += 1;
        (self.on_event)(UpdateEvent::Progress(&self.stats));
        Ok(())
    }
}

/// The planned regions, or `None` when there are none; for a full update that is warned about and nothing is
/// touched, so a misconfigured world never wipes a map.
fn plan<S: CellIo>(
    ctx: &MapContext,
    only: &Regions,
    region_state: &mut MapRegionState<S>,
    on_event: &mut dyn FnMut(UpdateEvent),
) -> Result<Option<Plan>> {
    let inside = |r: &(i32, i32)| ctx.mask.is_cell_inside(&REGION_GRID, *r, true);
    let mut regions: FxHashSet<(i32, i32)> = match only {
        Regions::All => ctx.world.regions()?.into_iter().filter(inside).collect(),
        Regions::Only(set) => {
            let regions: FxHashSet<_> = set.iter().copied().filter(inside).collect();
            return Ok((!regions.is_empty()).then(|| new_plan(ctx, regions, region_state)));
        }
    };
    if regions.is_empty() {
        let world = ctx.world.path.display();
        let msg = format!("No regions found in world '{world}', update-task for map '{}' will not be created.", ctx.id);
        on_event(UpdateEvent::Warning(msg));
        return Ok(None);
    }
    if ctx.config.check_for_removed_regions {
        region_state.for_each(|x, z, _| _ = regions.insert((x, z)))?;
    }
    Ok(Some(new_plan(ctx, regions, region_state)))
}

fn new_plan<S: CellIo>(ctx: &MapContext, regions: FxHashSet<(i32, i32)>, region_state: &mut MapRegionState<S>) -> Plan {
    let timed = regions.into_iter().map(|r| (r, region_state.get(r.0, r.1))).collect();
    let c = &ctx.config;
    let lowres = bm_format::grid::Grid { size: [c.lowres_tile_size; 2], offset: [0, 0] };
    Plan::new(timed, ctx.hires_grid, lowres, c.lod_count.max(1) as u32, c.lod_factor.max(1))
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
