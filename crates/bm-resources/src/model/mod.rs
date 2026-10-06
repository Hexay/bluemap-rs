//! Block and item models (`RP/model/*.java`, docs/02 §3): JSON reading with Gson's adapters and coercions,
//! the parent merge, texture-variable resolution and the baked form the hires renderer consumes.
//!
//! Load order: [`ModelLibrary::load_pack`] per pack (highest priority first), then
//! [`ModelLibrary::collect_used_texture_keys`] for the texture loader, then [`ModelLibrary::bake`] once
//! textures are known.

mod baked;
mod direction;
mod element;
mod gson;
mod library;
mod raw;
mod rotation;
mod texture_variable;

#[cfg(test)]
mod java_ref;
#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod parse_tests;

pub use baked::{BakedElement, BakedFace, BakedModel, BakedModels};
pub use direction::Direction;
pub use element::{Element, Face};
pub use library::ModelLibrary;
pub use raw::Model;
pub use rotation::Rotation;
pub use texture_variable::TextureVariable;

use crate::ResourcePath;

/// `ResourcePack.MISSING_TEXTURE`, the texture of faces that name none.
pub fn missing_texture() -> ResourcePath {
    ResourcePath::key("bluemap:block/missing")
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error(transparent)]
    Json(#[from] crate::json::JsonError),
    #[error("file is not valid UTF-8")]
    Utf8,
    #[error("{field}: expected {expected}")]
    Type { field: &'static str, expected: &'static str },
    #[error("unknown direction '{0}'")]
    Direction(String),
    #[error("duplicate face '{0}'")]
    DuplicateFace(String),
    #[error(transparent)]
    Axis(#[from] bm_math::ParseAxisError),
    #[error("can't parse an empty string into a texture variable")]
    EmptyTexture,
    #[error("texture variable object without a sprite")]
    NoSprite,
}
