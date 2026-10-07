use super::*;

const WHITE: u32 = 0xFFFF_FFFF;
const MAGENTA: u32 = 0xFFFF_00FF;
const UNWRITTEN_META: u32 = 0;
const ZERO_META: u32 = 0xFF00_0000;

fn manager(lod_count: u32) -> LowresTileManager<MemoryStore> {
    LowresTileManager::new(MemoryStore::default(), [4, 4], lod_count, 2)
}

fn tile(store: &MemoryStore, lod: u32, t: Tile) -> &LowresTile {
    store.tiles.get(&(lod, t)).unwrap_or_else(|| panic!("lod {lod} tile {t:?} missing"))
}

#[test]
fn seam_copies_reach_left_top_and_corner_neighbours() {
    let mut m = manager(1);
    m.set_argb(0, 0, MAGENTA, 70, 9).unwrap();
    m.set_argb(3, 2, WHITE, 1, 0).unwrap();
    m.flush().unwrap();
    let s = m.into_store();
    assert_eq!(s.tiles.len(), 4);
    for (t, x, z) in [((0, 0), 0, 0), ((-1, 0), 4, 0), ((0, -1), 0, 4), ((-1, -1), 4, 4)] {
        let d = tile(&s, 1, t);
        assert_eq!((d.color(x, z), d.height(x, z), d.block_light(x, z)), (MAGENTA, 70, 9), "{t:?}");
    }
    assert_eq!(tile(&s, 1, (0, 0)).color(3, 2), WHITE);
    assert_eq!(tile(&s, 1, (-1, 0)).meta(3, 2), UNWRITTEN_META, "seam-only tiles keep an unwritten core");
}

#[test]
fn colours_are_stored_straight_and_truncated() {
    let mut m = manager(1);
    m.set(0, 1, Color { a: 0.5, r: 0.5, premultiplied: true, ..Color::default() }, 3, 0).unwrap();
    m.set(1, 1, Color { premultiplied: true, ..Color::default() }, 0, 0).unwrap();
    m.flush().unwrap();
    let s = m.into_store();
    let d = tile(&s, 1, (0, 0));
    assert_eq!(d.color(0, 1), 0x7FFF_0000, "alpha 127.5 truncates");
    assert_eq!((d.color(1, 1), d.meta(1, 1)), (0, ZERO_META), "transparent columns still mark meta written");
}

#[test]
fn downsample_averages_premultiplied_and_truncates_toward_zero() {
    let mut m = manager(2);
    m.set_argb(1, 1, WHITE, -1, 15).unwrap();
    for (x, z) in [(1, 2), (2, 1), (2, 2)] {
        m.set_argb(x, z, 0, 0, 0).unwrap();
    }
    m.flush().unwrap();
    let s = m.into_store();
    let lod2 = tile(&s, 2, (0, 0));
    // group (0,0) holds only the untouched (0,0)/(0,1)/(1,0) zeros plus white at (1,1)
    assert_eq!(lod2.color(0, 0), 0x3FFF_FFFF, "premultiplied mean 0.25 re-straightened to white");
    assert_eq!((lod2.height(0, 0), lod2.block_light(0, 0)), (0, 3), "-1/4 truncates to 0, 15/4 to 3");
    assert_eq!((lod2.color(1, 1), lod2.height(1, 1)), (0, 0));
    assert_eq!(lod2.meta(1, 0), ZERO_META, "groups of a saved tile are written even if empty");
    assert_eq!(lod2.meta(2, 2), UNWRITTEN_META, "pixels of other lod-1 tiles stay unwritten");
}

#[test]
fn height_0x8000_averages_as_positive() {
    let mut m = manager(2);
    m.set_argb(0, 0, WHITE, -32768, 0).unwrap();
    m.flush().unwrap();
    let s = m.into_store();
    assert_eq!(tile(&s, 2, (0, 0)).height(0, 0), 8192, "Java reads 0x8000 back as +32768");
}

#[test]
fn negative_tiles_cascade_by_floor_div_and_mod() {
    let mut m = manager(3);
    for (x, z) in [(-4, -4), (-3, -4), (-4, -3), (-3, -3)] {
        m.set_argb(x, z, MAGENTA, 64, 4).unwrap();
    }
    m.flush().unwrap();
    let s = m.into_store();
    let lod2 = tile(&s, 2, (-1, -1));
    assert_eq!((lod2.color(2, 2), lod2.height(2, 2), lod2.block_light(2, 2)), (MAGENTA, 64, 4));
    assert_eq!(tile(&s, 3, (-1, -1)).meta(2, 2), ZERO_META, "every group of a saved tile is written");
    assert_eq!(tile(&s, 3, (-1, -1)).color(3, 3), 0x3FFF_00FF);
}

#[test]
fn incremental_updates_modify_stored_tiles() {
    let mut store = MemoryStore::default();
    let mut old = LowresTile::new([4, 4]);
    old.set(3, 3, WHITE, 5, 1);
    store.tiles.insert((1, (0, 0)), old);
    let mut m = LowresTileManager::new(store, [4, 4], 1, 2);
    m.set_argb(1, 1, MAGENTA, 6, 2).unwrap();
    m.flush().unwrap();
    let d = m.into_store().tiles.remove(&(1, (0, 0))).unwrap();
    assert_eq!((d.color(3, 3), d.height(3, 3), d.color(1, 1)), (WHITE, 5, MAGENTA));
}

#[test]
fn flush_saves_each_dirty_tile_once() {
    let mut m = manager(3);
    for x in -9..9 {
        for z in -9..9 {
            m.set_argb(x, z, WHITE, x + z, 0).unwrap();
        }
    }
    m.flush().unwrap();
    let s = m.into_store();
    assert_eq!(s.saves, s.tiles.len());
}

#[test]
fn unchanged_tiles_are_not_saved_again() {
    let mut m = manager(2);
    m.set_argb(1, 1, WHITE, 3, 0).unwrap();
    m.flush().unwrap();
    assert_eq!(m.store().saves, 5, "the lod-1 tile, its lod-2 tile and that one's 3 seam neighbours");
    m.set_argb(1, 1, WHITE, 3, 0).unwrap();
    m.flush().unwrap();
    assert_eq!(m.store().saves, 5, "same pixels: no save at any lod");
    m.set_argb(1, 1, MAGENTA, 3, 0).unwrap();
    m.flush().unwrap();
    assert_eq!(m.store().saves, 10, "every tile changed again");
    assert_eq!(tile(m.store(), 2, (0, 0)).color(0, 0), 0x3FFF_00FF);
}

#[derive(Debug, thiserror::Error)]
#[error("disk full")]
struct DiskFull;

#[derive(Default)]
struct FailingStore {
    inner: MemoryStore,
    failures: usize,
}

impl LowresStore for FailingStore {
    type Error = DiskFull;
    fn load(&mut self, lod: u32, t: Tile) -> std::result::Result<Option<LowresTile>, DiskFull> {
        Ok(self.inner.load(lod, t).unwrap())
    }
    fn save(&mut self, lod: u32, t: Tile, data: &LowresTile) -> std::result::Result<(), DiskFull> {
        if self.failures > 0 {
            self.failures -= 1;
            return Err(DiskFull);
        }
        self.inner.save(lod, t, data).map_err(|e| match e {})
    }
}

#[test]
fn failed_save_keeps_tile_dirty_for_retry() {
    let mut m = LowresTileManager::new(FailingStore { failures: 1, ..Default::default() }, [4, 4], 2, 2);
    m.set_argb(1, 1, WHITE, 0, 0).unwrap();
    let err = m.flush().unwrap_err();
    assert!(matches!(err, LowresLayerError::Save { lod: 1, tile: (0, 0), .. }), "{err}");
    assert_eq!(m.dirty_tiles(1).collect::<Vec<_>>(), vec![(0, 0)]);
    m.flush().unwrap();
    assert_eq!(m.dirty_tiles(1).count() + m.dirty_tiles(2).count(), 0);
    assert_eq!(m.store().inner.tiles.len(), 5, "lod-1 tile, lod-2 tile and its 3 seam neighbours");
}
