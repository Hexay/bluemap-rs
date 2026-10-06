//! Block tint colours and block properties (docs/02 §5): `blockColors.json`, `blockProperties.json`, colormaps and
//! the biome colour calculators.

mod block_colors;
mod colormap;
mod mapping;
mod properties;
mod tint;

pub use block_colors::{BlockColors, BlockColorsConfig, ColorSource, Tint};
pub use colormap::{ColorMap, ColorMaps};
pub use mapping::{StateMapping, StateMatcher};
pub use properties::{BlockProperties, BlockPropertiesConfig, ModelProperties, Tristate};
pub use tint::{
    DRY_FOLIAGE_DEFAULT, FOLIAGE_DEFAULT, GRASS_DEFAULT, blend, dry_foliage, foliage, grass, grass_modifier, redstone,
    redstone_power, water,
};

use crate::json::JsonError;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Json(#[from] JsonError),
    #[error("'{0}' could not be parsed to a BlockState!")]
    BadState(String),
    #[error("'{key}': expected {expected}")]
    Expected { key: String, expected: &'static str },
    #[error("png: {0}")]
    Png(#[from] png::DecodingError),
    #[error("colormap is {0}x{1}, needs at least 256x256")]
    ColormapTooSmall(u32, u32),
}

/// A pack file that failed to load (fully or from some entry on); upstream logs these at debug level.
#[derive(Debug)]
pub struct LoadFailure {
    pub file: String,
    pub error: ConfigError,
}

/// `assets/<ns>/<name>` of every namespace in `pack`, in the pack's (sorted) listing order.
fn namespace_files<'a>(pack: &'a crate::Pack, name: &'a str) -> impl Iterator<Item = (String, String)> + 'a {
    pack.list("assets").into_iter().filter_map(move |ns| {
        let path = format!("assets/{ns}/{name}");
        pack.read_string(&path).map(|src| (path, src))
    })
}
