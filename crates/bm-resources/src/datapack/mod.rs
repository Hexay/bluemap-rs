//! Datapack resources BlueMap reads (docs/02 §6): worldgen biomes for tint colours and dimension types.

mod biome;
mod dimension;
pub(crate) mod gson;

pub use biome::{Biome, GrassColorModifier};
pub use dimension::dimension_type_from_json;

use bm_world::{BiomeId, Biomes, DimensionType};
use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::json::{self, JsonError};
use crate::{Pack, ResourcePath};

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error(transparent)]
    Json(#[from] JsonError),
    #[error("'{field}': expected {expected}")]
    Invalid { field: String, expected: &'static str },
}

/// A file upstream would skip with a debug log.
#[derive(Debug)]
pub struct LoadFailure {
    pub file: String,
    pub error: DataError,
}

/// Biomes and dimension types of every pack, first (highest-priority) definition wins.
#[derive(Clone, Debug, Default)]
pub struct DataPack {
    biomes: FxHashMap<ResourcePath, Biome>,
    dimension_types: FxHashMap<ResourcePath, DimensionType>,
}

impl DataPack {
    /// Loads `packs` (highest priority first) then [`DataPack::bake`]s.
    pub fn load(packs: &[Pack]) -> (Self, Vec<LoadFailure>) {
        let mut dp = Self::default();
        let mut failures = Vec::new();
        for pack in packs {
            dp.load_pack(pack, &mut failures);
        }
        dp.bake();
        (dp, failures)
    }

    pub fn load_pack(&mut self, pack: &Pack, failures: &mut Vec<LoadFailure>) {
        load_pool(pack, "dimension_type", 3, &mut self.dimension_types, dimension_type_from_json, failures);
        load_pool(pack, "worldgen/biome", 4, &mut self.biomes, Biome::from_json, failures);
    }

    /// Adds the built-in dimension types that no pack defined.
    pub fn bake(&mut self) {
        for (key, t) in [
            ("minecraft:overworld", DimensionType::OVERWORLD),
            ("minecraft:overworld_caves", DimensionType::OVERWORLD_CAVES),
            ("minecraft:the_nether", DimensionType::NETHER),
            ("minecraft:the_end", DimensionType::END),
        ] {
            self.dimension_types.entry(ResourcePath::key(key)).or_insert(t);
        }
    }

    pub fn biome(&self, key: &str) -> Option<&Biome> {
        self.biomes.get(&ResourcePath::key(key))
    }

    pub fn dimension_type(&self, key: &str) -> Option<&DimensionType> {
        self.dimension_types.get(&ResourcePath::key(key))
    }

    pub fn biome_count(&self) -> usize {
        self.biomes.len()
    }

    pub fn dimension_type_count(&self) -> usize {
        self.dimension_types.len()
    }

    /// Interns every known biome into `registry` and indexes them by id. Ids interned later (biomes no pack
    /// defines) read as [`Biome::DEFAULT`], as upstream's unknown-biome fallback.
    pub fn biome_table(&self, registry: &Biomes) -> BiomeTable {
        let mut by_id = Vec::new();
        for (key, biome) in &self.biomes {
            let id = registry.intern(key.as_str()).0 as usize;
            if by_id.len() <= id {
                by_id.resize(id + 1, Biome::DEFAULT);
            }
            by_id[id] = *biome;
        }
        BiomeTable { by_id }
    }
}

/// Biome colour data indexed by [`BiomeId`].
#[derive(Clone, Debug, Default)]
pub struct BiomeTable {
    by_id: Vec<Biome>,
}

impl BiomeTable {
    pub fn get(&self, id: BiomeId) -> &Biome {
        self.by_id.get(id.0 as usize).unwrap_or(&Biome::DEFAULT)
    }
}

/// `data/<ns>/<dir>/**.json` into `pool`, skipping keys already present (they are not even parsed).
fn load_pool<T>(
    pack: &Pack,
    dir: &str,
    value_segment: usize,
    pool: &mut FxHashMap<ResourcePath, T>,
    parse: fn(&Value) -> Result<T, DataError>,
    failures: &mut Vec<LoadFailure>,
) {
    for ns in pack.list("data") {
        for file in pack.walk(&format!("data/{ns}/{dir}")) {
            let Some(key) = file.strip_suffix(".json").and(ResourcePath::from_file(&file, 1, value_segment)) else {
                continue;
            };
            if pool.contains_key(&key) {
                continue;
            }
            let Some(src) = pack.read_string(&file) else { continue };
            match json::parse(&src).map_err(DataError::from).and_then(|v| parse(&v)) {
                Ok(t) => {
                    pool.insert(key, t);
                }
                Err(error) => failures.push(LoadFailure { file: format!("{}: {file}", pack.origin), error }),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
