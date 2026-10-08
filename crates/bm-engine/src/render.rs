//! `WorldRegionUpdateTask.processTile` for a batch of tiles, in parallel: render (or skip, as BlueMap's tile
//! preconditions say), write the hires tile and hand its lowres columns to the persistence thread.

use std::sync::mpsc::SyncSender;

use bm_compress::Compression;
use bm_format::grid::Tile;
use bm_map::renderstate::{Action, TileState};
use bm_render::{HiresRenderer, StateCache, TileBuffers};
use bm_storage::GridKey;
use bm_world::{BlockStates, ChunkArea, ChunkSlot};
use rayon::prelude::*;

use crate::actions::{TileJob, chunk_range};
use crate::map::MapContext;
use crate::persist::{Column, Msg};
use crate::resources::Resources;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Rendered,
    /// A precondition failed; the tile was un-rendered.
    Skipped,
    Deleted,
}

pub(crate) struct TileResult {
    pub tile: Tile,
    /// `RenderError` when `error` is set.
    pub state: TileState,
    pub outcome: Outcome,
    pub error: Option<String>,
}

pub(crate) struct RegionRender<'a> {
    pub ctx: &'a MapContext,
    pub resources: &'a Resources,
    pub cache: &'a StateCache<'a>,
    pub area: &'a ChunkArea,
    /// The block states as of loading `area`: the shared registry grows while the next region loads.
    pub registry: &'a BlockStates,
    pub queue: &'a SyncSender<Msg>,
}

impl RegionRender<'_> {
    pub fn run(&self, jobs: &[TileJob]) -> Vec<TileResult> {
        let renderer = HiresRenderer {
            pack: &self.resources.pack,
            states: self.cache,
            settings: &self.ctx.render,
            biomes: &self.ctx.biome_table,
            dimension: &self.ctx.world.dimension_type,
        };
        jobs.par_iter()
            .map_init(
                || (TileBuffers::default(), Vec::new(), Vec::new()),
                |(buf, prbm, stored), job| match self.process(&renderer, job, buf, prbm, stored) {
                    Ok((outcome, state)) => TileResult { tile: job.tile, state, outcome, error: None },
                    Err(e) => TileResult {
                        tile: job.tile,
                        state: TileState::RenderError,
                        outcome: Outcome::Rendered,
                        error: Some(format!("tile {:?}: {e}", job.tile)),
                    },
                },
            )
            .collect()
    }

    fn process(
        &self,
        renderer: &HiresRenderer,
        job: &TileJob,
        buf: &mut TileBuffers,
        prbm: &mut Vec<u8>,
        stored: &mut Vec<u8>,
    ) -> Result<(Outcome, TileState), Box<dyn std::error::Error + Send + Sync>> {
        match job.action.action {
            Action::None => unreachable!("tile jobs never carry Action::None"),
            Action::Delete => {
                self.unrender(job.tile)?;
                Ok((Outcome::Deleted, job.action.state))
            }
            Action::Render => {
                if let Some(failed) = self.failed_precondition(job.tile) {
                    self.unrender(job.tile)?;
                    return Ok((Outcome::Skipped, failed));
                }
                let (states, grid) = (self.registry, &self.ctx.hires_grid);
                if self.ctx.save_hires() {
                    renderer.render_tile(self.area, states, grid, job.tile, buf)?;
                    buf.model.write_prbm(prbm)?;
                    if !self.stored_hires_equals(job.tile, prbm, stored) {
                        self.ctx.storage.write_grid(GridKey::Hires, job.tile, prbm)?;
                    }
                    // Java notifies for every saved tile, so unchanged ones too
                    if let Some(listener) = &self.ctx.tile_listener {
                        listener(job.tile, 0);
                    }
                } else {
                    renderer.render_lowres(self.area, states, grid, job.tile, buf)?;
                }
                let columns = buf.columns.iter().map(|c| {
                    let mut color = c.color;
                    let argb = color.straight().get_int() as u32;
                    Column { x: c.x, z: c.z, argb, height: c.height, light: c.block_light as u8 }
                });
                self.send(columns.collect())?;
                Ok((Outcome::Rendered, job.action.state))
            }
        }
    }

    /// Whether storage already holds exactly `prbm` in the configured compression, so a rewrite can be skipped:
    /// re-reading costs ~1.5 ms, a rewrite ~17 ms (compress + atomic write). Unreadable or differently compressed
    /// tiles count as changed. Optimized storage reads return the decoded PRBM uncompressed.
    fn stored_hires_equals(&self, tile: Tile, prbm: &[u8], scratch: &mut Vec<u8>) -> bool {
        let storage = &self.ctx.storage;
        let Ok(Some(stored)) = storage.read_grid(GridKey::Hires, tile) else { return false };
        if stored.compression != storage.grid_compression(GridKey::Hires) {
            return false;
        }
        if stored.compression == Compression::None {
            return stored.data == prbm;
        }
        stored.compression.decompress_into(&stored.data, prbm.len(), scratch).is_ok() && scratch[..] == prbm[..]
    }

    /// `HiresModelManager.unrender`: delete the hires tile, clear its lowres columns.
    fn unrender(&self, tile: Tile) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.ctx.storage.delete_grid(GridKey::Hires, tile)?;
        let (x0, z0) = self.ctx.hires_grid.tile_min(tile);
        let [w, d] = self.ctx.hires_grid.size;
        let columns = (x0..x0 + w).flat_map(|x| (z0..z0 + d).map(move |z| Column { x, z, argb: 0, height: 0, light: 0 }));
        self.send(columns.collect())
    }

    fn send(&self, columns: Vec<Column>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.queue.send(Msg::Columns(columns)).map_err(|_| "the persistence thread has stopped".into())
    }

    /// `checkTileRenderPreconditions`: the state a tile gets instead of being rendered, if any.
    pub fn failed_precondition(&self, tile: Tile) -> Option<TileState> {
        let c = &self.ctx.config;
        let require_light = !c.ignore_missing_light_data;
        let ((x0, z0), (x1, z1)) = chunk_range(self.ctx, tile);
        let (mut generated, mut inhabited) = (false, false);
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                let Some(info) = self.chunk(cx, cz) else { return Some(TileState::ChunkError) };
                if require_light && !info.generated {
                    return Some(TileState::NotGenerated);
                }
                if require_light && !info.has_light {
                    return Some(TileState::MissingLight);
                }
                generated |= info.generated;
                inhabited |= info.inhabited_time >= c.min_inhabited_time;
            }
        }
        if !generated {
            return Some(TileState::NotGenerated);
        }
        let r = c.min_inhabited_time_radius;
        if !inhabited && r > 0 {
            let time = |cx, cz| self.chunk(cx, cz).map_or(0, |i| i.inhabited_time);
            inhabited = (x0 - r..=x1 + r).any(|cx| (z0 - r..=z1 + r).any(|cz| time(cx, cz) >= c.min_inhabited_time));
        }
        (!inhabited).then_some(TileState::LowInhabitedTime)
    }

    /// `None` for an errored chunk (`ERRORED_CHUNK` fails the tile as `ChunkError`).
    fn chunk(&self, cx: i32, cz: i32) -> Option<ChunkInfo> {
        match self.area.slot(cx, cz) {
            Some(ChunkSlot::Loaded(c)) => {
                Some(ChunkInfo { generated: c.generated, has_light: c.has_light, inhabited_time: c.inhabited_time })
            }
            Some(ChunkSlot::Failed(_)) => None,
            Some(ChunkSlot::Absent) | None => Some(ChunkInfo::default()),
        }
    }
}

#[derive(Default)]
struct ChunkInfo {
    generated: bool,
    has_light: bool,
    inhabited_time: i64,
}
