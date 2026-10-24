//! 1.18+ chunks (DataVersion ≥ 2844, BlueMap's `Chunk_1_18`): flat root, `sections[].block_states` and per-section
//! biomes, padded index arrays.

use bm_nbt::{BeArray, Compound, List, Tag};

use super::sections::{RawLight, RawSection, build_from};
use super::{BlockEntity, Chunk, ChunkContext};
use crate::Result;
use crate::packed::{Padded, ceil_log2};
use crate::registry::StateId;

pub const MIN_DATA_VERSION: i32 = 2844;

pub fn parse(root: Compound, data_version: i32, ctx: &ChunkContext) -> Result<Chunk> {
    let (mut status, mut inhabited_time) = ("minecraft:empty", 0);
    let (mut heightmaps, mut sections, mut block_entities) = (None, None, None);
    for (name, tag) in root.entries() {
        match name {
            b"Status" => status = tag.as_str().unwrap_or(status),
            b"InhabitedTime" => inhabited_time = tag.as_i64().unwrap_or(0),
            b"Heightmaps" => heightmaps = tag.as_compound(),
            b"sections" => sections = tag.as_list(),
            b"block_entities" => block_entities = tag.as_list(),
            _ => {}
        }
    }
    let status = status.strip_prefix("minecraft:").unwrap_or(status);
    let dim = ctx.dimension;
    let heightmap = |key: &str| {
        let data = heightmaps?.get(key)?.as_long_array()?;
        let packed = Padded::new(ceil_log2(dim.height as usize + 1) as u8);
        packed
            .holds(data.len(), 256)
            .then(|| Box::new(std::array::from_fn(|i| (packed.get_with(|l| data.get(l), i) as i32 + dim.min_y) as i16)))
    };
    let sections = build_from(sections, raw_section, ctx);
    Ok(Chunk {
        data_version,
        generated: status != "empty",
        has_light: status == "full",
        inhabited_time,
        min_section: sections.min,
        sections: sections.sections,
        arrays: sections.arrays,
        legacy_biomes: None,
        sky_default: if dim.has_skylight { 15 } else { 0 },
        world_surface: heightmap("WORLD_SURFACE"),
        ocean_floor: heightmap("OCEAN_FLOOR"),
        block_entities: block_entities.map(parse_block_entities).unwrap_or_default(),
    })
}

fn raw_section(c: Compound) -> Option<RawSection> {
    let (mut y, mut block_states, mut biomes, mut block_light, mut sky_light) = (None, None, None, None, None);
    for (name, tag) in c.entries() {
        match name {
            b"Y" => y = tag.as_i64(),
            b"block_states" => block_states = tag.as_compound(),
            b"biomes" => biomes = tag.as_compound(),
            b"BlockLight" => block_light = tag.as_byte_array().and_then(RawLight::from_nibbles),
            b"SkyLight" => sky_light = tag.as_byte_array().and_then(RawLight::from_nibbles),
            _ => {}
        }
    }
    let (palette, data) = palette_and_data(block_states);
    let (biome_palette, biome_data) = palette_and_data(biomes);
    Some(RawSection { y: y? as i32, palette, data, spanning: false, biome_palette, biome_data, block_light, sky_light })
}

/// `palette` and `data` of a paletted container.
fn palette_and_data(c: Option<Compound>) -> (Option<List>, Option<BeArray<8>>) {
    let (mut palette, mut data) = (None, None);
    for (name, tag) in c.iter().flat_map(|c| c.entries()) {
        match name {
            b"palette" => palette = tag.as_list(),
            b"data" => data = tag.as_long_array(),
            _ => {}
        }
    }
    (palette, data)
}

/// A palette entry: `{Name, Properties}` (≤ 26.2), `{id, properties}` (26.3+), or a bare name / `{"": name}`
/// meaning the block's default state. Anything malformed is `MISSING` rather than failing the chunk.
pub(super) fn palette_state(tag: Tag, ctx: &ChunkContext) -> StateId {
    match ctx.palette {
        Some(cache) => cache.get_or(tag, || resolve_state(tag, ctx)),
        None => resolve_state(tag, ctx),
    }
}

fn resolve_state(tag: Tag, ctx: &ChunkContext) -> StateId {
    let c = match tag {
        Tag::String(_) => return tag.as_str().map_or(StateId::MISSING, |n| ctx.states.default_state(n)),
        Tag::Compound(c) => c,
        _ => return StateId::MISSING,
    };
    let (mut name, mut bare, mut props) = (None, None, None);
    for (key, value) in c.entries() {
        match key {
            b"Name" | b"id" => name = value.as_str(),
            b"" => bare = value.as_str(),
            b"Properties" | b"properties" => props = value.as_compound(),
            _ => {}
        }
    }
    match (name.or(bare), props) {
        (None, _) => StateId::MISSING,
        (Some(n), None) if bare.is_some() => ctx.states.default_state(n),
        (Some(n), props) => {
            let mut pairs: Vec<(&str, &str)> = props
                .into_iter()
                .flat_map(|p| p.entries())
                .filter_map(|(k, v)| Some((std::str::from_utf8(k).ok()?, v.as_str()?)))
                .collect();
            ctx.states.intern(n, &mut pairs)
        }
    }
}

pub(super) fn parse_block_entities(list: List) -> Vec<BlockEntity> {
    list.compounds()
        .filter_map(|c| {
            let (mut id, mut x, mut y, mut z) = (None, None, None, None);
            for (key, value) in c.entries() {
                match key {
                    b"id" => id = value.as_str(),
                    b"x" => x = value.as_i64(),
                    b"y" => y = value.as_i64(),
                    b"z" => z = value.as_i64(),
                    _ => {}
                }
            }
            Some(BlockEntity { id: id?.into(), x: x? as i32, y: y? as i32, z: z? as i32 })
        })
        .collect()
}
