//! Resource and data pack loading: everything BlueMap reads from the Minecraft client jar, resource packs, mod jars
//! and world datapacks to turn block states into models, textures and colours (docs/02-resources.md).

pub mod blockstate;
pub mod client_jar;
pub mod color;
pub mod datapack;
pub mod json;
pub mod key;
pub mod manifest;
pub mod model;
pub mod pack_meta;
pub mod packs;
pub mod texture;
pub mod vfs;

pub use client_jar::{MinecraftVersion, PackVersions};
pub use key::ResourcePath;
pub use pack_meta::{PackMeta, PackVersion, VersionRange};
pub use vfs::Pack;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] json::JsonError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("pack: {0}")]
    Pack(String),
    #[error("pack.mcmeta: {0}")]
    PackMeta(String),
    #[error("version manifest: {0}")]
    Manifest(String),
    #[error("http: {0}")]
    Http(String),
    #[error("SHA-1 of the downloaded file does not match: expected {expected}, got {actual}")]
    Checksum { expected: String, actual: String },
    #[error("Minecraft client jar '{}' is missing and downloads are not accepted (set accept-download: true)", .0.display())]
    DownloadNotAccepted(PathBuf),
    #[error("Resource-File missing: {}", .0.display())]
    ResourceFileMissing(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;
