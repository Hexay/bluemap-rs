//! Upstream file layout (`FileMapStorage.java`, `FileGridStorage.java`); see docs/04-storage-web.md §1.

use std::path::{Path, PathBuf};

use bm_compress::Compression;
use bm_format::grid::{Tile, tile_path};

use crate::key::{GridKey, ItemKey, escape_asset_name};

pub(crate) const TILES: &str = "tiles";
pub(crate) const RSTATE: &str = "rstate";
pub(crate) const ASSETS: &str = "assets";

/// Directory holding the grid and the file name suffix of its cells.
pub(crate) fn grid_dir(map_root: &Path, grid: GridKey, configured: Compression) -> (PathBuf, String) {
    match grid {
        GridKey::Hires => (map_root.join(TILES).join("0"), format!(".prbm{}", configured.file_suffix())),
        GridKey::Lowres(lod) => (map_root.join(TILES).join(lod.to_string()), ".png".into()),
        GridKey::TileState => (map_root.join(RSTATE), ".tiles.dat".into()),
        GridKey::ChunkState => (map_root.join(RSTATE), ".chunks.dat".into()),
        GridKey::RegionState => (map_root.join(RSTATE).join("regions"), ".regions.dat".into()),
    }
}

/// `x=-12, z=5` → `<dir>/x-1/2/z5<suffix>`.
pub(crate) fn cell_path(dir: &Path, suffix: &str, tile: Tile) -> PathBuf {
    let mut path = dir.to_path_buf();
    let rel = tile_path(tile);
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            path.push(part);
        } else {
            path.push(format!("{part}{suffix}"));
        }
    }
    path
}

pub(crate) fn item_path(map_root: &Path, item: &ItemKey, configured: Compression) -> PathBuf {
    match item {
        ItemKey::Settings => map_root.join("settings.json"),
        ItemKey::Textures => map_root.join(format!("textures.json{}", configured.file_suffix())),
        ItemKey::Markers => map_root.join("live").join("markers.json"),
        ItemKey::Players => map_root.join("live").join("players.json"),
        ItemKey::Asset(name) => {
            let mut path = map_root.join(ASSETS);
            // String.split drops empty parts' effect: Path.resolve("") is a no-op in Java
            escape_asset_name(name).split('/').filter(|p| !p.is_empty()).for_each(|p| path.push(p));
            path
        }
    }
}

/// Inverse of [`cell_path`] for a path relative to the grid dir: Java strips the suffix, removes separators and
/// requires a full `x(-?\d+)z(-?\d+)` match.
pub(crate) fn parse_cell(rel: &Path, suffix: &str) -> Option<Tile> {
    let rel = rel.to_str()?;
    let flat: String = rel.strip_suffix(suffix)?.chars().filter(|&c| c != '/' && c != '\\').collect();
    let rest = flat.strip_prefix('x')?;
    let (x, rest) = split_int(rest)?;
    let (z, rest) = split_int(rest.strip_prefix('z')?)?;
    rest.is_empty().then_some((x, z))
}

fn split_int(s: &str) -> Option<(i32, &str)> {
    let sign = usize::from(s.starts_with('-'));
    let digits = s[sign..].bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let end = sign + digits;
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// Lowres LOD directories: `tiles/<n>` with `n >= 1`.
pub(crate) fn parse_lod_dir(name: &str) -> Option<u32> {
    let lod: u32 = name.parse().ok()?;
    (lod >= 1 && lod.to_string() == name).then_some(lod)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_match_upstream() {
        let root = Path::new("m");
        let (dir, sfx) = grid_dir(root, GridKey::Hires, Compression::Gzip);
        assert_eq!(cell_path(&dir, &sfx, (-12, 5)), Path::new("m/tiles/0/x-1/2/z5.prbm.gz"));
        let (dir, sfx) = grid_dir(root, GridKey::Lowres(2), Compression::Gzip);
        assert_eq!(cell_path(&dir, &sfx, (103, -7)), Path::new("m/tiles/2/x1/0/3/z-7.png"));
        let (dir, sfx) = grid_dir(root, GridKey::RegionState, Compression::Zstd);
        assert_eq!(cell_path(&dir, &sfx, (0, -1)), Path::new("m/rstate/regions/x0/z-1.regions.dat"));
        let (dir, sfx) = grid_dir(root, GridKey::ChunkState, Compression::None);
        assert_eq!(cell_path(&dir, &sfx, (0, 0)), Path::new("m/rstate/x0/z0.chunks.dat"));
        assert_eq!(item_path(root, &ItemKey::Textures, Compression::Zstd), Path::new("m/textures.json.zst"));
        assert_eq!(item_path(root, &ItemKey::Textures, Compression::None), Path::new("m/textures.json"));
        assert_eq!(item_path(root, &ItemKey::Players, Compression::Gzip), Path::new("m/live/players.json"));
        assert_eq!(
            item_path(root, &ItemKey::asset("/playerheads//../a b.png"), Compression::Gzip),
            Path::new("m/assets/playerheads/_./a_b.png")
        );
    }

    #[test]
    fn cells_parse_strictly() {
        assert_eq!(parse_cell(Path::new("x-1/2/z5.prbm.gz"), ".prbm.gz"), Some((-12, 5)));
        assert_eq!(parse_cell(Path::new("x0\\z0.tiles.dat"), ".tiles.dat"), Some((0, 0)));
        assert_eq!(parse_cell(Path::new("x0/0/7/z1.png"), ".png"), Some((7, 1)));
        assert_eq!(parse_cell(Path::new("x0/z0.prbm.gz.filepart"), ".prbm.gz"), None);
        assert_eq!(parse_cell(Path::new("x0/z0.prbm"), ".prbm.gz"), None);
        assert_eq!(parse_cell(Path::new("x0/z0.extra.png"), ".png"), None);
        assert_eq!(parse_cell(Path::new("x9/9/9/9/9/9/9/9/9/9/9/z0.png"), ".png"), None);
        assert_eq!(parse_lod_dir("3"), Some(3));
        assert_eq!(parse_lod_dir("0"), None);
        assert_eq!(parse_lod_dir("03"), None);
    }
}
