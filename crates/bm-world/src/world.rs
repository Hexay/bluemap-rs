//! One dimension of a world, and the chunk working set the renderer reads from: a rectangle of chunks decoded
//! in parallel into a flat array (docs/01 §7 "Working set"), instead of BlueMap's global hashed chunk cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use crate::chunk::{Chunk, ChunkContext};
use crate::dimension::{DimensionType, dimension_folder, load_dimension_type};
use crate::region::{Region, list_regions};
use crate::registry::{BiomeId, Biomes, BlockStates, StateId};
use crate::{Error, Result};

pub struct World {
    pub path: PathBuf,
    /// Namespaced dimension key, e.g. `minecraft:overworld`.
    pub dimension: String,
    pub dimension_type: DimensionType,
    region_dir: PathBuf,
    states: Arc<BlockStates>,
    biomes: Arc<Biomes>,
}

pub enum ChunkSlot {
    /// Never generated.
    Absent,
    Loaded(Chunk),
    /// Unreadable; BlueMap's `ERRORED_CHUNK`. Tiles touching it are skipped, not rendered empty. Shared because
    /// a bad region header fails all of its chunks.
    Failed(Arc<Error>),
}

/// Chunks `[x0, x0 + width) × [z0, z0 + depth)` in chunk coordinates.
pub struct ChunkArea {
    pub x0: i32,
    pub z0: i32,
    pub width: i32,
    pub depth: i32,
    slots: Vec<ChunkSlot>,
}

impl World {
    /// `datapack` resolves dimension type references (see [`load_dimension_type`]).
    pub fn open(
        path: &Path,
        dimension: &str,
        states: Arc<BlockStates>,
        biomes: Arc<Biomes>,
        datapack: &dyn Fn(&str) -> Option<DimensionType>,
    ) -> Result<Self> {
        let dimension = if dimension.contains(':') { dimension.to_owned() } else { format!("minecraft:{dimension}") };
        Ok(Self {
            dimension_type: load_dimension_type(path, &dimension, datapack)?,
            region_dir: dimension_folder(path, &dimension).join("region"),
            path: path.to_owned(),
            dimension,
            states,
            biomes,
        })
    }

    pub fn states(&self) -> &Arc<BlockStates> {
        &self.states
    }

    pub fn biomes(&self) -> &Arc<Biomes> {
        &self.biomes
    }

    /// The folder holding this dimension's `r.<x>.<z>.mca` files (may not exist yet).
    pub fn region_dir(&self) -> &Path {
        &self.region_dir
    }

    pub fn regions(&self) -> Result<Vec<(i32, i32)>> {
        list_regions(&self.region_dir)
    }

    pub fn region(&self, x: i32, z: i32) -> Result<Region> {
        Region::open(&self.region_dir, x, z)
    }

    /// Whether `r.<x>.<z>.mca` exists (even empty), like `Region.exists()`.
    pub fn region_exists(&self, x: i32, z: i32) -> bool {
        self.region_dir.join(format!("r.{x}.{z}.mca")).exists()
    }

    /// Decodes every chunk in the rectangle, in parallel. Region headers that can't be read fail their chunks only.
    pub fn load_area(&self, x0: i32, z0: i32, width: i32, depth: i32) -> ChunkArea {
        let ctx = ChunkContext { states: &self.states, biomes: &self.biomes, dimension: &self.dimension_type };
        let (rx0, rz0) = (x0.div_euclid(32), z0.div_euclid(32));
        let (rx1, rz1) = ((x0 + width - 1).div_euclid(32), (z0 + depth - 1).div_euclid(32));
        let coords: Vec<(i32, i32)> = (rx0..=rx1).flat_map(|rx| (rz0..=rz1).map(move |rz| (rx, rz))).collect();
        let regions: Vec<std::result::Result<Region, Arc<Error>>> = coords
            .into_par_iter()
            .map(|(rx, rz)| {
                let mut region = self.region(rx, rz).map_err(Arc::new)?;
                let local = |lo: i32, len: i32, r: i32| {
                    (lo.max(r * 32) - r * 32) as usize..((lo + len).min(r * 32 + 32) - r * 32) as usize
                };
                let (xs, zs) = (local(x0, width, rx), local(z0, depth, rz));
                region.preload(xs.flat_map(|lx| zs.clone().map(move |lz| (lx, lz))));
                Ok(region)
            })
            .collect();
        let region_of = |cx: i32, cz: i32| {
            &regions[((cx.div_euclid(32) - rx0) * (rz1 - rz0 + 1) + (cz.div_euclid(32) - rz0)) as usize]
        };
        let slots = (0..width * depth)
            .into_par_iter()
            .map_init(
                || (Vec::new(), Vec::new()),
                |(raw, nbt), i| {
                    let (cx, cz) = (x0 + i % width, z0 + i / width);
                    let (lx, lz) = (cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize);
                    let region = match region_of(cx, cz) {
                        Ok(region) => region,
                        Err(e) => return ChunkSlot::Failed(e.clone()),
                    };
                    let chunk = region
                        .read_chunk_into(lx, lz, raw, nbt)
                        .and_then(|found| found.then(|| Chunk::parse(nbt, &ctx)).transpose());
                    match chunk {
                        Ok(None) => ChunkSlot::Absent,
                        Ok(Some(c)) => ChunkSlot::Loaded(c),
                        Err(e) => ChunkSlot::Failed(Arc::new(e)),
                    }
                },
            )
            .collect();
        ChunkArea { x0, z0, width, depth, slots }
    }
}

impl ChunkArea {
    pub fn slot(&self, cx: i32, cz: i32) -> Option<&ChunkSlot> {
        let (dx, dz) = (cx - self.x0, cz - self.z0);
        if dx < 0 || dz < 0 || dx >= self.width || dz >= self.depth {
            return None;
        }
        self.slots.get((dz * self.width + dx) as usize)
    }

    pub fn chunk(&self, cx: i32, cz: i32) -> Option<&Chunk> {
        match self.slot(cx, cz)? {
            ChunkSlot::Loaded(c) => Some(c),
            _ => None,
        }
    }

    pub fn chunk_at_block(&self, x: i32, z: i32) -> Option<&Chunk> {
        self.chunk(x >> 4, z >> 4)
    }

    pub fn block(&self, x: i32, y: i32, z: i32) -> StateId {
        self.chunk_at_block(x, z).map_or(StateId::AIR, |c| c.block(x, y, z))
    }

    pub fn biome(&self, x: i32, y: i32, z: i32) -> BiomeId {
        self.chunk_at_block(x, z).map_or(BiomeId::DEFAULT, |c| c.biome(x, y, z))
    }

    pub fn slots(&self) -> impl Iterator<Item = ((i32, i32), &ChunkSlot)> {
        self.slots
            .iter()
            .enumerate()
            .map(|(i, s)| ((self.x0 + i as i32 % self.width, self.z0 + i as i32 / self.width), s))
    }
}
