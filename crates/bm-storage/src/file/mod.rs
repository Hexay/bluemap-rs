//! File storage, path- and byte-compatible with upstream `FileStorage` (default root `bluemap/web/maps`).

pub(crate) mod fsops;
pub(crate) mod layout;
mod marker;
mod version;

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use bm_compress::Compression;
use bm_format::grid::Tile;

pub(crate) use self::marker::{detect_format, set_marker};
use crate::api::{MapStorage, Storage, Stored, Version};
use crate::error::{Error, IoContext, Result};
use crate::format::{self, Format};
use crate::key::{GridKey, ItemKey};
use crate::locks::KeyLocks;
use crate::optimized::OptimizedMapStorage;
use crate::optimized::bundle::BundleStore;

/// Directory of an optimized map's hires bundles, beside upstream's `tiles/`.
pub(crate) const HIRES_BUNDLES: &str = "hires";

/// Upstream's `atomic: false` is not honoured: every write is atomic.
pub struct FileStorage {
    root: PathBuf,
    compression: Compression,
    read_only: bool,
    format: Format,
    maps: Mutex<HashMap<String, Arc<FileMapStorage>>>,
    optimized: Mutex<HashMap<String, Arc<OptimizedMapStorage>>>,
}

impl FileStorage {
    /// A compat storage, without checking what `root` holds (see [`FileStorage::open`]).
    pub fn new(root: impl Into<PathBuf>, compression: Compression) -> Self {
        Self {
            root: root.into(),
            compression,
            read_only: false,
            format: Format::Compat,
            maps: Mutex::default(),
            optimized: Mutex::default(),
        }
    }

    /// Opens a storage of `format`, refusing one that holds the other format and marking a new optimized one
    /// (rules in [`crate::format`]).
    pub fn open(root: impl Into<PathBuf>, compression: Compression, format: Format, read_only: bool) -> Result<Self> {
        let storage = Self { format, ..Self::new(root, compression) }.read_only(read_only);
        let found = detect_format(&storage.root)?;
        format::check(&storage.root.display().to_string(), format, found)?;
        if format == Format::Optimized && found.is_none() && !read_only {
            set_marker(&storage.root, true)?;
        }
        Ok(storage)
    }

    /// Writes and deletes fail with [`Error::ReadOnly`].
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// The map as an optimized map storage over this root, whatever this storage's format (for conversion).
    pub(crate) fn optimized_map(&self, map_id: &str) -> Result<Arc<OptimizedMapStorage>> {
        let inner = self.file_map(map_id)?;
        let mut maps = self.optimized.lock().unwrap_or_else(PoisonError::into_inner);
        let map = maps.entry(map_id.to_owned()).or_insert_with(|| {
            let bundles = BundleStore::new(inner.root.join(HIRES_BUNDLES), self.read_only);
            Arc::new(OptimizedMapStorage::new(inner, Box::new(bundles)))
        });
        Ok(map.clone())
    }

    pub fn file_map(&self, map_id: &str) -> Result<Arc<FileMapStorage>> {
        layout::validate_map_id(map_id)?;
        let mut maps = self.maps.lock().unwrap_or_else(PoisonError::into_inner);
        let map = maps.entry(map_id.to_owned()).or_insert_with(|| {
            Arc::new(FileMapStorage {
                id: map_id.to_owned(),
                root: self.root.join(map_id),
                compression: self.compression,
                read_only: self.read_only,
                locks: KeyLocks::default(),
            })
        });
        Ok(map.clone())
    }
}

impl Storage for FileStorage {
    fn map(&self, map_id: &str) -> Result<Arc<dyn MapStorage>> {
        match self.format {
            Format::Compat => Ok(self.file_map(map_id)?),
            Format::Optimized => Ok(self.optimized_map(map_id)?),
        }
    }

    fn map_ids(&self) -> Result<Vec<String>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).ctx("list", &self.root),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.ctx("list", &self.root)?;
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                ids.push(name.to_owned());
            }
        }
        ids.sort();
        Ok(ids)
    }
}

pub struct FileMapStorage {
    id: String,
    root: PathBuf,
    compression: Compression,
    read_only: bool,
    locks: KeyLocks,
}

impl FileMapStorage {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn grid_cell_path(&self, grid: GridKey, tile: Tile) -> PathBuf {
        let (dir, suffix) = layout::grid_dir(&self.root, grid, self.compression);
        layout::cell_path(&dir, &suffix, tile)
    }

    pub fn item_path(&self, item: &ItemKey) -> PathBuf {
        layout::item_path(&self.root, item, self.compression)
    }

    fn writable(&self) -> Result<()> {
        if self.read_only { Err(Error::ReadOnly) } else { Ok(()) }
    }

    fn stored(&self, path: &Path, compression: Compression) -> Result<Option<Stored>> {
        Ok(fsops::read(path)?.map(|data| Stored { data, compression }))
    }
}

impl MapStorage for FileMapStorage {
    fn map_id(&self) -> &str {
        &self.id
    }

    fn compression(&self) -> Compression {
        self.compression
    }

    fn read_grid(&self, grid: GridKey, tile: Tile) -> Result<Option<Stored>> {
        self.stored(&self.grid_cell_path(grid, tile), self.grid_compression(grid))
    }

    fn write_grid_encoded(&self, grid: GridKey, tile: Tile, encoded: &[u8]) -> Result<()> {
        self.writable()?;
        fsops::write_atomic(&self.grid_cell_path(grid, tile), encoded)
    }

    fn delete_grid(&self, grid: GridKey, tile: Tile) -> Result<()> {
        self.writable()?;
        fsops::remove_file(&self.grid_cell_path(grid, tile))
    }

    fn grid_exists(&self, grid: GridKey, tile: Tile) -> Result<bool> {
        Ok(self.grid_cell_path(grid, tile).is_file())
    }

    fn list_grid(&self, grid: GridKey) -> Result<Vec<Tile>> {
        let (dir, suffix) = layout::grid_dir(&self.root, grid, self.compression);
        let mut files = Vec::new();
        fsops::walk_files(&dir, &mut files)?;
        Ok(files.iter().filter_map(|f| layout::parse_cell(f.strip_prefix(&dir).ok()?, &suffix)).collect())
    }

    fn grids(&self) -> Result<Vec<GridKey>> {
        let mut grids = Vec::new();
        let tiles = self.root.join(layout::TILES);
        if tiles.join("0").is_dir() {
            grids.push(GridKey::Hires);
        }
        if let Ok(entries) = fs::read_dir(&tiles) {
            for entry in entries.flatten() {
                if let Some(lod) = entry.file_name().to_str().and_then(layout::parse_lod_dir)
                    && entry.path().is_dir()
                {
                    grids.push(GridKey::Lowres(lod));
                }
            }
        }
        let rstate = self.root.join(layout::RSTATE);
        if rstate.is_dir() {
            grids.extend([GridKey::TileState, GridKey::ChunkState]);
        }
        if rstate.join("regions").is_dir() {
            grids.push(GridKey::RegionState);
        }
        grids.sort();
        Ok(grids)
    }

    fn read_item(&self, item: &ItemKey) -> Result<Option<Stored>> {
        self.stored(&self.item_path(item), self.item_compression(item))
    }

    fn write_item_encoded(&self, item: &ItemKey, encoded: &[u8]) -> Result<()> {
        self.writable()?;
        fsops::write_atomic(&self.item_path(item), encoded)
    }

    fn delete_item(&self, item: &ItemKey) -> Result<()> {
        self.writable()?;
        fsops::remove_file(&self.item_path(item))
    }

    fn item_exists(&self, item: &ItemKey) -> Result<bool> {
        Ok(self.item_path(item).is_file())
    }

    fn list_assets(&self) -> Result<Vec<String>> {
        let dir = self.root.join(layout::ASSETS);
        let mut files = Vec::new();
        fsops::walk_files(&dir, &mut files)?;
        let mut names: Vec<String> = files
            .iter()
            .filter_map(|f| {
                let rel = f.strip_prefix(&dir).ok()?.to_str()?.replace('\\', "/");
                (!rel.ends_with(fsops::TEMP_SUFFIX)).then_some(rel)
            })
            .collect();
        names.sort();
        Ok(names)
    }

    fn delete(&self, on_progress: &mut dyn FnMut(f64) -> bool) -> Result<()> {
        self.writable()?;
        let mut entries = Vec::new();
        fsops::walk_depth(&self.root, 3, &mut entries)?;
        let total = entries.len();
        // deepest-last order means popping removes children before parents
        while let Some(entry) = entries.pop() {
            fsops::remove_tree(&entry)?;
            if !on_progress(1.0 - entries.len() as f64 / total as f64) {
                return Ok(());
            }
        }
        fsops::remove_tree(&self.root)
    }

    fn exists(&self) -> Result<bool> {
        Ok(self.root.exists())
    }

    fn key_locks(&self) -> &KeyLocks {
        &self.locks
    }

    fn grid_version(&self, grid: GridKey, tile: Tile) -> Result<Option<Version>> {
        version::file_version(&self.grid_cell_path(grid, tile))
    }

    fn item_version(&self, item: &ItemKey) -> Result<Option<Version>> {
        version::file_version(&self.item_path(item))
    }

    fn read_grid_versioned(&self, grid: GridKey, tile: Tile) -> Result<Option<(Stored, Option<Version>)>> {
        version::read_stored(&self.grid_cell_path(grid, tile), self.grid_compression(grid))
    }

    fn read_item_versioned(&self, item: &ItemKey) -> Result<Option<(Stored, Option<Version>)>> {
        version::read_stored(&self.item_path(item), self.item_compression(item))
    }
}
