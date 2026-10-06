//! BlueMap map storages, byte-compatible with upstream so users switch (and switch back) without re-rendering:
//! [`FileStorage`] (the `web/maps` tree served by nginx `gzip_static`) and [`SqlStorage`] (the schema `sql.php`
//! reads). Contract: docs/04-storage-web.md §1-2.
//!
//! Model: a [`Storage`] holds maps; a [`MapStorage`] holds grids ([`GridKey`]: hires, lowres per LOD, render
//! state) and items ([`ItemKey`]: settings, textures, live markers/players, assets). Reads return the stored,
//! still-compressed bytes ([`Stored`]); writes take raw bytes (`write_*`) or bytes already in the target
//! compression (`write_*_encoded`). The traits are object-safe so further layouts (`optimized`) plug in beside
//! these two, and [`copy_map`] converts between any pair.
//!
//! All calls block. Async code calls them via `spawn_blocking`.

mod api;
mod copy;
mod error;
mod file;
mod key;
mod locks;
mod sql;

pub use api::{MAX_DECODED, MapStorage, Storage, Stored};
pub use bm_compress::Compression;
pub use bm_format::grid::Tile;
pub use copy::{CopyStats, copy_map};
pub use error::{Error, Result};
pub use file::{FileMapStorage, FileStorage};
pub use key::{GridKey, ItemKey, escape_asset_name};
pub use locks::{KeyLock, KeyLocks, LockKey};
pub use sql::{Dialect, SqlConfig, SqlMapStorage, SqlStorage};
