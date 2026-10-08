//! bm-map's storage-agnostic traits over a [`MapStorage`]: render-state cells and lowres tiles.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::SyncSender;

use bm_format::grid::Tile;
use bm_format::lowres::{LowresError, LowresTile};
use bm_map::lowres::LowresStore;
use bm_map::renderstate::{CellIo, CellKind};
use bm_storage::{GridKey, MapStorage};

use crate::map::TileListener;
use crate::persist::Msg;

pub(crate) fn grid_key(kind: CellKind) -> GridKey {
    match kind {
        CellKind::Tiles => GridKey::TileState,
        CellKind::Chunks => GridKey::ChunkState,
        CellKind::Regions => GridKey::RegionState,
    }
}

/// Each encoder holds ~2 MB, and flushes overlap rendering, so a few threads keep the peak-RSS cost small.
const FLUSH_THREADS: usize = 4;

fn io_err(e: bm_storage::Error) -> io::Error {
    io::Error::other(e)
}

/// Reads go straight to storage; writes are queued to the persistence thread so saving never blocks the caller.
pub(crate) struct QueuedCells {
    pub storage: Arc<dyn MapStorage>,
    pub queue: SyncSender<Msg>,
}

impl CellIo for QueuedCells {
    fn read_cell(&self, kind: CellKind, cell: Tile) -> io::Result<Option<Vec<u8>>> {
        Ok(self.storage.read_grid(grid_key(kind), cell).map_err(io_err)?.map(|s| s.data))
    }

    fn write_cell(&self, kind: CellKind, cell: Tile, bytes: &[u8]) -> io::Result<()> {
        let msg = Msg::WriteCell { grid: grid_key(kind), cell, bytes: bytes.to_vec() };
        self.queue.send(msg).map_err(|_| io::Error::other("the persistence thread has stopped"))
    }

    fn delete_cell(&self, kind: CellKind, cell: Tile) -> io::Result<()> {
        self.storage.delete_grid(grid_key(kind), cell).map_err(io_err)
    }

    fn list_cells(&self, kind: CellKind) -> io::Result<Vec<Tile>> {
        self.storage.list_grid(grid_key(kind)).map_err(io_err)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LowresStoreError {
    #[error(transparent)]
    Storage(#[from] bm_storage::Error),
    #[error(transparent)]
    Png(#[from] LowresError),
}

/// Lowres PNGs in a map storage, never compressed (`GridKey::Lowres`).
pub(crate) struct StorageLowres {
    pub storage: Arc<dyn MapStorage>,
    pub tile_size: [usize; 2],
    pub png: Vec<u8>,
    pub saves: usize,
    pub on_save: Option<TileListener>,
}

impl LowresStore for StorageLowres {
    type Error = LowresStoreError;

    fn load(&mut self, lod: u32, tile: Tile) -> Result<Option<LowresTile>, LowresStoreError> {
        let Some(stored) = self.storage.read_grid(GridKey::Lowres(lod), tile)? else { return Ok(None) };
        // like `LowresLayer`, an unreadable tile starts over empty instead of failing the render
        Ok(LowresTile::decode_png(&stored.decompress()?, self.tile_size).ok())
    }

    fn save(&mut self, lod: u32, tile: Tile, data: &LowresTile) -> Result<(), LowresStoreError> {
        let mut png = std::mem::take(&mut self.png);
        let result = self.write_png(lod, tile, data, &mut png);
        self.png = png;
        self.saves += usize::from(result.is_ok());
        result
    }

    /// PNG encoding makes the persistence thread the bottleneck once the last regions finish, so tiles are encoded in
    /// parallel, on scoped threads: rayon's workers may be blocked sending to this thread.
    fn save_all(&mut self, lod: u32, tiles: &[(Tile, &LowresTile)]) -> Vec<Result<(), LowresStoreError>> {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(FLUSH_THREADS).min(tiles.len());
        if threads < 2 {
            return tiles.iter().map(|&(tile, data)| self.save(lod, tile, data)).collect();
        }
        let next = AtomicUsize::new(0);
        let this = &*self;
        let mut results: Vec<(usize, Result<(), LowresStoreError>)> = std::thread::scope(|s| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| {
                        let (mut png, mut done) = (Vec::new(), Vec::new());
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(&(tile, data)) = tiles.get(i) else { break };
                            done.push((i, this.write_png(lod, tile, data, &mut png)));
                        }
                        done
                    })
                })
                .collect();
            workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
        });
        results.sort_unstable_by_key(|&(i, _)| i);
        self.saves += results.iter().filter(|(_, r)| r.is_ok()).count();
        results.into_iter().map(|(_, r)| r).collect()
    }

    /// Java's listeners fire for every re-saved tile, changed or not.
    fn unchanged(&mut self, lod: u32, tile: Tile) {
        self.notify(lod, tile);
    }
}

impl StorageLowres {
    fn write_png(&self, lod: u32, tile: Tile, data: &LowresTile, png: &mut Vec<u8>) -> Result<(), LowresStoreError> {
        data.encode_png(png)?;
        self.storage.write_grid(GridKey::Lowres(lod), tile, png)?;
        self.notify(lod, tile);
        Ok(())
    }

    fn notify(&self, lod: u32, tile: Tile) {
        if let Some(listener) = &self.on_save {
            listener(tile, lod);
        }
    }
}
