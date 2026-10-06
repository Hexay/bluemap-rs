use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;

use bm_format::grid::Tile;

use super::*;

#[derive(Default)]
struct MemIo {
    files: RefCell<BTreeMap<(String, Tile), Vec<u8>>>,
    fail_writes: bool,
}

fn key(kind: CellKind, cell: Tile) -> (String, Tile) {
    (kind.storage_key().to_owned(), cell)
}

impl CellIo for MemIo {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>> {
        Ok(self.files.borrow().get(&key(kind, cell)).cloned())
    }
    fn write_cell(&self, kind: CellKind, cell: Tile, bytes: &[u8]) -> io::Result<()> {
        if self.fail_writes {
            return Err(io::Error::other("disk full"));
        }
        self.files.borrow_mut().insert(key(kind, cell), bytes.to_vec());
        Ok(())
    }
    fn delete_cell(&self, kind: CellKind, cell: Tile) -> io::Result<()> {
        self.files.borrow_mut().remove(&key(kind, cell));
        Ok(())
    }
    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>> {
        let k = kind.storage_key();
        Ok(self.files.borrow().keys().filter(|(n, _)| n == k).map(|(_, c)| *c).collect())
    }
}

#[test]
fn save_writes_only_modified_cells_and_reloads() {
    let io = MemIo::default();
    let mut tiles = MapTileState::new(&io);
    assert_eq!(tiles.get(-40, 3), TileInfo { render_time: 0, state: TileState::Unknown });
    tiles.set(-40, 3, TileInfo { render_time: 1_700_000_000, state: TileState::Rendered });
    tiles.get(100, 100);
    tiles.save().unwrap();
    assert_eq!(io.list_cells(CellKind::Tiles).unwrap(), vec![(-2, 0)]);

    tiles.evict_clean();
    let mut reloaded = MapTileState::new(&io);
    assert_eq!(reloaded.get(-40, 3).render_time, 1_700_000_000);
    assert!(reloaded.take_load_errors().is_empty());
}

#[test]
fn failed_save_keeps_cell_modified() {
    let io = MemIo { fail_writes: true, ..MemIo::default() };
    let mut chunks = MapChunkState::new(&io);
    chunks.set(5, 5, 42);
    assert!(matches!(chunks.save(), Err(Error::Io { op: "write", .. })));
    assert!(chunks.cell_mut((0, 0)).is_modified());
}

#[test]
fn corrupt_cell_is_deleted_and_starts_empty() {
    let io = MemIo::default();
    io.write_cell(CellKind::Chunks, (0, 0), b"garbage").unwrap();
    let mut chunks = MapChunkState::new(&io);
    assert_eq!(chunks.get(1, 1), 0);
    assert!(matches!(chunks.take_load_errors()[..], [Error::Corrupt { kind: CellKind::Chunks, .. }]));
    assert!(io.read_cell(CellKind::Chunks, (0, 0)).unwrap().is_none());
}

#[test]
fn region_for_each_yields_world_coordinates() {
    let io = MemIo::default();
    let mut regions = MapRegionState::new(&io);
    regions.set(-1, 0, 10);
    regions.set(-65, 2, 11);
    regions.set(3, 3, 12);
    regions.delete(3, 3);
    regions.save().unwrap();
    let mut seen = Vec::new();
    MapRegionState::new(&io).for_each(|x, z, t| seen.push((x, z, t))).unwrap();
    seen.sort();
    assert_eq!(seen, vec![(-65, 2, 11), (-1, 0, 10)]);
}
