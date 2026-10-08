//! The persistence thread of one map update: sole owner of the lowres layers and writer of render-state cells, so
//! render workers only ever enqueue (docs/10-perf-audit-render.md §1–2, #821). Closing the queue flushes everything.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use bm_format::grid::Tile;
use bm_map::lowres::LowresTileManager;
use bm_storage::{GridKey, MapStorage};

use crate::error::{Error, Result};
use crate::io::StorageLowres;
use crate::map::TileListener;

/// Tiles' worth of column batches that may wait; a full queue makes workers wait for the writer, not each other.
const QUEUE: usize = 1024;

/// One block column for the lowres layer, colour already straight ARGB.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Column {
    pub x: i32,
    pub z: i32,
    pub argb: u32,
    pub height: i32,
    pub light: u8,
}

pub(crate) enum Msg {
    Columns(Vec<Column>),
    /// Save these `(lod, tile)`s if dirty: no remaining region writes to them.
    Flush(Vec<(u32, Tile)>),
    /// A gzip'd render-state cell.
    WriteCell {
        grid: GridKey,
        cell: Tile,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Default)]
pub struct PersistStats {
    pub lowres_saves: usize,
    pub cell_writes: usize,
}

pub(crate) struct Persister {
    pub queue: SyncSender<Msg>,
    handle: JoinHandle<Result<PersistStats>>,
}

pub(crate) struct LowresSettings {
    pub tile_size: i32,
    pub lod_count: u32,
    pub lod_factor: i32,
}

impl Persister {
    pub fn start(storage: Arc<dyn MapStorage>, lowres: LowresSettings, on_save: Option<TileListener>) -> Result<Self> {
        if lowres.tile_size <= 0 || lowres.lod_count == 0 || lowres.lod_factor <= 0 {
            return Err(Error::Invalid("lowres-tile-size, lod-count and lod-factor must be positive".into()));
        }
        let (queue, rx) = sync_channel(QUEUE);
        let handle = std::thread::Builder::new()
            .name("bluemap-persist".into())
            .spawn(move || run(rx, storage, lowres, on_save))
            .map_err(|e| Error::Lowres(format!("failed to start the persistence thread: {e}")))?;
        Ok(Self { queue, handle })
    }

    /// Closes the queue (every other sender must be dropped already) and waits for the final flush.
    pub fn finish(self) -> Result<PersistStats> {
        drop(self.queue);
        self.handle.join().unwrap_or_else(|_| Err(Error::Lowres("the persistence thread panicked".into())))
    }
}

fn run(
    rx: Receiver<Msg>,
    storage: Arc<dyn MapStorage>,
    s: LowresSettings,
    on_save: Option<TileListener>,
) -> Result<PersistStats> {
    let size = s.tile_size as usize;
    let store = StorageLowres { storage: storage.clone(), tile_size: [size, size], png: Vec::new(), saves: 0, on_save };
    let mut lowres = LowresTileManager::new(store, [s.tile_size; 2], s.lod_count, s.lod_factor);
    let mut stats = PersistStats::default();
    // keep going after a failure so one bad tile doesn't lose the rest; report the first
    let mut first_err: Option<Error> = None;
    let mut fail = |e: Error| _ = first_err.get_or_insert(e);
    for msg in rx {
        match msg {
            Msg::Columns(columns) => {
                for c in columns {
                    if let Err(e) = lowres.set_argb(c.x, c.z, c.argb, c.height, i32::from(c.light)) {
                        fail(Error::Lowres(e.to_string()));
                    }
                }
            }
            Msg::Flush(tiles) => {
                let tiles: HashSet<(u32, Tile)> = tiles.into_iter().collect();
                for lod in 1..=lowres.lod_count() {
                    if let Err(e) = lowres.flush_lod(lod, |t| tiles.contains(&(lod, t))) {
                        fail(Error::Lowres(e.to_string()));
                    }
                }
            }
            Msg::WriteCell { grid, cell, bytes } => match storage.write_grid_encoded(grid, cell, &bytes) {
                Ok(()) => stats.cell_writes += 1,
                Err(e) => fail(e.into()),
            },
        }
    }
    if let Err(e) = lowres.flush() {
        fail(Error::Lowres(e.to_string()));
    }
    stats.lowres_saves = lowres.store().saves;
    first_err.map_or(Ok(stats), Err)
}
