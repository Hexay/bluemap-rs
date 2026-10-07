//! Backend-neutral storage traits, mirroring BlueMap's `Storage` / `MapStorage` / `GridStorage` / `ItemStorage`.

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use bm_compress::Compression;
use bm_format::grid::Tile;

use crate::Result;
use crate::key::{GridKey, ItemKey};
use crate::locks::{KeyLock, KeyLocks, LockKey};

/// Upper bound for [`Stored::decompress`]: well above the largest textures.json seen in modpacks.
pub const MAX_DECODED: usize = 1 << 30;

/// Bytes exactly as stored (still compressed) plus their compression; the web layer forwards them as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub data: Vec<u8>,
    pub compression: Compression,
}

impl Stored {
    pub fn decompress(&self) -> Result<Vec<u8>> {
        Ok(self.compression.decompress(&self.data, MAX_DECODED)?)
    }
}

/// Identity of a key's stored bytes, read without the bytes (HTTP validators): an unchanged version means
/// unchanged bytes, and every write gives a new version (even of equal bytes). Opaque; compare or print it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Version(pub(crate) [u64; 3]);

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c] = self.0;
        write!(f, "{a:x}-{b:x}-{c:x}")
    }
}

/// A storage holding many maps (BlueMap `Storage`). Blocking; see the crate docs for async callers.
pub trait Storage: Send + Sync {
    /// The same `Arc` for the same id for the storage's lifetime, so [`MapStorage::lock_grid`] is shared.
    fn map(&self, map_id: &str) -> Result<Arc<dyn MapStorage>>;

    fn map_ids(&self) -> Result<Vec<String>>;
}

/// One map's data (BlueMap `MapStorage`). Writes are atomic per cell/item: readers see the old or the new bytes.
pub trait MapStorage: Send + Sync {
    fn map_id(&self) -> &str;

    /// Configured compression, used for hires tiles and textures.json.
    fn compression(&self) -> Compression;

    fn read_grid(&self, grid: GridKey, tile: Tile) -> Result<Option<Stored>>;

    /// Writes bytes already encoded with [`MapStorage::grid_compression`].
    fn write_grid_encoded(&self, grid: GridKey, tile: Tile, encoded: &[u8]) -> Result<()>;

    fn delete_grid(&self, grid: GridKey, tile: Tile) -> Result<()>;

    fn grid_exists(&self, grid: GridKey, tile: Tile) -> Result<bool>;

    /// All stored cells of a grid (in the grid's current compression), in no particular order.
    fn list_grid(&self, grid: GridKey) -> Result<Vec<Tile>>;

    /// Grids that may hold cells for this map (a superset is allowed: listed grids may be empty).
    fn grids(&self) -> Result<Vec<GridKey>>;

    fn read_item(&self, item: &ItemKey) -> Result<Option<Stored>>;

    /// Writes bytes already encoded with [`MapStorage::item_compression`].
    fn write_item_encoded(&self, item: &ItemKey, encoded: &[u8]) -> Result<()>;

    fn delete_item(&self, item: &ItemKey) -> Result<()>;

    fn item_exists(&self, item: &ItemKey) -> Result<bool>;

    /// Escaped names of all stored assets.
    fn list_assets(&self) -> Result<Vec<String>>;

    /// Deletes everything of this map. `on_progress(0..=1)` returning false aborts (data stays partially deleted).
    fn delete(&self, on_progress: &mut dyn FnMut(f64) -> bool) -> Result<()>;

    fn exists(&self) -> Result<bool>;

    fn key_locks(&self) -> &KeyLocks;

    /// The cell's [`Version`] from metadata only. `None` when it is missing or the backend has no cheap identity
    /// (SQL: Java's schema has no change column).
    fn grid_version(&self, grid: GridKey, tile: Tile) -> Result<Option<Version>> {
        let _ = (grid, tile);
        Ok(None)
    }

    /// The item's [`Version`], like [`MapStorage::grid_version`].
    fn item_version(&self, item: &ItemKey) -> Result<Option<Version>> {
        let _ = item;
        Ok(None)
    }

    /// [`MapStorage::read_grid`] with a version no newer than the bytes. Taken first here, so a concurrent
    /// rewrite can pair new bytes with the old version (a needless refetch later), never the reverse; backends
    /// override it to take both from one file handle.
    fn read_grid_versioned(&self, grid: GridKey, tile: Tile) -> Result<Option<(Stored, Option<Version>)>> {
        let version = self.grid_version(grid, tile)?;
        Ok(self.read_grid(grid, tile)?.map(|s| (s, version)))
    }

    /// [`MapStorage::read_item`] with its version, like [`MapStorage::read_grid_versioned`].
    fn read_item_versioned(&self, item: &ItemKey) -> Result<Option<(Stored, Option<Version>)>> {
        let version = self.item_version(item)?;
        Ok(self.read_item(item)?.map(|s| (s, version)))
    }

    fn grid_compression(&self, grid: GridKey) -> Compression {
        grid.compression(self.compression())
    }

    fn item_compression(&self, item: &ItemKey) -> Compression {
        item.compression(self.compression())
    }

    /// Compresses `raw` with the grid's compression and writes it.
    fn write_grid(&self, grid: GridKey, tile: Tile, raw: &[u8]) -> Result<()> {
        with_encoded(self.grid_compression(grid), raw, |e| self.write_grid_encoded(grid, tile, e))
    }

    fn write_item(&self, item: &ItemKey, raw: &[u8]) -> Result<()> {
        with_encoded(self.item_compression(item), raw, |e| self.write_item_encoded(item, e))
    }

    /// Exclusive per-cell lock for read-modify-write sequences (e.g. lowres tiles, #821). Not reentrant;
    /// plain writes don't take it.
    fn lock_grid(&self, grid: GridKey, tile: Tile) -> KeyLock<'_> {
        self.key_locks().lock(LockKey::Grid(grid, tile))
    }

    fn lock_item(&self, item: &ItemKey) -> KeyLock<'_> {
        self.key_locks().lock(LockKey::Item(item.clone()))
    }
}

const SCRATCH_KEEP: usize = 4 << 20;

thread_local! {
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn with_encoded(c: Compression, raw: &[u8], write: impl FnOnce(&[u8]) -> Result<()>) -> Result<()> {
    if c == Compression::None {
        return write(raw);
    }
    // take() keeps this reentrancy-safe if a backend writes from inside `write`
    let mut buf = SCRATCH.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let result = c.compress_into(raw, &mut buf).map_err(Into::into).and_then(|()| write(&buf));
    if buf.capacity() <= SCRATCH_KEEP {
        SCRATCH.with(|s| *s.borrow_mut() = buf);
    }
    result
}
