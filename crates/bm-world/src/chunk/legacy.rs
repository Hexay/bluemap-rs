//! 1.13–1.17 chunks (BlueMap's `Chunk_1_13`, `_1_15`, `_1_16`): everything under `Level`, numeric biomes for the
//! whole chunk, index arrays spanning longs before 1.16 and padded from 1.16.

use bm_nbt::{BeArray, Compound};

use super::modern::{collect_sections, palette_state, parse_block_entities};
use super::{Blocks, Chunk, ChunkContext, LegacyBiomes, Light, Section, SectionBiomes, legacy_biomes};
use crate::packed::{Padded, ceil_log2, padded_block_bits, spanning_get};
use crate::registry::{BiomeId, StateId};
use crate::{Error, Result};

/// 17w47a, the first snapshot with block palettes; older chunks hold numeric block ids BlueMap never supported.
pub const MIN_DATA_VERSION: i32 = 1451;
const V1_15: i32 = 2200;
const V1_16: i32 = 2500;

pub fn parse(root: Compound, data_version: i32, ctx: &ChunkContext) -> Result<Chunk> {
    let level = root.compound("Level").ok_or(Error::Corrupt("chunk without a Level compound"))?;
    let (mut status, mut inhabited_time) = ("minecraft:empty", 0);
    let (mut heightmaps, mut sections, mut biomes, mut tile_entities) = (None, None, None, None);
    for (name, tag) in level.entries() {
        match name {
            b"Status" => status = tag.as_str().unwrap_or(status),
            b"InhabitedTime" => inhabited_time = tag.as_i64().unwrap_or(0),
            b"Heightmaps" => heightmaps = tag.as_compound(),
            b"Sections" => sections = tag.as_list(),
            b"Biomes" => biomes = tag.as_int_array(),
            b"TileEntities" => tile_entities = tag.as_list(),
            _ => {}
        }
    }
    let status = status.strip_prefix("minecraft:").unwrap_or(status);
    let has_light = status == "full" || (data_version < V1_16 && matches!(status, "fullchunk" | "postprocessed"));
    let padded = data_version >= V1_16;
    let heightmap = |key: &str| -> Option<Box<[i16; 256]>> {
        let data: Box<[u64]> = heightmaps?.get(key)?.as_long_array()?.iter().collect();
        if padded {
            let packed = Padded::new(ceil_log2(ctx.dimension.height as usize + 1) as u8, data);
            packed.holds(256).then(|| Box::new(std::array::from_fn(|i| packed.get(i) as i16)))
        } else {
            (data.len() >= 36).then(|| Box::new(std::array::from_fn(|i| spanning_get(&data, 9, i) as i16)))
        }
    };
    let (min_section, sections) = collect_sections(
        sections.into_iter().flat_map(|l| l.compounds()).filter_map(|c| parse_section(c, padded, ctx)),
    );
    Ok(Chunk {
        data_version,
        generated: status != "empty",
        has_light,
        inhabited_time,
        min_section,
        sections,
        legacy_biomes: biomes.and_then(|b| parse_biomes(b, data_version >= V1_15, ctx)),
        sky_default: if ctx.dimension.has_skylight { 15 } else { 0 },
        world_surface: heightmap("WORLD_SURFACE"),
        ocean_floor: heightmap("OCEAN_FLOOR"),
        block_entities: tile_entities.map(parse_block_entities).unwrap_or_default(),
    })
}

fn parse_section(c: Compound, padded: bool, ctx: &ChunkContext) -> Option<(i32, Section)> {
    let (mut y, mut palette, mut data, mut block_light, mut sky_light) = (None, None, None, None, None);
    for (name, tag) in c.entries() {
        match name {
            b"Y" => y = tag.as_i64(),
            b"Palette" => palette = tag.as_list(),
            b"BlockStates" => data = tag.as_long_array(),
            b"BlockLight" => block_light = tag.as_byte_array().and_then(Light::from_nibbles),
            b"SkyLight" => sky_light = tag.as_byte_array().and_then(Light::from_nibbles),
            _ => {}
        }
    }
    let palette: Box<[StateId]> =
        palette.map(|l| l.iter().map(|t| palette_state(t, ctx)).collect()).unwrap_or_default();
    let blocks = match (palette.len(), data) {
        (0, _) => Blocks::Single(StateId::AIR),
        (1, _) | (_, None) => Blocks::Single(palette[0]),
        (n, Some(data)) if padded => Blocks::Paletted { palette, indices: padded_indices(n, data) },
        (_, Some(data)) => Blocks::Paletted { palette, indices: respan(data) },
    };
    let section = Section { blocks, biomes: SectionBiomes::Single(BiomeId::DEFAULT), block_light, sky_light };
    Some((y? as i32, section))
}

fn padded_indices(palette_len: usize, data: BeArray<8>) -> Padded {
    Padded::new(padded_block_bits(palette_len, data.len()), data.iter().collect())
}

/// Spanning indices re-packed padded, so lookups share one code path. Bits come from the array length, which is
/// exact for spanning arrays (`bits × 64` longs).
fn respan(data: BeArray<8>) -> Padded {
    let longs: Vec<u64> = data.iter().collect();
    let bits = (longs.len() >> 6).clamp(1, 32) as u32;
    let per_long = (64 / bits) as usize;
    let mut packed = vec![0u64; 4096usize.div_ceil(per_long)];
    for i in 0..4096 {
        packed[i / per_long] |= (spanning_get(&longs, bits, i) as u64) << ((i % per_long) as u32 * bits);
    }
    Padded::new(bits as u8, packed.into())
}

/// 1.13–1.14: 256 ids per block column. 1.15–1.17: 4×4×4 cells, of which BlueMap only ever reads the lowest 64
/// (its index uses `y & 12`), repeating them every 16 blocks; kept for tint parity.
fn parse_biomes(ids: BeArray<4>, cells: bool, ctx: &ChunkContext) -> Option<LegacyBiomes> {
    let biome = |id: i32| legacy_biomes::name(id).map_or(BiomeId::DEFAULT, |n| ctx.biomes.intern(n));
    if !cells {
        return (ids.len() >= 256)
            .then(|| LegacyBiomes::Columns(Box::new(std::array::from_fn(|i| biome(ids.get(i).unwrap())))));
    }
    let len = ids.len() as i32;
    (len >= 16).then(|| {
        LegacyBiomes::Cells(Box::new(std::array::from_fn(|i| {
            let mut i = i as i32;
            if i >= len {
                i -= (((i - len) >> 4) + 1) * 16;
            }
            biome(ids.get(i as usize).unwrap())
        })))
    })
}
