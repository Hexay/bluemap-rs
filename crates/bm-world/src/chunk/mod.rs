//! A decoded chunk: blocks, biomes and light as dense ids per section. Coordinates passed to accessors are block
//! coordinates; only their low 4 bits are used for x and z.

mod legacy;
mod legacy_biomes;
mod modern;
mod palette;
mod sections;

pub use palette::PaletteCache;

use crate::dimension::DimensionType;
use crate::packed::Padded;
use crate::registry::{BiomeId, Biomes, BlockStates, StateId};
use crate::{Error, Result};
use sections::{Arrays, Span};

/// Everything a chunk needs from outside to decode.
pub struct ChunkContext<'a> {
    pub states: &'a BlockStates,
    pub biomes: &'a Biomes,
    pub dimension: &'a DimensionType,
    /// Resolved palette entries to reuse, if the caller keeps them.
    pub palette: Option<&'a PaletteCache>,
}

pub struct Chunk {
    pub data_version: i32,
    /// Status isn't `empty`.
    pub generated: bool,
    /// Status is `full`; otherwise light reads as open sky (BlueMap renders these as `MISSING_LIGHT` tiles).
    pub has_light: bool,
    pub inhabited_time: i64,
    /// Section y of `sections[0]`.
    min_section: i32,
    /// Contiguous from `min_section`; `None` for gaps.
    sections: Vec<Option<Section>>,
    arrays: Arrays,
    /// 1.13–1.17 store biomes for the whole chunk, not per section.
    legacy_biomes: Option<LegacyBiomes>,
    /// Light returned where nothing is stored: 15 with sky light, else 0.
    sky_default: u8,
    world_surface: Option<Box<[i16; 256]>>,
    ocean_floor: Option<Box<[i16; 256]>>,
    pub block_entities: Vec<BlockEntity>,
}

/// Indices into the chunk's [`Arrays`].
struct Section {
    blocks: Blocks,
    biomes: SectionBiomes,
    block_light: Option<Light>,
    sky_light: Option<Light>,
}

enum Blocks {
    Single(StateId),
    Paletted { palette: Span, words: Span, indices: Padded },
}

enum SectionBiomes {
    Single(BiomeId),
    /// 4×4×4 cells, index `(y/4)*16 + (z/4)*4 + x/4`.
    Cells(u32),
}

enum LegacyBiomes {
    /// 1.13–1.14: one id per block column, index `z*16 + x`.
    Columns(Box<[BiomeId; 256]>),
    /// 1.15–1.17 as BlueMap reads them: 64 cells repeated every 16 blocks of height.
    Cells(Box<[BiomeId; 64]>),
}

enum Light {
    Uniform(u8),
    Nibbles(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEntity {
    pub id: Box<str>,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Chunk {
    /// Decodes chunk NBT of any supported version.
    pub fn parse(nbt: &[u8], ctx: &ChunkContext) -> Result<Self> {
        let doc = bm_nbt::Document::parse(nbt)?;
        let root = doc.root();
        let data_version = root.i64("DataVersion").unwrap_or(0) as i32;
        if data_version >= modern::MIN_DATA_VERSION {
            modern::parse(root, data_version, ctx)
        } else if data_version >= legacy::MIN_DATA_VERSION {
            legacy::parse(root, data_version, ctx)
        } else {
            Err(Error::UnsupportedVersion(data_version))
        }
    }

    /// The section holding `y`, or its index relative to `min_section` when there is none (negative = below).
    fn section(&self, y: i32) -> std::result::Result<&Section, i32> {
        let i = (y >> 4) - self.min_section;
        match usize::try_from(i).ok().and_then(|i| self.sections.get(i)) {
            Some(Some(s)) => Ok(s),
            _ => Err(i),
        }
    }

    pub fn block(&self, x: i32, y: i32, z: i32) -> StateId {
        self.section(y).map_or(StateId::AIR, |s| match s.blocks {
            Blocks::Single(s) => s,
            Blocks::Paletted { palette, words, indices } => {
                let p = indices.get(words.of(&self.arrays.words), block_index(x, y, z));
                palette.of(&self.arrays.palettes).get(p as usize).copied().unwrap_or(StateId::MISSING)
            }
        })
    }

    pub fn biome(&self, x: i32, y: i32, z: i32) -> BiomeId {
        let cell = ((y & 12) << 2 | z & 12 | (x & 12) >> 2) as usize;
        match &self.legacy_biomes {
            Some(LegacyBiomes::Columns(ids)) => return ids[column_index(x, z)],
            Some(LegacyBiomes::Cells(cells)) => return cells[cell],
            None => {}
        }
        self.section(y).map_or(BiomeId::DEFAULT, |s| match s.biomes {
            SectionBiomes::Single(b) => b,
            SectionBiomes::Cells(k) => self.arrays.biomes[k as usize][cell],
        })
    }

    /// Whether [`Chunk::biome`] is constant over every aligned 4×4×4 cell: all formats but 1.13–1.14's per-column ids.
    pub fn biome_cells(&self) -> bool {
        !matches!(self.legacy_biomes, Some(LegacyBiomes::Columns(_)))
    }

    /// `(sky, block)` light, with BlueMap's fallbacks: open sky without light data, dark below the lowest section.
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        if !self.has_light {
            return (self.sky_default, 0);
        }
        match self.section(y) {
            Ok(s) if s.block_light.is_none() && s.sky_light.is_none() => (0, 0),
            Ok(s) => {
                let i = block_index(x, y, z);
                let get = |l: &Option<Light>| l.as_ref().map_or(0, |l| l.get(&self.arrays, i));
                (get(&s.sky_light), get(&s.block_light))
            }
            Err(below) if below < 0 => (0, 0),
            Err(_) => (self.sky_default, 0),
        }
    }

    /// [`Chunk::block`] and [`Chunk::light`] (as `[sky, block]`) of column `x, z` for every y in `y0..=y1`,
    /// appended: one section lookup per 16 blocks, uniform runs filled in bulk.
    pub fn column_into(&self, x: i32, z: i32, y0: i32, y1: i32, states: &mut Vec<StateId>, light: &mut Vec<[u8; 2]>) {
        let mut y = y0;
        while y <= y1 {
            let end = (y | 15).min(y1);
            let n = (end - y + 1) as usize;
            let section = self.section(y);
            match section {
                Ok(Section { blocks: Blocks::Single(s), .. }) => states.extend(std::iter::repeat_n(*s, n)),
                Ok(Section { blocks: Blocks::Paletted { palette, words, indices }, .. }) => {
                    let palette = palette.of(&self.arrays.palettes);
                    states.extend(
                        indices
                            .strided(words.of(&self.arrays.words), block_index(x, y, z), 256, n)
                            .map(|p| palette.get(p as usize).copied().unwrap_or(StateId::MISSING)),
                    )
                }
                Err(_) => states.extend(std::iter::repeat_n(StateId::AIR, n)),
            }
            let uniform = match section {
                _ if !self.has_light => Some([self.sky_default, 0]),
                Ok(s) => Light::uniform(&s.sky_light).zip(Light::uniform(&s.block_light)).map(|(s, b)| [s, b]),
                Err(below) if below < 0 => Some([0, 0]),
                Err(_) => Some([self.sky_default, 0]),
            };
            match (uniform, section) {
                (Some(l), _) => light.extend(std::iter::repeat_n(l, n)),
                (None, Ok(s)) => {
                    let i = block_index(x, y, z);
                    let column = |l: &Option<Light>| Light::column(l, &self.arrays, i, n);
                    let (sky, block) = (column(&s.sky_light), column(&s.block_light));
                    light.extend(sky[..n].iter().zip(&block[..n]).map(|(&s, &b)| [s, b]));
                }
                (None, Err(_)) => unreachable!("absent sections have uniform light"),
            }
            y = end + 1;
        }
    }

    /// Top y of the highest section holding anything but one state `is_air` accepts; every block above it is air.
    pub fn top_y(&self, is_air: impl Fn(StateId) -> bool) -> Option<i32> {
        let solid =
            |s: &Option<Section>| s.as_ref().is_some_and(|s| !matches!(s.blocks, Blocks::Single(id) if is_air(id)));
        let i = self.sections.iter().rposition(solid)?;
        Some((self.min_section + i as i32) * 16 + 15)
    }

    /// The highest [`Chunk::light`] block light of column `x, z` over `y0..=y1`.
    pub fn max_block_light(&self, x: i32, z: i32, y0: i32, y1: i32) -> u8 {
        if !self.has_light {
            return 0;
        }
        let mut max = 0;
        let mut y = y0;
        while y <= y1 {
            let end = (y | 15).min(y1);
            max = max.max(match self.section(y).map(|s| &s.block_light) {
                Ok(Some(Light::Uniform(v))) => *v,
                Ok(Some(l)) => (y..=end).map(|y| l.get(&self.arrays, block_index(x, y, z))).max().unwrap_or(0),
                Ok(None) | Err(_) => 0,
            });
            y = end + 1;
        }
        max
    }

    /// Lowest block y with a stored section (light-only padding sections included, as BlueMap).
    pub fn min_y(&self) -> i32 {
        self.min_section * 16
    }

    pub fn max_y(&self) -> i32 {
        (self.min_section + self.sections.len() as i32) * 16 - 1
    }

    /// Y of the first air block above the top non-air block, if the chunk has a valid heightmap.
    pub fn world_surface_y(&self, x: i32, z: i32) -> Option<i32> {
        self.world_surface.as_ref().map(|h| h[column_index(x, z)] as i32)
    }

    /// Like [`Chunk::world_surface_y`] but ignoring non-solid blocks; drives BlueMap's cave removal.
    pub fn ocean_floor_y(&self, x: i32, z: i32) -> Option<i32> {
        self.ocean_floor.as_ref().map(|h| h[column_index(x, z)] as i32)
    }
}

impl Light {
    fn get(&self, arrays: &Arrays, i: usize) -> u8 {
        match self {
            Self::Uniform(v) => *v,
            Self::Nibbles(k) => (arrays.light[*k as usize][i >> 1] >> ((i & 1) * 4)) & 15,
        }
    }

    /// Values `start`, `start + 256`, … (`n` ≤ 16 of them: one column within the section); absent light reads as 0.
    fn column(light: &Option<Self>, arrays: &Arrays, start: usize, n: usize) -> [u8; 16] {
        let mut out = [0; 16];
        match light {
            None => {}
            Some(Self::Uniform(v)) => out = [*v; 16],
            Some(Self::Nibbles(k)) => {
                let nibbles = &arrays.light[*k as usize];
                // a 256 stride keeps the nibble half fixed: 128 bytes apart
                let shift = (start & 1) * 4;
                for (j, o) in out[..n].iter_mut().enumerate() {
                    *o = (nibbles[(start >> 1) + 128 * j] >> shift) & 15;
                }
            }
        }
        out
    }

    /// The one value of a section's light, if it has one; absent light reads as 0.
    fn uniform(light: &Option<Self>) -> Option<u8> {
        match light {
            None => Some(0),
            Some(Self::Uniform(v)) => Some(*v),
            Some(Self::Nibbles(_)) => None,
        }
    }
}

fn block_index(x: i32, y: i32, z: i32) -> usize {
    ((y & 15) << 8 | (z & 15) << 4 | x & 15) as usize
}

fn column_index(x: i32, z: i32) -> usize {
    ((z & 15) << 4 | x & 15) as usize
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
