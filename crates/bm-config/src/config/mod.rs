//! Typed config files with BlueMap's keys (kebab-case), Java field defaults and hidden keys.

mod core;
mod map;
mod mask;
mod plugin;
mod storage;
mod webapp;
mod webserver;

pub use core::{CoreConfig, LogConfig};
pub(crate) use map::LEGACY_MESSAGE;
pub use map::MapConfig;
pub use mask::{MaskShape, RenderMask};
pub use plugin::PluginConfig;
pub use storage::{Compression, Dialect, FileStorageConfig, SqlStorageConfig, StorageConfig, StorageFormat};
pub use webapp::WebappConfig;
pub use webserver::{WebserverConfig, WebserverLogConfig};
