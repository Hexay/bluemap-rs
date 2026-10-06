//! Textures: the `minecraft:blocks` atlas, PNG loading and analysis, animation meta and the per-map
//! `textures.json` gallery (docs/02-resources.md §4, §9).
//!
//! `texture` data URLs embed the source PNG unchanged unless it was generated (unstitch, paletted
//! permutations) or carries colour-management chunks; upstream always re-encodes. The webapp only decodes the
//! image into a `<img>`, so pixels are identical either way, but the base64 strings differ from Java's.

mod animation;
mod atlas;
mod bake;
mod gallery;
mod gson;
mod image;
mod load;

pub use animation::{AnimationMeta, FrameMeta};
pub use atlas::{Atlas, Region, Source};
pub use gallery::TextureGallery;
pub use gson::GsonError;
pub use image::{DecodedPng, RgbaImage, decode_png};
pub use load::{TexturePool, load_textures};

use std::sync::Arc;

use base64::Engine;
use bm_math::Color;

use crate::key::ResourcePath;

/// `ResourcePack.MISSING_TEXTURE`: the gallery's id 0 and every unresolved face's texture.
pub const MISSING_TEXTURE: &str = "bluemap:block/missing";
const MISSING_KEY: &str = "bluemap:missing";
const MISSING_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAPklEQVR4Xu3MsQkAMAwDQe2/tFPnBB4gpLhG8MpkZpNkZ6AKZKAKZKAKZKAKZKAKZKAKZKAKWg0XD/UPnjg4MbX+EDdeTUwAAAAASUVORK5CYII=";
const DATA_URL_PREFIX: &str = "data:image/png;base64,";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("png: {0}")]
    Decode(#[from] png::DecodingError),
    #[error("png: {0}")]
    Encode(#[from] png::EncodingError),
    #[error("png decoder returned an unexpanded palette image")]
    UnexpandedPalette,
    #[error(transparent)]
    Json(#[from] crate::json::JsonError),
    #[error(transparent)]
    Gson(#[from] GsonError),
    #[error("invalid UTF-8 in {0}")]
    Utf8(&'static str),
    #[error("textures.json: {0}")]
    Gallery(&'static str),
    #[error("texture is not a base64 PNG data URL")]
    DataUrl,
}

/// One `textures.json` entry (`RP/texture/Texture.java`).
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub key: ResourcePath,
    /// Straight alpha.
    pub color: Color,
    pub half_transparent: bool,
    /// `data:image/png;base64,...`; `None` only when read back as an explicit null.
    pub texture: Option<Arc<str>>,
    pub animation: Option<AnimationMeta>,
}

impl Texture {
    /// `Texture.MISSING` (key `bluemap:missing`), which fills unused gallery slots.
    pub fn missing() -> Self {
        Self::missing_for(ResourcePath::key(MISSING_KEY))
    }

    /// `Texture.missing(key)`: the missing texture under another key.
    pub fn missing_for(key: ResourcePath) -> Self {
        Self {
            key,
            color: Color { r: 0.5, g: 0.0, b: 0.5, a: 1.0, premultiplied: false },
            half_transparent: false,
            texture: Some(MISSING_DATA_URL.into()),
            animation: None,
        }
    }

    /// `Texture.from`. `png` is the source file to embed; `None` encodes `image`.
    pub fn from_image(
        key: ResourcePath,
        image: &RgbaImage,
        animation: Option<AnimationMeta>,
        png: Option<&[u8]>,
    ) -> Result<Self, Error> {
        let encoded;
        let png = match png {
            Some(p) => p,
            None => {
                encoded = image.encode_png()?;
                &encoded
            }
        };
        let mut url = String::with_capacity(DATA_URL_PREFIX.len() + png.len() * 4 / 3 + 4);
        url.push_str(DATA_URL_PREFIX);
        base64::engine::general_purpose::STANDARD.encode_string(png, &mut url);
        Ok(Self {
            key,
            color: *image.average_color().straight(),
            half_transparent: image.half_transparent(),
            texture: Some(url.into()),
            animation,
        })
    }

    pub fn color_premultiplied(&self) -> Color {
        let mut c = self.color;
        *c.premultiplied()
    }

    /// The embedded PNG's bytes.
    pub fn png_bytes(&self) -> Result<Vec<u8>, Error> {
        let b64 = self.texture.as_deref().and_then(|t| t.strip_prefix(DATA_URL_PREFIX)).ok_or(Error::DataUrl)?;
        base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| Error::DataUrl)
    }

    /// `getTextureImage`: the embedded PNG decoded.
    pub fn decode_image(&self) -> Result<RgbaImage, Error> {
        Ok(decode_png(&self.png_bytes()?)?.image)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "golden_tests.rs"]
mod golden_tests;
