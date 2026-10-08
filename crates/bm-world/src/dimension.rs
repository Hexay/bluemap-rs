//! Dimensions: folder layout and dimension types (`W/DimensionType.java`, `MCAWorld.java:174-224`).

use std::path::{Path, PathBuf};

use bm_compress::Compression;
use bm_nbt::{Compound, Tag};

use crate::Result;

const DAT_LIMIT: usize = 64 << 20;
const DAT_RETRIES: u32 = 50;
const DAT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

/// Where a dimension's `region/` lives: `dimensions/<ns>/<path>` (26.1+ and modded), else the legacy layout
/// (world root, `DIM-1`, `DIM1`) if it has a `region/` folder, else the new path (it may be created later).
pub fn dimension_folder(world: &Path, dimension: &str) -> PathBuf {
    let (ns, path) = dimension.split_once(':').filter(|(ns, _)| !ns.is_empty()).unwrap_or(("minecraft", dimension));
    let modern = world.join("dimensions").join(ns).join(path);
    if modern.is_dir() {
        return modern;
    }
    let legacy = match (ns, path) {
        ("minecraft", "overworld") => world.to_owned(),
        ("minecraft", "the_nether") => world.join("DIM-1"),
        ("minecraft", "the_end") => world.join("DIM1"),
        _ => return modern,
    };
    if legacy.join("region").is_dir() { legacy } else { modern }
}

/// The dimension's type from `world_gen_settings.dat` (dimension folder, then world root) or `level.dat`.
/// `datapack` resolves type references like `minecraft:overworld`; unresolved ones fall back to the overworld and
/// unknown dimensions to the built-ins, as BlueMap.
pub fn load_dimension_type(
    world: &Path,
    dimension: &str,
    datapack: &dyn Fn(&str) -> Option<DimensionType>,
) -> Result<DimensionType> {
    let key = if dimension.contains(':') { dimension.to_owned() } else { format!("minecraft:{dimension}") };
    let folder = dimension_folder(world, dimension);
    let sources = [
        (folder.join("data/minecraft/world_gen_settings.dat"), &["data"][..]),
        (world.join("data/minecraft/world_gen_settings.dat"), &["data"][..]),
        (world.join("level.dat"), &["Data", "WorldGenSettings"][..]),
    ];
    for (file, path) in sources {
        let Some(nbt) = read_dat(&file)? else { continue };
        let root = bm_nbt::read_root(&nbt)?;
        let settings = path.iter().try_fold(root, |c, name| c.compound(name));
        if let Some(ty) = settings.and_then(|s| s.compound("dimensions")?.compound(&key)) {
            return Ok(match ty.get("type") {
                Some(Tag::Compound(c)) => DimensionType::from_nbt(c),
                Some(t) => t.as_str().and_then(datapack).unwrap_or(DimensionType::OVERWORLD),
                None => DimensionType::OVERWORLD,
            });
        }
    }
    Ok(DimensionType::builtin(&key).filter(|_| key != "minecraft:overworld_caves").unwrap_or(DimensionType::OVERWORLD))
}

fn read_dat(path: &Path) -> Result<Option<Vec<u8>>> {
    // a server can be rewriting it while a plugin core loads (Paper saves level data off-thread at enable, seconds under load)
    for _ in 0..DAT_RETRIES {
        match std::fs::read(path) {
            Ok(bytes) => match Compression::Gzip.decompress(&bytes, DAT_LIMIT) {
                Ok(nbt) => return Ok(Some(nbt)),
                Err(_) => std::thread::sleep(DAT_RETRY_DELAY),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        }
    }
    let bytes = std::fs::read(path)?;
    Ok(Some(Compression::Gzip.decompress(&bytes, DAT_LIMIT)?))
}

#[derive(Clone, Debug, PartialEq)]
pub struct DimensionType {
    pub has_skylight: bool,
    pub has_ceiling: bool,
    pub ambient_light: f32,
    pub min_y: i32,
    pub height: i32,
    pub fixed_time: Option<i64>,
    pub coordinate_scale: f64,
}

impl DimensionType {
    pub const OVERWORLD: Self = Self {
        has_skylight: true,
        has_ceiling: false,
        ambient_light: 0.0,
        min_y: -64,
        height: 384,
        fixed_time: None,
        coordinate_scale: 1.0,
    };
    pub const NETHER: Self = Self {
        has_skylight: false,
        has_ceiling: true,
        ambient_light: 0.1,
        min_y: 0,
        height: 256,
        fixed_time: Some(6000),
        coordinate_scale: 8.0,
    };
    /// Values copied from BlueMap's built-ins (`DimensionType.java`), including sky light in the End. Worlds normally
    /// read their type from level data; these only apply when that fails.
    pub const END: Self = Self {
        has_skylight: true,
        has_ceiling: false,
        ambient_light: 0.0,
        min_y: 0,
        height: 256,
        fixed_time: Some(18000),
        coordinate_scale: 1.0,
    };
    pub const OVERWORLD_CAVES: Self = Self { has_ceiling: true, ..Self::OVERWORLD };

    /// An inline dimension type; missing fields are false/0/none (`DimensionTypeData`).
    pub fn from_nbt(c: Compound) -> Self {
        let flag = |name| c.i64(name).is_some_and(|v| v != 0);
        Self {
            has_skylight: flag("has_skylight"),
            has_ceiling: flag("has_ceiling"),
            ambient_light: c.get("ambient_light").and_then(|t| t.as_f64()).unwrap_or(0.0) as f32,
            min_y: c.i64("min_y").unwrap_or(0) as i32,
            height: c.i64("height").unwrap_or(0) as i32,
            fixed_time: c.i64("fixed_time"),
            coordinate_scale: c.get("coordinate_scale").and_then(|t| t.as_f64()).unwrap_or(0.0),
        }
    }

    pub fn builtin(key: &str) -> Option<Self> {
        match key.strip_prefix("minecraft:").unwrap_or(key) {
            "overworld" => Some(Self::OVERWORLD),
            "the_nether" => Some(Self::NETHER),
            "the_end" => Some(Self::END),
            "overworld_caves" => Some(Self::OVERWORLD_CAVES),
            _ => None,
        }
    }
}
