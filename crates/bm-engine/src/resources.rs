//! `BlueMapService.getOrLoadResourcePack` / `loadDataPack`: the client jar, pack layering and the baked resource
//! pack every map renders with, plus the block-state and biome registries all worlds share.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bm_config::BlueMapConfig;
use bm_resources::datapack::DataPack;
use bm_resources::packs::{OpenedRoots, PackRootsConfig, load_order, load_order_in, pack_roots};
use bm_resources::resource_pack::ResourcePack;
use bm_resources::{MinecraftVersion, Pack};
use bm_world::{Biomes, BlockStates};

use crate::error::{Error, Result, io};

/// BlueMap's bundled pack, written to `<data>/resourceExtensions.zip` on every load like upstream.
const RESOURCE_EXTENSIONS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/resourceExtensions.zip"));

/// Where packs come from besides the configs (`BlueMapConfigManager` paths, CLI `-n`, `-v`).
#[derive(Clone, Debug, Default)]
pub struct ResourceOptions {
    /// `None`: latest release.
    pub minecraft_version: Option<String>,
    pub packs_folder: Option<PathBuf>,
    pub mods_folder: Option<PathBuf>,
}

pub struct Resources {
    pub minecraft: MinecraftVersion,
    pub pack: ResourcePack,
    pub states: Arc<BlockStates>,
    pub biomes: Arc<Biomes>,
    roots: PackRootsConfig,
}

impl Resources {
    pub fn load(config: &BlueMapConfig, options: &ResourceOptions) -> Result<Self> {
        let data = &config.core.data;
        std::fs::create_dir_all(data).map_err(io("create", data))?;
        let minecraft =
            MinecraftVersion::load(options.minecraft_version.as_deref(), data, config.core.accept_download)
                .map_err(|e| match e {
                    e @ bm_resources::Error::DownloadNotAccepted(_) => Error::MissingResources(e),
                    e => Error::Resources(e),
                })?;
        if let Some(packs) = &options.packs_folder {
            std::fs::create_dir_all(packs).map_err(io("create", packs))?;
        }
        let roots = PackRootsConfig {
            packs_folder: options.packs_folder.clone(),
            mods_folder: options.mods_folder.clone(),
            scan_for_mod_resources: config.core.scan_for_mod_resources,
            data_root: data.clone(),
            resource_extensions: write_resource_extensions(data)?,
        };
        let resource_roots = pack_roots(&roots, &[], &minecraft.resource_pack).map_err(io("list packs in", data))?;
        let mut opened = OpenedRoots::default();
        let resource_packs = load_order_in(&mut opened, &resource_roots, minecraft.resource_pack_version);
        let data_roots = pack_roots(&roots, &[], &minecraft.data_pack).map_err(io("list packs in", data))?;
        let data_packs = load_order_in(&mut opened, &data_roots, minecraft.data_pack_version);
        let pack = ResourcePack::load(&resource_packs, &data_packs);

        let states = Arc::new(BlockStates::default());
        load_default_states(&states, &resource_packs);
        Ok(Self { minecraft, pack, states, biomes: Arc::new(Biomes::default()), roots })
    }

    /// The datapack of a world: the shared one, or a fresh load when the world has a `datapacks/` folder.
    pub fn world_datapack(&self, world: &Path) -> Result<DataPack> {
        let folder = world.join("datapacks");
        if !folder.is_dir() {
            return Ok(self.pack.datapack.clone());
        }
        let mut world_packs: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map_err(io("list", &folder))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        world_packs.sort();
        // no extra roots: the shared pack roots, so the shared datapack (reloading costs ~60 ms, mostly the jar)
        if world_packs.is_empty() {
            return Ok(self.pack.datapack.clone());
        }
        let roots = pack_roots(&self.roots, &world_packs, &self.minecraft.data_pack).map_err(io("list", &folder))?;
        Ok(DataPack::load(&load_order(&roots, self.minecraft.data_pack_version)).0)
    }
}

fn write_resource_extensions(data: &Path) -> Result<PathBuf> {
    let file = data.join("resourceExtensions.zip");
    std::fs::write(&file, RESOURCE_EXTENSIONS).map_err(io("write", &file))?;
    Ok(file)
}

/// `data/<namespace>/defaultBlockstates.json` of every pack; higher-priority packs come first and win.
fn load_default_states(states: &BlockStates, packs: &[Pack]) {
    for pack in packs {
        for ns in pack.list("data") {
            if let Some(json) = pack.read_string(&format!("data/{ns}/defaultBlockstates.json")) {
                // a malformed file is skipped, as upstream only logs it
                let _ = states.add_defaults_json(&json);
            }
        }
    }
}
