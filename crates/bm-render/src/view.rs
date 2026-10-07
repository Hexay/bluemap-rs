//! What the renderers see of the world: `ExtendedBlock`'s masked reads over a [`ChunkArea`]. Positions outside the
//! area, in absent or failed chunks read as BlueMap's empty chunk (air, no light, sections 0..=255).
//!
//! Masked states and light of the tile plus its border are copied into a dense [`Volume`] once per tile, so the
//! many neighbour reads per block are index math; reads outside it fall back to the chunks.

use std::cell::Cell;

use bm_math::Color;
use bm_world::{BiomeId, Chunk, ChunkArea, StateId};

use crate::settings::{RenderMask, RenderSettings};
use crate::tints::UniformTints;

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

    /// [`Masking::read`]'s edge rule over a column already read from the chunk, starting at `y0`.
    fn edge_column(&self, x: i32, z: i32, y0: i32, states: &mut [StateId], light: &mut [[u8; 2]]) {
        if !self.render_edges || self.mask.is_none() {
            return;
        }
        for (y, (state, l)) in (y0..).zip(states.iter_mut().zip(light)) {
            if !self.inside(x, y, z) {
                *state = StateId::AIR;
                l[0] = self.edge_sky;
            }
        }
    }
}

/// Per-thread storage for a tile's dense copy, reused across tiles.
#[derive(Default)]
pub(crate) struct Volume {
    bounds: Bounds,
    states: Vec<StateId>,
    /// `[sky, block]`.
    light: Vec<[u8; 2]>,
    solid_top: i32,
    /// Biome ids, read on first use: only tinted blocks need them, 75 each (`BLEND`).
    biome_bounds: Bounds,
    biomes: Vec<Cell<u16>>,
    /// Every chunk the blends read stores biomes per 4×4×4 cell.
    biome_cells: bool,
    tints: UniformTints,
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

    /// [`Bounds::index`] for positions whose 26 neighbours are inside as well.
    fn interior_index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let [ox, oy, oz] = self.origin;
        let [w, h, d] = self.size;
        let (dx, dy, dz) = (x.wrapping_sub(ox), y.wrapping_sub(oy), z.wrapping_sub(oz));
        let interior = |p: i32, s: i32| p >= 1 && p < s - 1;
        (interior(dx, w) && interior(dy, h) && interior(dz, d)).then(|| ((dx * d + dz) * h + dy) as usize)
    }
}

impl Volume {
    /// Fills the volume for blocks x in `min[0]..=max[0]`, z in `min[1]..=max[1]` plus the border.
    /// Above the highest block that isn't `is_air`, only up to the border: see [`View::solid_top`].
    pub fn fill(
        &mut self,
        area: &ChunkArea,
        masking: &Masking,
        min: [i32; 2],
        max: [i32; 2],
        is_air: impl Fn(StateId) -> bool,
    ) {
        let (x0, z0) = (min[0] - BORDER, min[1] - BORDER);
        let (x1, z1) = (max[0] + BORDER, max[1] + BORDER);
        let (mut y0, mut y1, mut top) = (i32::MAX, i32::MIN, i32::MIN);
        for (cx, cz) in ((x0 >> 4)..=(x1 >> 4)).flat_map(|cx| ((z0 >> 4)..=(z1 >> 4)).map(move |cz| (cx, cz))) {
            let (lo, hi) = area.chunk(cx, cz).map_or((0, 255), |c| (c.min_y(), c.max_y()));
            (y0, y1) = (y0.min(lo - BORDER), y1.max(hi + BORDER));
            top = top.max(area.chunk(cx, cz).and_then(|c| c.top_y(&is_air)).unwrap_or(i32::MIN));
        }
        self.solid_top = top.max(y0);
        let y1 = y1.min(self.solid_top + BORDER);
        self.bounds = Bounds::new([x0, y0, z0], [x1, y1, z1]);
        self.states.clear();
        self.light.clear();
        self.states.reserve(self.bounds.len());
        self.light.reserve(self.bounds.len());
        let height = (y1 - y0 + 1) as usize;
        for x in x0..=x1 {
            for z in z0..=z1 {
                let start = self.states.len();
                match area.chunk_at_block(x, z) {
                    Some(c) => c.column_into(x, z, y0, y1, &mut self.states, &mut self.light),
                    None => {
                        self.states.extend(std::iter::repeat_n(StateId::AIR, height));
                        self.light.extend(std::iter::repeat_n([0, 0], height));
                    }
                }
                masking.edge_column(x, z, y0, &mut self.states[start..], &mut self.light[start..]);
            }
        }
        self.biome_bounds = Bounds::new([min[0] - BLEND, y0, min[1] - BLEND], [max[0] + BLEND, y1, max[1] + BLEND]);
        self.biomes.clear();
        self.biomes.resize(self.biome_bounds.len(), Cell::new(UNREAD));
        let chunks = |lo: i32, hi: i32| (lo >> 4)..=(hi >> 4);
        self.biome_cells = chunks(min[0] - BLEND, max[0] + BLEND).all(|cx| {
            chunks(min[1] - BLEND, max[1] + BLEND).all(|cz| area.chunk(cx, cz).is_none_or(Chunk::biome_cells))
        });
        self.tints.clear();
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
    /// Index distance per [`crate::relative::Offset::slot`].
    steps: [isize; 27],
}

impl<'a> View<'a> {
    pub fn new(area: &'a ChunkArea, masking: Masking<'a>, volume: &'a Volume) -> Self {
        let steps = std::array::from_fn(|k| {
            let [_, h, d] = volume.bounds.size;
            let [dx, dy, dz] = [k as i32 / 9 - 1, k as i32 / 3 % 3 - 1, k as i32 % 3 - 1];
            ((dx * d + dz) * h + dy) as isize
        });
        Self { area, masking, volume, steps }
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

    /// The block's index into the tile's dense copy when every neighbour within one block is in it too, so
    /// [`View::step`] from it stays in bounds for offsets in `-1..=1`³.
    pub fn interior_index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        self.volume.bounds.interior_index(x, y, z)
    }

    /// The index `slot` (a [`crate::relative::Offset::slot`] other than `FAR`) away from `i`.
    pub fn step(&self, i: usize, slot: u8) -> usize {
        i.wrapping_add_signed(self.steps[slot as usize])
    }

    pub fn state_at(&self, i: usize) -> StateId {
        self.volume.states[i]
    }

    /// `(sky, block)`.
    pub fn light_at(&self, i: usize) -> (u8, u8) {
        let [sky, block] = self.volume.light[i];
        (sky, block)
    }

    pub fn biome(&self, x: i32, y: i32, z: i32) -> BiomeId {
        self.volume.biome(self.area, x, y, z)
    }

    /// The blended tint of `state` at a block whose blend samples (±2, ±1, ±2) all lie in one biome, from `compute`
    /// once per state and biome; `None` when the samples differ or `compute` has no uniform tint.
    pub fn uniform_tint(
        &self,
        state: StateId,
        (x, y, z): (i32, i32, i32),
        compute: impl FnOnce(BiomeId) -> Option<Color>,
    ) -> Option<Color> {
        if !self.volume.biome_cells {
            return None;
        }
        // with biomes constant per aligned 4-cube, a 5×3×5 box spans at most two cells per axis: its corners cover them
        let b = BLEND;
        let [first, rest @ ..] =
            [(-b, -1, -b), (-b, -1, b), (-b, 1, -b), (-b, 1, b), (b, -1, -b), (b, -1, b), (b, 1, -b), (b, 1, b)];
        let at = |(dx, dy, dz)| self.biome(x + dx, y + dy, z + dz);
        let biome = at(first);
        if !rest.into_iter().all(|corner| at(corner) == biome) {
            return None;
        }
        self.volume.tints.get_or_insert_with(state, biome, || compute(biome))
    }

    /// Every block of the tile above this y is air; the dense copy ends one block above it.
    pub fn solid_top(&self) -> i32 {
        self.volume.solid_top
    }

    /// The highest block light over the blocks of column `x, z` in `y0..=y1` inside the mask.
    pub fn max_block_light(&self, x: i32, z: i32, y0: i32, y1: i32) -> u8 {
        if self.masking.mask.is_some() {
            return (y0..=y1).filter(|&y| self.inside(x, y, z)).map(|y| self.light(x, y, z).1).max().unwrap_or(0);
        }
        self.area.chunk_at_block(x, z).map_or(0, |c| c.max_block_light(x, z, y0, y1))
    }

    pub fn ocean_floor_y(&self, x: i32, z: i32) -> Option<i32> {
        self.area.chunk_at_block(x, z).and_then(|c| c.ocean_floor_y(x, z))
    }

    /// `chunk.getMinY/getMaxY` of the column's chunk.
    pub fn column_y_range(&self, x: i32, z: i32) -> (i32, i32) {
        self.area.chunk_at_block(x, z).map_or((0, 255), |c| (c.min_y(), c.max_y()))
    }
}
