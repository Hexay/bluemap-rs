//! BlueMap's configuration, drop-in: the same HOCON files (`core.conf`, `webserver.conf`, `webapp.conf`,
//! `plugin.conf`, `maps/*.conf`, `storages/*.conf`, or `.json`) read with the same keys, defaults and coercions as
//! BlueMap's Configurate object mapping, and the same files generated on first start (docs/05 §3).
//!
//! Entry point: [`BlueMapConfig::load`] with [`ConfigOptions::cli`] or [`ConfigOptions::server`]. Single files:
//! [`load_config`], [`load_map_config`], [`load_storage_config`]; raw trees: [`hocon::parse_file`].

pub mod config;
mod de;
mod error;
pub mod generate;
pub mod hocon;
mod key;
mod load;
pub mod template;
mod value;

pub use config::*;
pub use de::size::{MemorySizeError, parse_memory_size};
pub use de::{DeError, from_value};
pub use error::{ConfigError, ParseError};
pub use key::Key;
pub use load::{BlueMapConfig, ConfigOptions, load_config, load_map_config, load_storage_config, resolve_config_file};
pub use value::{Map, Value, java_double_to_string};
