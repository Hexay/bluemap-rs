//! Render-state cells compared by meaning: tile states (not render times), chunk hashes exactly, and which regions
//! have an update time (not the time itself).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use bm_compress::Compression;
use bm_format::grid::{Tile, parse_tile_path};
use bm_map::renderstate::{Cell, CellKind, ChunkInfoRegion, RegionInfoRegion, TileInfoRegion};

use super::Section;

pub fn compare(golden: &Path, candidate: &Path, section: &mut Section) -> Result<()> {
    let tiles = differing::<TileInfoRegion>(golden, candidate, |c, x, z| format!("{:?}", c.get(x, z).state))?;
    let chunks = differing::<ChunkInfoRegion>(golden, candidate, |c, x, z| c.get(x, z).to_string())?;
    let regions = differing::<RegionInfoRegion>(golden, candidate, |c, x, z| (c.get(x, z) != 0).to_string())?;
    for (what, (checked, bad)) in [("tile states", tiles), ("chunk hashes", chunks), ("region entries", regions)] {
        section.check(bad.is_empty(), format!("rstate {what}: {checked} cells, {} entries differ", bad.len()));
        bad.iter().take(10).for_each(|b| section.detail(b.clone()));
    }
    Ok(())
}

/// Cells of grid `C` present in either map dir, and every entry where `key` differs.
fn differing<C: Cell>(
    golden: &Path,
    candidate: &Path,
    key: impl Fn(&C, i32, i32) -> String,
) -> Result<(usize, Vec<String>)> {
    let kind = C::KIND;
    let cells: BTreeSet<Tile> = list(golden, kind)?.union(&list(candidate, kind)?).copied().collect();
    let mut bad = Vec::new();
    let n = 1 << kind.shift();
    for &cell in &cells {
        let (g, c) = (load::<C>(golden, kind, cell)?, load::<C>(candidate, kind, cell)?);
        for lx in 0..n {
            for lz in 0..n {
                let (x, z) = ((cell.0 << kind.shift()) + lx, (cell.1 << kind.shift()) + lz);
                let (a, b) = (key(&g, x, z), key(&c, x, z));
                if a != b {
                    bad.push(format!("{kind:?} {x},{z}: golden {a}, candidate {b}"));
                }
            }
        }
    }
    Ok((cells.len(), bad))
}

fn list(map_dir: &Path, kind: CellKind) -> Result<BTreeSet<Tile>> {
    let dir = map_dir.join(kind.dir());
    let mut out = BTreeSet::new();
    for rel in super::walk(&dir)? {
        // tile and chunk cells share `rstate/`; the suffix tells them apart
        out.extend(rel.strip_suffix(kind.suffix()).and_then(parse_tile_path));
    }
    Ok(out)
}

/// A missing cell reads as all defaults, like BlueMap.
fn load<C: Cell>(map_dir: &Path, kind: CellKind, cell: Tile) -> Result<C> {
    let path = map_dir.join(kind.relative_path(cell));
    match std::fs::read(&path) {
        Ok(bytes) => {
            C::from_nbt(&Compression::Gzip.decompress(&bytes, 1 << 26)?).with_context(|| path.display().to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(C::new()),
        Err(e) => Err(e).context(path.display().to_string()),
    }
}
