//! A map in a BlueMap file-storage webroot (`<webroot>/maps/<id>/…`), read straight from disk.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bm_compress::Compression;
use bm_format::grid::{Tile, parse_tile_path, tile_file};

use crate::settings::{MapSettings, SiteSettings};

/// Bigger than any tile BlueMap writes (it caps a hires tile at 1M triangles).
const LIMIT: usize = 1 << 30;

pub struct WebrootMap {
    pub id: String,
    pub settings: MapSettings,
    dir: PathBuf,
}

impl WebrootMap {
    /// `id` may be omitted when the webroot holds exactly one map.
    pub fn open(webroot: &Path, id: Option<&str>) -> Result<Self> {
        let site: SiteSettings = serde_json::from_slice(&read(&webroot.join("settings.json"))?)?;
        let id = match (id, site.maps.as_slice()) {
            (Some(id), _) => id.to_owned(),
            (None, [only]) => only.clone(),
            (None, maps) => bail!("webroot has maps {maps:?}; pick one with --map"),
        };
        let dir = webroot.join(&site.map_data_root).join(&id);
        let settings = serde_json::from_slice(&read_decompressed(&dir.join("settings.json"))?)
            .with_context(|| format!("{id}/settings.json"))?;
        Ok(Self { id, settings, dir })
    }

    /// Tiles stored at `lod` (0 = hires).
    pub fn tiles(&self, lod: u32) -> Result<BTreeSet<Tile>> {
        let root = self.dir.join(format!("tiles/{lod}"));
        let ext = if lod == 0 { ".prbm" } else { ".png" };
        let mut tiles = BTreeSet::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e).context(dir.display().to_string()),
            };
            for entry in entries {
                let path = entry?.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let rel = path.strip_prefix(&root)?.to_string_lossy().replace('\\', "/");
                let is_tile = Compression::ALL.iter().any(|c| rel.ends_with(&format!("{ext}{}", c.file_suffix())));
                if let Some(t) = parse_tile_path(&rel).filter(|_| is_tile) {
                    tiles.insert(t);
                }
            }
        }
        Ok(tiles)
    }

    pub fn tile_bytes(&self, lod: u32, t: Tile) -> Result<Vec<u8>> {
        read_decompressed(&self.dir.join(tile_file(lod, t)))
    }

    pub fn textures_json(&self) -> Result<Vec<u8>> {
        read_decompressed(&self.dir.join("textures.json"))
    }

    /// World x/z of a hires tile's min corner.
    pub fn hires_origin(&self, t: Tile) -> [i32; 2] {
        let (x, z) = self.settings.hires_grid().tile_min(t);
        [x, z]
    }
}

fn read(p: &Path) -> Result<Vec<u8>> {
    fs::read(p).with_context(|| p.display().to_string())
}

/// `path` or `path<suffix>` for whichever compression the storage used.
fn read_decompressed(path: &Path) -> Result<Vec<u8>> {
    for c in Compression::ALL {
        let mut name = path.as_os_str().to_owned();
        name.push(c.file_suffix());
        match fs::read(&name) {
            Ok(bytes) => return c.decompress(&bytes, LIMIT).with_context(|| path.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).context(path.display().to_string()),
        }
    }
    bail!("{} not found with any compression suffix", path.display())
}
