//! Everything loaded from the ordered packs and baked for rendering: BlueMap's `ResourcePack` and `DataPack`
//! combined. Every pool is first-loaded-wins over the packs in priority order (docs/02 §1).

use bm_world::BlockState;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::blockstate::{BlockStateDef, Variant};
use crate::color::{BlockColors, BlockColorsConfig, BlockProperties, BlockPropertiesConfig, ColorMaps, ModelProperties};
use crate::datapack::DataPack;
use crate::model::{BakedModel, BakedModels, ModelLibrary};
use crate::texture::{Atlas, TexturePool, load_textures};
use crate::{Pack, ResourcePath};

pub struct ResourcePack {
    blockstates: FxHashMap<ResourcePath, BlockStateDef>,
    pub models: BakedModels,
    pub textures: TexturePool,
    pub block_colors: BlockColors,
    block_properties: BlockPropertiesConfig,
    pub datapack: DataPack,
    /// One line per file that failed to load; BlueMap logs these at debug level and carries on.
    pub failures: Vec<String>,
}

impl ResourcePack {
    /// `resource_packs` and `data_packs` highest priority first, as [`crate::packs::load_order`] returns them.
    pub fn load(resource_packs: &[Pack], data_packs: &[Pack]) -> Self {
        let mut failures = Vec::new();
        let blockstates = load_blockstates(resource_packs, &mut failures);

        let mut library = ModelLibrary::new();
        resource_packs.iter().for_each(|p| library.load_pack(p));
        failures.extend(library.failures().iter().map(|(key, e)| format!("model {key}: {e}")));
        library.merge_parents();
        let used = library.collect_used_texture_keys();
        let textures = load_textures(resource_packs, &Atlas::load_blocks(resource_packs), &|k| used.contains(k));
        let models = library.bake(&|k| textures.get(k).map(|t| t.color.a));

        let (mut colormaps, mut colors, mut properties) =
            (ColorMaps::default(), BlockColorsConfig::default(), BlockPropertiesConfig::default());
        let mut config_failures = Vec::new();
        for pack in resource_packs {
            colormaps.load_pack(pack, &mut config_failures);
            colors.load_pack(pack, &mut config_failures);
            properties.load_pack(pack, &mut config_failures);
        }
        failures.extend(config_failures.iter().map(|f| format!("{f:?}")));
        let (datapack, data_failures) = DataPack::load(data_packs);
        failures.extend(data_failures.iter().map(|f| format!("{f:?}")));

        Self {
            blockstates,
            models,
            textures,
            block_colors: colors.bake(&colormaps),
            block_properties: properties,
            datapack,
            failures,
        }
    }

    /// The blockstate file for a block, e.g. `minecraft:oak_stairs`.
    pub fn blockstate(&self, state: &BlockState) -> Option<&BlockStateDef> {
        self.blockstates.get(&ResourcePath::key(&state.name))
    }

    /// The variants BlueMap renders for `state` at a position (weighted picks depend on it).
    pub fn variants<'a>(&'a self, state: &'a BlockState, x: i32, y: i32, z: i32) -> impl Iterator<Item = &'a Variant> + 'a {
        self.blockstate(state).into_iter().flat_map(move |def| def.variants_at(state, x, y, z))
    }

    pub fn model(&self, key: &ResourcePath) -> Option<&BakedModel> {
        self.models.models.get(key)
    }

    /// Config properties, with undefined culling/occluding taken from the models at (0,0,0) (`ResourcePack.java`).
    pub fn block_properties(&self, state: &BlockState) -> BlockProperties {
        self.block_properties.resolve(state, || {
            self.variants(state, 0, 0, 0)
                .filter_map(|v| self.model(&v.model))
                .map(|m| ModelProperties { culling: m.culling, occluding: m.occluding })
                .collect::<Vec<_>>()
        })
    }

    pub fn blockstate_count(&self) -> usize {
        self.blockstates.len()
    }
}

fn load_blockstates(packs: &[Pack], failures: &mut Vec<String>) -> FxHashMap<ResourcePath, BlockStateDef> {
    let mut defs = FxHashMap::default();
    for pack in packs {
        let files: Vec<String> = pack
            .list("assets")
            .iter()
            .flat_map(|ns| pack.walk(&format!("assets/{ns}/blockstates")))
            .filter(|f| f.ends_with(".json"))
            .filter(|f| ResourcePath::from_file(f, 1, 3).is_some_and(|k| !defs.contains_key(&k)))
            .collect();
        let parsed: Vec<(ResourcePath, Result<BlockStateDef, String>)> = files
            .par_iter()
            .map(|f| {
                let key = ResourcePath::from_file(f, 1, 3).expect("filtered above");
                let def = pack
                    .read_string(f)
                    .ok_or_else(|| "unreadable".to_owned())
                    .and_then(|src| crate::json::parse(&src).map_err(|e| e.to_string()))
                    .and_then(|json| BlockStateDef::from_json(&json).map_err(|e| e.to_string()));
                (key, def)
            })
            .collect();
        for (key, def) in parsed {
            match def {
                Ok(def) => {
                    defs.insert(key, def);
                }
                Err(e) => failures.push(format!("blockstate {key} ({}): {e}", pack.origin)),
            }
        }
    }
    defs
}
