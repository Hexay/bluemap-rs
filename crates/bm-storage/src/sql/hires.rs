//! Optimized hires tiles in SQL: one `grid_storage_data` row per tile, BMQ2 blob, under storage key
//! [`HIRES_KEY`] and compression key [`COMPACT_KEY`]. No bundling: the database already packs small rows into
//! pages, and one row per tile keeps single-tile writes a single `REPLACE`. The distinct storage key keeps the
//! rows invisible to Java BlueMap and `sql.php`, which look for `bluemap:hires`.

use std::sync::Arc;

use bm_format::grid::Tile;

use super::SqlMapStorage;
use crate::Result;
use crate::optimized::HiresStore;

pub(crate) const HIRES_KEY: &str = "bluemap-rs:hires";
pub(crate) const COMPACT_KEY: &str = "bluemap-rs:bmq2";

pub(crate) struct SqlHires(pub Arc<SqlMapStorage>);

impl HiresStore for SqlHires {
    fn read(&self, tile: Tile) -> Result<Option<Vec<u8>>> {
        self.0.read_cell(HIRES_KEY, COMPACT_KEY, tile)
    }

    fn write(&self, tile: Tile, blob: &[u8]) -> Result<()> {
        self.0.write_cell(HIRES_KEY, COMPACT_KEY, tile, blob)
    }

    fn delete(&self, tile: Tile) -> Result<()> {
        self.0.delete_cell(HIRES_KEY, tile)
    }

    fn exists(&self, tile: Tile) -> Result<bool> {
        self.0.has_cell(HIRES_KEY, COMPACT_KEY, tile)
    }

    fn list(&self) -> Result<Vec<Tile>> {
        self.0.list_cells(HIRES_KEY, COMPACT_KEY)
    }

    fn clear(&self) -> Result<()> {
        self.0.delete_cells(HIRES_KEY)
    }
}
