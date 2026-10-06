//! What the renderers see of the world: `ExtendedBlock`'s masked reads over a [`ChunkArea`]. Positions outside the
//! area, in absent or failed chunks read as BlueMap's empty chunk (air, no light, sections 0..=255).
//!
//! Masked states and light of the tile plus its border are copied into a dense [`Volume`] once per tile, so the
//! many neighbour reads per block are index math; reads outside it fall back to the chunks.

use std::cell::Cell;

use bm_world::{BiomeId, Chunk, ChunkArea, StateId};

use crate::settings::{RenderMask, RenderSettings};

/// Blocks around the tile the renderers read states and light of (AO diagonals, liquid corners).
const BORDER: i32 = 1;

/// The render-edge rule, with the mask dropped when the tile is wholly inside it.
#[derive(Clone, Copy)]
pub(crate) struct Masking<'a> {
    mask: Option<&'a dyn RenderMask>,
    render_edges: bool,
    edge_sky: u8,
}

impl<'a> Masking<'a> {
    pub fn new(settings: &'a RenderSettings, has_skylight: bool, min: [i32; 2], max: [i32; 2]) -> Self {
        // the tile, the volume's border and the cave/biome reads one further out
        let b = BORDER + 1;
        let whole = |m: &&dyn RenderMask| m.test_area([min[0] - b, min[1] - b], [max[0] + b, max[1] + b]) == Some(true);
        Self {
            mask: settings.mask.as_deref().filter(|m| !whole(m)),
            render_edges: settings.render_edges,
            edge_sky: if has_skylight { settings.edge_light_strength as u8 } else { 0 },
        }
    }

    pub fn inside(&self, x: i32, y: i32, z: i32) -> bool {
        self.mask.is_none_or(|m| m.test(x, y, z))
    }

    pub fn inside_column(&self, x: i32, z: i32) -> bool {
        self.mask.is_none_or(|m| m.test_column(x, z))
    }

    fn edge(&self, x: i32, y: i32, z: i32) -> bool {
        self.render_edges && !self.inside(x, y, z)
    }

    /// A chunk read as the renderers see it.
    fn read(&self, chunk: Option<&Chunk>, x: i32, y: i32, z: i32) -> (StateId, [u8; 2]) {
        let (state, (sky, block)) = chunk.map_or((StateId::AIR, (0, 0)), |c| (c.block(x, y, z), c.light(x, y, z)));
        if self.edge(x, y, z) { (StateId::AIR, [self.edge_sky, block]) } else { (state, [sky, block]) }
    }
}

/// Per-thread storage for a tile's dense copy, reused across tiles.
#[derive(Default)]
pub(crate) struct Volume {
    bounds: Bounds,
    states: Vec<StateId>,
    /// `[sky, block]`.
    light: Vec<[u8; 2]>,
    /// Biome ids, read on first use: only tinted blocks need them, 75 each (`BLEND`).
    biome_bounds: Bounds,
    biomes: Vec<Cell<u16>>,
}

/// Blocks the biome blend reaches around a block.
const BLEND: i32 = 2;
const UNREAD: u16 = u16::MAX;

#[derive(Default)]
struct Bounds {
    origin: [i32; 3],
    size: [i32; 3],
}

impl Bounds {
    fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { origin: min, size: [max[0] - min[0] + 1, max[1] - min[1] + 1, max[2] - min[2] + 1] }
    }

    fn len(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    /// Columns are contiguous in y.
    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let [ox, oy, oz] = self.origin;
        let [w, h, d] = self.size;
        let (dx, dy, dz) = (x.wrapping_sub(ox), y.wrapping_sub(oy), z.wrapping_sub(oz));
        let inside = (dx as u32) < w as u32 && (dy as u32) < h as u32 && (dz as u32) < d as u32;
        inside.then(|| ((dx * d + dz) * h + dy) as usize)
    }
}

impl Volume {
    /// Fills the volume for blocks x in `min[0]..=max[0]`, z in `min[1]..=max[1]` plus the border.
    pub fn fill(&mut self, area: &ChunkArea, masking: &Masking, min: [i32; 2], max: [i32; 2]) {
        let (x0, z0) = (min[0] - BORDER, min[1] - BORDER);
        let (x1, z1) = (max[0] + BORDER, max[1] + BORDER);
        let (mut y0, mut y1) = (i32::MAX, i32::MIN);
        for x in x0..=x1 {
            for z in z0..=z1 {
                let (lo, hi) = area.chunk_at_block(x, z).map_or((0, 255), |c| (c.min_y(), c.max_y()));
                (y0, y1) = (y0.min(lo - BORDER), y1.max(hi + BORDER));
            }
        }
        self.bounds = Bounds::new([x0, y0, z0], [x1, y1, z1]);
        self.states.clear();
        self.light.clear();
        for x in x0..=x1 {
            for z in z0..=z1 {
                let chunk = area.chunk_at_block(x, z);
                for y in y0..=y1 {
                    let (state, light) = masking.read(chunk, x, y, z);
                    self.states.push(state);
                    self.light.push(light);
                }
            }
        }
        self.biome_bounds = Bounds::new([min[0] - BLEND, y0, min[1] - BLEND], [max[0] + BLEND, y1, max[1] + BLEND]);
        self.biomes.clear();
        self.biomes.resize(self.biome_bounds.len(), Cell::new(UNREAD));
    }

    fn biome(&self, area: &ChunkArea, x: i32, y: i32, z: i32) -> BiomeId {
        let Some(i) = self.biome_bounds.index(x, y, z) else { return area.biome(x, y, z) };
        let cell = &self.biomes[i];
        if cell.get() == UNREAD {
            cell.set(area.biome(x, y, z).0);
        }
        BiomeId(cell.get())
    }
}

pub(crate) struct View<'a> {
    pub area: &'a ChunkArea,
    masking: Masking<'a>,
    volume: &'a Volume,
}

impl<'a> View<'a> {
    pub fn new(area: &'a ChunkArea, masking: Masking<'a>, volume: &'a Volume) -> Self {
        Self { area, masking, volume }
    }

    pub fn inside(&self, x: i32, y: i32, z: i32) -> bool {
        self.masking.inside(x, y, z)
    }

    pub fn inside_column(&self, x: i32, z: i32) -> bool {
        self.masking.inside_column(x, z)
    }

    pub fn state(&self, x: i32, y: i32, z: i32) -> StateId {
        match self.volume.bounds.index(x, y, z) {
            Some(i) => self.volume.states[i],
            None => self.masking.read(self.area.chunk_at_block(x, z), x, y, z).0,
        }
    }

    /// `(sky, block)`.
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let [sky, block] = match self.volume.bounds.index(x, y, z) {
            Some(i) => self.volume.light[i],
            None => self.masking.read(self.area.chunk_at_block(x, z), x, y, z).1,
        };
        (sky, block)
    }

    pub fn biome(&self, x: i32, y: i32, z: i32) -> BiomeId {
        self.volume.biome(self.area, x, y, z)
    }

    pub fn ocean_floor_y(&self, x: i32, z: i32) -> Option<i32> {
        self.area.chunk_at_block(x, z).and_then(|c| c.ocean_floor_y(x, z))
    }

    /// `chunk.getMinY/getMaxY` of the column's chunk.
    pub fn column_y_range(&self, x: i32, z: i32) -> (i32, i32) {
        self.area.chunk_at_block(x, z).map_or((0, 255), |c| (c.min_y(), c.max_y()))
    }
}
