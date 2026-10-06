//! `CellStorage` and its three subclasses: lazily loaded cells over a storage-agnostic byte API.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use bm_format::grid::Tile;

use super::cells::{Cell, ChunkInfoRegion, IntCell, IntField, RegionInfoRegion, TileInfo, TileInfoRegion};
use super::{CellKind, Error, Result};

/// Raw access to a map's render-state grids (a `GridStorage` per [`CellKind`]). Bytes are the stored, gzip'd form.
pub trait CellIo {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>>;
    fn write_cell(&self, kind: CellKind, cell: Tile, bytes: &[u8]) -> io::Result<()>;
    fn delete_cell(&self, kind: CellKind, cell: Tile) -> io::Result<()>;
    /// Every cell that currently exists in storage.
    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>>;
}

impl<T: CellIo + ?Sized> CellIo for &T {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>> {
        (**self).read_cell(kind, cell)
    }
    fn write_cell(&self, kind: CellKind, cell: Tile, bytes: &[u8]) -> io::Result<()> {
        (**self).write_cell(kind, cell, bytes)
    }
    fn delete_cell(&self, kind: CellKind, cell: Tile) -> io::Result<()> {
        (**self).delete_cell(kind, cell)
    }
    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>> {
        (**self).list_cells(kind)
    }
}

impl<T: CellIo + ?Sized> CellIo for Arc<T> {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>> {
        (**self).read_cell(kind, cell)
    }
    fn write_cell(&self, kind: CellKind, cell: Tile, bytes: &[u8]) -> io::Result<()> {
        (**self).write_cell(kind, cell, bytes)
    }
    fn delete_cell(&self, kind: CellKind, cell: Tile) -> io::Result<()> {
        (**self).delete_cell(kind, cell)
    }
    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>> {
        (**self).list_cells(kind)
    }
}

pub type MapTileState<S> = CellStore<TileInfoRegion, S>;
pub type MapChunkState<S> = CellStore<ChunkInfoRegion, S>;
pub type MapRegionState<S> = CellStore<RegionInfoRegion, S>;

/// Loaded cells of one grid. Like Java, a cell that fails to load starts empty: a corrupt one is also deleted
/// (self-healing), and every failure is queued for [`CellStore::take_load_errors`] instead of being logged.
pub struct CellStore<C, S> {
    io: S,
    cells: HashMap<Tile, C>,
    load_errors: Vec<Error>,
}

impl<C: Cell, S: CellIo> CellStore<C, S> {
    pub fn new(io: S) -> Self {
        Self { io, cells: HashMap::new(), load_errors: Vec::new() }
    }

    /// The cell at cell coordinates `pos`, loading it on first access.
    pub fn cell_mut(&mut self, pos: Tile) -> &mut C {
        if !self.cells.contains_key(&pos) {
            let cell = self.load(pos);
            self.cells.insert(pos, cell);
        }
        self.cells.get_mut(&pos).expect("inserted above")
    }

    fn load(&mut self, cell: Tile) -> C {
        let kind = C::KIND;
        match self.io.read_cell(kind, cell) {
            Ok(Some(bytes)) => match C::decode(&bytes) {
                Ok(c) => return c,
                Err(e) => {
                    self.load_errors.push(Error::Corrupt { kind, cell, source: Box::new(e) });
                    if let Err(source) = self.io.delete_cell(kind, cell) {
                        self.load_errors.push(Error::Io { op: "delete", kind, cell, source });
                    }
                }
            },
            Ok(None) => {}
            Err(source) => self.load_errors.push(Error::Io { op: "read", kind, cell, source }),
        }
        C::new()
    }

    /// Writes every modified cell. Failed cells stay modified for the next save; the first failure is returned.
    pub fn save(&mut self) -> Result<()> {
        let mut first_err = None;
        for (&cell, c) in self.cells.iter_mut().filter(|(_, c)| c.is_modified()) {
            let written = c.encode().and_then(|bytes| {
                let kind = C::KIND;
                self.io.write_cell(kind, cell, &bytes).map_err(|source| Error::Io { op: "write", kind, cell, source })
            });
            match written {
                Ok(()) => c.set_modified(false),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Drops unmodified cells from memory; they reload from storage on next access.
    pub fn evict_clean(&mut self) {
        self.cells.retain(|_, c| c.is_modified());
    }

    /// Forgets all loaded cells without saving (`MapPurgeTask`, after the storage was deleted).
    pub fn reset(&mut self) {
        self.cells.clear();
    }

    pub fn take_load_errors(&mut self) -> Vec<Error> {
        std::mem::take(&mut self.load_errors)
    }

    fn cell_at(&mut self, x: i32, z: i32) -> &mut C {
        self.cell_mut(C::KIND.cell_of(x, z))
    }
}

impl<S: CellIo> CellStore<TileInfoRegion, S> {
    /// Tile (x, z) in hires-tile coordinates.
    pub fn get(&mut self, x: i32, z: i32) -> TileInfo {
        self.cell_at(x, z).get(x, z)
    }

    pub fn set(&mut self, x: i32, z: i32, info: TileInfo) -> TileInfo {
        self.cell_at(x, z).set(x, z, info)
    }
}

impl<F: IntField, S: CellIo> CellStore<IntCell<F>, S> {
    /// Entry (x, z) in chunk or region coordinates.
    pub fn get(&mut self, x: i32, z: i32) -> i32 {
        self.cell_at(x, z).get(x, z)
    }

    pub fn set(&mut self, x: i32, z: i32, value: i32) -> i32 {
        self.cell_at(x, z).set(x, z, value)
    }
}

impl<S: CellIo> CellStore<RegionInfoRegion, S> {
    /// Marks a region as gone (its file no longer exists).
    pub fn delete(&mut self, x: i32, z: i32) -> i32 {
        self.set(x, z, 0)
    }

    /// Every region with a non-zero last-update time across all stored cells, as `(x, z, time)`, in Java's order
    /// (cells in storage order, then x-major within a cell).
    pub fn for_each(&mut self, mut f: impl FnMut(i32, i32, i32)) -> Result<()> {
        let kind = CellKind::Regions;
        let cells = self.io.list_cells(kind).map_err(|source| Error::Io { op: "list", kind, cell: (0, 0), source })?;
        let len = 1 << kind.shift();
        for (cx, cz) in cells {
            let cell = self.cell_mut((cx, cz));
            for x in 0..len {
                for z in 0..len {
                    let time = cell.get(x, z);
                    if time != 0 {
                        f((cx << kind.shift()) + x, (cz << kind.shift()) + z, time);
                    }
                }
            }
        }
        Ok(())
    }
}
