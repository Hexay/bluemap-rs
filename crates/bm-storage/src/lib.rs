//! BlueMap map storages, byte-compatible with upstream so users switch (and switch back) without re-rendering:
//! [`FileStorage`] (the `web/maps` tree served by nginx `gzip_static`) and [`SqlStorage`] (the schema `sql.php`
//! reads). Contract: docs/04-storage-web.md §1-2. Both also come in the bluemap-rs-only `optimized` [`Format`]
//! (compact hires tiles, see [`format`](crate::Format)), converted in place by [`convert_file_storage`] /
//! [`convert_sql_storage`].
//!
//! Model: a [`Storage`] holds maps; a [`MapStorage`] holds grids ([`GridKey`]: hires, lowres per LOD, render
//! state) and items ([`ItemKey`]: settings, textures, live markers/players, assets). Reads return the stored,
//! still-compressed bytes ([`Stored`]); writes take raw bytes (`write_*`) or bytes already in the target
//! compression (`write_*_encoded`). Optimized hires cells go in and out as raw PRBM. The traits are object-safe;
//! [`copy_map`] converts between any pair.
//!
//! All calls block. Async code calls them via `spawn_blocking`.

mod api;
mod convert;
mod copy;
mod error;
mod file;
mod format;
mod key;
mod locks;
mod optimized;
mod sql;

pub use api::{MAX_DECODED, MapStorage, Storage, Stored, Version};
pub use bm_compress::Compression;
pub use bm_format::grid::Tile;
pub use convert::{ConvertStats, Grids, Progress, convert_file_storage, convert_sql_storage};
pub use copy::{CopyStats, copy_map};
pub use error::{Error, Result};
pub use file::{FileMapStorage, FileStorage};
pub use format::{FILE_MARKER, Format, SQL_MARKER};
pub use key::{GridKey, ItemKey, escape_asset_name};
pub use locks::{KeyLock, KeyLocks, LockKey};
pub use optimized::{OptimizedMapStorage, packed_model_frame, unpack_hires, with_unpacked_hires};
pub use sql::{Dialect, STATEMENT_TIMEOUT, SqlConfig, SqlMapStorage, SqlStorage, UNBOUNDED_CONNECTIONS};
