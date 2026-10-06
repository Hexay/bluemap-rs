//! Backend-neutral map copy (file ↔ SQL, compat ↔ future layouts). Bytes pass through untouched when both sides
//! use the same compression, so a same-compression copy is byte-identical.

use crate::Result;
use crate::api::{MapStorage, Stored};
use crate::key::ItemKey;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CopyStats {
    pub cells: usize,
    pub items: usize,
    /// Entries that had to be decompressed and recompressed.
    pub transcoded: usize,
}

/// Copies every grid cell, fixed item and asset of `src` into `dst`.
pub fn copy_map(src: &dyn MapStorage, dst: &dyn MapStorage) -> Result<CopyStats> {
    let mut stats = CopyStats::default();
    for grid in src.grids()? {
        let target = dst.grid_compression(grid);
        for tile in src.list_grid(grid)? {
            let Some(stored) = src.read_grid(grid, tile)? else { continue };
            stats.cells += 1;
            transfer(&stored, target, &mut stats, |enc| dst.write_grid_encoded(grid, tile, enc))?;
        }
    }
    let assets = src.list_assets()?.into_iter().map(ItemKey::Asset);
    for item in ItemKey::FIXED.into_iter().chain(assets) {
        let Some(stored) = src.read_item(&item)? else { continue };
        stats.items += 1;
        transfer(&stored, dst.item_compression(&item), &mut stats, |enc| dst.write_item_encoded(&item, enc))?;
    }
    Ok(stats)
}

fn transfer(
    stored: &Stored,
    target: bm_compress::Compression,
    stats: &mut CopyStats,
    write: impl FnOnce(&[u8]) -> Result<()>,
) -> Result<()> {
    if stored.compression == target {
        return write(&stored.data);
    }
    stats.transcoded += 1;
    write(&target.compress(&stored.decompress()?)?)
}
