//! Sections decoded into flat per-chunk [`Arrays`]: a chunk's block indices, palettes, light and biome cells each
//! sit in one allocation sized before decoding, instead of several boxes per section (docs/23).

use bm_nbt::{BeArray, Compound, List, Tag};

use super::modern::palette_state;
use super::{Blocks, ChunkContext, Light, Section, SectionBiomes};
use crate::packed::{Padded, ceil_log2, padded_block_bits, spanning_get};
use crate::registry::{BiomeId, StateId};

/// A section as a format's reader found it, still pointing into the NBT.
pub(super) struct RawSection<'a> {
    pub y: i32,
    pub palette: Option<List<'a>>,
    pub data: Option<BeArray<'a, 8>>,
    /// 1.13–1.15 index arrays, re-packed padded so lookups share one code path.
    pub spanning: bool,
    pub biome_palette: Option<List<'a>>,
    pub biome_data: Option<BeArray<'a, 8>>,
    pub block_light: Option<RawLight<'a>>,
    pub sky_light: Option<RawLight<'a>>,
}

/// A nibble array, or one value for all 4096 blocks (most sections are fully dark or fully lit).
#[derive(Clone, Copy)]
pub(super) enum RawLight<'a> {
    Uniform(u8),
    Nibbles(&'a [u8; 2048]),
}

/// The chunk-wide storage sections index into.
#[derive(Default)]
pub(super) struct Arrays {
    pub words: Box<[u64]>,
    pub palettes: Box<[StateId]>,
    pub light: Box<[[u8; 2048]]>,
    pub biomes: Box<[[BiomeId; 64]]>,
}

/// A range of one of the [`Arrays`].
#[derive(Clone, Copy)]
pub(super) struct Span {
    start: u32,
    len: u32,
}

/// Sections by y, contiguous from `min`, gaps `None`.
#[derive(Default)]
pub(super) struct Sections {
    pub min: i32,
    pub sections: Vec<Option<Section>>,
    pub arrays: Arrays,
}

impl<'a> RawLight<'a> {
    pub fn from_nibbles(bytes: &'a [u8]) -> Option<Self> {
        let nibbles: &[u8; 2048] = bytes.try_into().ok()?;
        let first = nibbles[0];
        let uniform = first & 15 == first >> 4 && nibbles.iter().all(|&b| b == first);
        Some(if uniform { Self::Uniform(first & 15) } else { Self::Nibbles(nibbles) })
    }
}

impl RawSection<'_> {
    fn palette_len(&self) -> usize {
        self.palette.map_or(0, |l| l.len())
    }

    fn word_count(&self) -> usize {
        match self.data {
            Some(data) if self.palette_len() >= 2 && self.spanning => 4096usize.div_ceil(64 / spanning_bits(data)),
            Some(data) if self.palette_len() >= 2 => data.len(),
            _ => 0,
        }
    }

    fn nibble_arrays(&self) -> usize {
        [self.block_light, self.sky_light].iter().filter(|l| matches!(l, Some(RawLight::Nibbles(_)))).count()
    }

    fn has_biome_cells(&self) -> bool {
        self.biome_palette.is_some_and(|l| l.len() >= 2) && self.biome_data.is_some()
    }
}

impl Span {
    pub fn of<T>(self, items: &[T]) -> &[T] {
        &items[self.start as usize..][..self.len as usize]
    }

    fn since<T>(start: usize, items: &[T]) -> Self {
        Self { start: start as u32, len: (items.len() - start) as u32 }
    }
}

/// The compounds of `list` read by `read` (those it returns `None` for are skipped), then [`build`].
pub(super) fn build_from<'a>(
    list: Option<List<'a>>,
    read: impl Fn(Compound<'a>) -> Option<RawSection<'a>>,
    ctx: &ChunkContext,
) -> Sections {
    let mut raw = Vec::with_capacity(list.map_or(0, |l| l.len()));
    raw.extend(list.into_iter().flat_map(|l| l.compounds()).filter_map(read));
    build(&raw, ctx)
}

/// Decodes `raw` in order (a later section with the same y replaces an earlier one).
fn build(raw: &[RawSection], ctx: &ChunkContext) -> Sections {
    let Some(min) = raw.iter().map(|r| r.y).min() else { return Sections::default() };
    let max = raw.iter().map(|r| r.y).max().unwrap();
    let mut words = Vec::with_capacity(raw.iter().map(RawSection::word_count).sum());
    let mut palettes = Vec::with_capacity(raw.iter().map(RawSection::palette_len).sum());
    let mut light = Vec::with_capacity(raw.iter().map(RawSection::nibble_arrays).sum());
    let mut cells = Vec::with_capacity(raw.iter().filter(|r| r.has_biome_cells()).count());
    let mut sections: Vec<Option<Section>> = (min..=max).map(|_| None).collect();
    for r in raw {
        let blocks = blocks(r, ctx, &mut palettes, &mut words);
        let biomes = biomes(r, ctx, &mut cells);
        let mut light_of = |l: Option<RawLight>| {
            l.map(|l| match l {
                RawLight::Uniform(v) => Light::Uniform(v),
                RawLight::Nibbles(n) => {
                    light.push(*n);
                    Light::Nibbles(light.len() as u32 - 1)
                }
            })
        };
        let (block_light, sky_light) = (light_of(r.block_light), light_of(r.sky_light));
        sections[(r.y - min) as usize] = Some(Section { blocks, biomes, block_light, sky_light });
    }
    let arrays = Arrays {
        words: words.into_boxed_slice(),
        palettes: palettes.into_boxed_slice(),
        light: light.into_boxed_slice(),
        biomes: cells.into_boxed_slice(),
    };
    Sections { min, sections, arrays }
}

fn blocks(r: &RawSection, ctx: &ChunkContext, palettes: &mut Vec<StateId>, words: &mut Vec<u64>) -> Blocks {
    let start = palettes.len();
    if let Some(list) = r.palette {
        palettes.extend(list.iter().map(|t| palette_state(t, ctx)));
    }
    let palette = Span::since(start, palettes);
    match (palette.len, r.data) {
        (0, _) => Blocks::Single(StateId::AIR),
        (1, _) | (_, None) => Blocks::Single(palettes[start]),
        (n, Some(data)) => {
            let at = words.len();
            let indices = if r.spanning {
                respan_into(data, words)
            } else {
                words.extend(data.iter());
                Padded::new(padded_block_bits(n as usize, data.len()))
            };
            Blocks::Paletted { palette, words: Span::since(at, words), indices }
        }
    }
}

fn spanning_bits(data: BeArray<8>) -> usize {
    // exact for spanning arrays: `bits × 64` longs
    (data.len() >> 6).clamp(1, 32)
}

fn respan_into(data: BeArray<8>, words: &mut Vec<u64>) -> Padded {
    let bits = spanning_bits(data);
    let per_long = 64 / bits;
    let at = words.len();
    words.resize(at + 4096usize.div_ceil(per_long), 0);
    let packed = &mut words[at..];
    for i in 0..4096 {
        packed[i / per_long] |= (spanning_get(|l| data.get(l), bits as u32, i) as u64) << (i % per_long * bits);
    }
    Padded::new(bits as u8)
}

fn biomes(r: &RawSection, ctx: &ChunkContext, cells: &mut Vec<[BiomeId; 64]>) -> SectionBiomes {
    let Some(list) = r.biome_palette else { return SectionBiomes::Single(BiomeId::DEFAULT) };
    let resolve = |t: Tag| t.as_str().map_or(BiomeId::DEFAULT, |n| ctx.biomes.intern(n));
    let mut small = [BiomeId::DEFAULT; 64];
    let big: Vec<BiomeId>;
    // 64 cells index at most 64 entries of a valid palette; only corrupt ones are longer
    let palette: &[BiomeId] = if list.len() <= 64 {
        let mut n = 0;
        for t in list.iter() {
            small[n] = resolve(t);
            n += 1;
        }
        &small[..n]
    } else {
        big = list.iter().map(resolve).collect();
        &big
    };
    match (palette.len(), r.biome_data) {
        (0, _) => SectionBiomes::Single(BiomeId::DEFAULT),
        (1, _) | (_, None) => SectionBiomes::Single(palette[0]),
        (n, Some(data)) => {
            let indices = Padded::new(ceil_log2(n).max(1) as u8);
            cells.push(std::array::from_fn(|i| {
                palette.get(indices.get_with(|l| data.get(l), i) as usize).copied().unwrap_or(BiomeId::DEFAULT)
            }));
            SectionBiomes::Cells(cells.len() as u32 - 1)
        }
    }
}
