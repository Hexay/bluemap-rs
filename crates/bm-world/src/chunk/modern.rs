//! 1.18+ chunks (DataVersion ≥ 2844, BlueMap's `Chunk_1_18`): flat root, `sections[].block_states` and per-section
//! biomes, padded index arrays.

use bm_nbt::{Compound, List, Tag};

use super::{BlockEntity, Blocks, Chunk, ChunkContext, Light, Section, SectionBiomes};
use crate::Result;
use crate::packed::{Padded, ceil_log2, padded_block_bits};
use crate::registry::{BiomeId, StateId};

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
        let bits = ceil_log2(dim.height as usize + 1) as u8;
        let packed = Padded::new(bits, data.iter().collect());
        packed.holds(256).then(|| Box::new(std::array::from_fn(|i| (packed.get(i) as i32 + dim.min_y) as i16)))
    };
    let (min_section, sections) =
        collect_sections(sections.into_iter().flat_map(|l| l.compounds()).filter_map(|c| parse_section(c, ctx)));
    Ok(Chunk {
        data_version,
        generated: status != "empty",
        has_light: status == "full",
        inhabited_time,
        min_section,
        sections,
        legacy_biomes: None,
        sky_default: if dim.has_skylight { 15 } else { 0 },
        world_surface: heightmap("WORLD_SURFACE"),
        ocean_floor: heightmap("OCEAN_FLOOR"),
        block_entities: block_entities.map(parse_block_entities).unwrap_or_default(),
    })
}

/// Sections by y, contiguous from the lowest one, gaps `None`.
pub(super) fn collect_sections(parsed: impl Iterator<Item = (i32, Section)>) -> (i32, Vec<Option<Section>>) {
    let parsed: Vec<(i32, Section)> = parsed.collect();
    let Some(min) = parsed.iter().map(|(y, _)| *y).min() else { return (0, Vec::new()) };
    let max = parsed.iter().map(|(y, _)| *y).max().unwrap();
    let mut sections: Vec<Option<Section>> = (min..=max).map(|_| None).collect();
    for (y, section) in parsed {
        sections[(y - min) as usize] = Some(section);
    }
    (min, sections)
}

fn parse_section(c: Compound, ctx: &ChunkContext) -> Option<(i32, Section)> {
    let (mut y, mut block_states, mut biomes, mut block_light, mut sky_light) = (None, None, None, None, None);
    for (name, tag) in c.entries() {
        match name {
            b"Y" => y = tag.as_i64(),
            b"block_states" => block_states = tag.as_compound(),
            b"biomes" => biomes = tag.as_compound(),
            b"BlockLight" => block_light = tag.as_byte_array().and_then(Light::from_nibbles),
            b"SkyLight" => sky_light = tag.as_byte_array().and_then(Light::from_nibbles),
            _ => {}
        }
    }
    let section = Section {
        blocks: block_states.map_or(Blocks::Single(StateId::AIR), |b| parse_blocks(b, ctx)),
        biomes: biomes.map_or(SectionBiomes::Single(BiomeId::DEFAULT), |b| parse_biomes(b, ctx)),
        block_light,
        sky_light,
    };
    Some((y? as i32, section))
}

fn parse_blocks(c: Compound, ctx: &ChunkContext) -> Blocks {
    let palette: Box<[StateId]> =
        c.list("palette").map(|l| l.iter().map(|t| palette_state(t, ctx)).collect()).unwrap_or_default();
    match (palette.len(), c.get("data").and_then(|t| t.as_long_array())) {
        (0, _) => Blocks::Single(StateId::AIR),
        (1, _) | (_, None) => Blocks::Single(palette[0]),
        (n, Some(data)) => {
            let bits = padded_block_bits(n, data.len());
            Blocks::Paletted { palette, indices: Padded::new(bits, data.iter().collect()) }
        }
    }
}

/// A palette entry: `{Name, Properties}` (≤ 26.2), `{id, properties}` (26.3+), or a bare name / `{"": name}`
/// meaning the block's default state. Anything malformed is `MISSING` rather than failing the chunk.
pub(super) fn palette_state(tag: Tag, ctx: &ChunkContext) -> StateId {
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

fn parse_biomes(c: Compound, ctx: &ChunkContext) -> SectionBiomes {
    let palette: Vec<BiomeId> = c
        .list("palette")
        .map(|l| l.iter().map(|t| t.as_str().map_or(BiomeId::DEFAULT, |n| ctx.biomes.intern(n))).collect())
        .unwrap_or_default();
    match (palette.len(), c.get("data").and_then(|t| t.as_long_array())) {
        (0, _) => SectionBiomes::Single(BiomeId::DEFAULT),
        (1, _) | (_, None) => SectionBiomes::Single(palette[0]),
        (n, Some(data)) => {
            let indices = Padded::new(ceil_log2(n).max(1) as u8, data.iter().collect());
            let cells =
                std::array::from_fn(|i| palette.get(indices.get(i) as usize).copied().unwrap_or(BiomeId::DEFAULT));
            SectionBiomes::Cells(Box::new(cells))
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
