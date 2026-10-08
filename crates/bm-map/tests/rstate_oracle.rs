//! Oracle: every rstate file Java BlueMap wrote under the golden webroots must survive parse → write with
//! byte-identical NBT, and its contents must agree with the rendered tiles. Read-only; run with `--ignored`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bm_format::grid::{Tile, parse_tile_path};
use bm_map::renderstate::{Cell, CellKind, ChunkInfoRegion, RegionInfoRegion, TileInfoRegion, TileState};

/// 2020-01-01: anything older is not a BlueMap 5 timestamp.
const MIN_TIME: i64 = 1_577_836_800;

/// Golden webroots: this repo's `work/bluemap`, plus a sibling `bluemap_reverse` checkout's when present.
fn roots() -> Vec<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let find = |rel: &str| here.ancestors().map(|a| a.join(rel)).find(|p| p.is_dir());
    [find("work/bluemap"), find("../bluemap_reverse/work/bluemap")].into_iter().flatten().collect()
}

fn map_dirs() -> Vec<PathBuf> {
    let mut maps = Vec::new();
    for root in roots() {
        for fixture in read_dir(&root) {
            maps.extend(read_dir(&fixture.join("web/maps")).into_iter().filter(|m| m.join("rstate").is_dir()));
        }
    }
    maps
}

fn read_dir(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for p in read_dir(dir) {
        if p.is_dir() { files_under(&p, out) } else { out.push(p) }
    }
}

fn kind_of(path: &Path) -> Option<CellKind> {
    let name = path.file_name()?.to_str()?;
    CellKind::ALL.into_iter().find(|k| name.ends_with(k.suffix()))
}

fn cell_of(map: &Path, kind: CellKind, path: &Path) -> Tile {
    let rel = path.strip_prefix(map.join(kind.dir())).unwrap().to_string_lossy().replace('\\', "/");
    let cell = parse_tile_path(&rel).unwrap_or_else(|| panic!("bad cell path {rel}"));
    assert_eq!(kind.relative_path(cell), format!("{}/{rel}", kind.dir()));
    cell
}

fn round_trip<C: Cell>(stored: &[u8]) -> (C, bool) {
    let original = bm_compress::Compression::Gzip.decompress(stored, 64 << 20).unwrap();
    let cell = C::decode(stored).unwrap();
    let ok = cell.to_nbt() == original && C::decode(&cell.encode().unwrap()).unwrap().to_nbt() == original;
    (cell, ok)
}

#[derive(Default)]
struct Stats {
    files: [usize; 3],
    equal: [usize; 3],
    tiles_with_hires: usize,
    hires_not_rendered: Vec<String>,
    rendered_without_hires: usize,
    implausible_times: Vec<String>,
    states: [usize; 9],
}

fn plausible(t: i32, now: i64) -> bool {
    t == 0 || (MIN_TIME..=now + 86_400).contains(&i64::from(t))
}

#[test]
#[ignore = "needs golden webroots under work/"]
fn java_rstate_round_trips_byte_equal() {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let mut s = Stats::default();
    for map in map_dirs() {
        let hires = hires_tiles(&map);
        let mut files = Vec::new();
        files_under(&map.join("rstate"), &mut files);
        for path in files {
            let Some(kind) = kind_of(&path) else { continue };
            let k = kind as usize;
            let cell = cell_of(&map, kind, &path);
            let stored = fs::read(&path).unwrap();
            s.files[k] += 1;
            let (ok, times) = match kind {
                CellKind::Tiles => {
                    let (c, ok) = round_trip::<TileInfoRegion>(&stored);
                    check_tiles(&map, &hires, cell, &c, &mut s);
                    (ok, tile_times(&c))
                }
                CellKind::Chunks => {
                    let (c, ok) = round_trip::<ChunkInfoRegion>(&stored);
                    (ok, int_values(|x, z| c.get(x, z), kind))
                }
                CellKind::Regions => {
                    let (c, ok) = round_trip::<RegionInfoRegion>(&stored);
                    (ok, int_values(|x, z| c.get(x, z), kind))
                }
            };
            s.equal[k] += usize::from(ok);
            if !ok {
                eprintln!("NOT EQUAL: {}", path.display());
            }
            if let Some(t) = times.into_iter().find(|&t| !plausible(t, now)) {
                s.implausible_times.push(format!("{} ({t})", path.display()));
            }
        }
    }
    eprintln!("files tiles/chunks/regions: {:?}, byte-equal: {:?}", s.files, s.equal);
    eprintln!("state histogram {:?}: {:?}", TileState::ALL, s.states);
    eprintln!(
        "tiles with hires: {}, rendered without hires: {}, hires but not rendered: {}",
        s.tiles_with_hires,
        s.rendered_without_hires,
        s.hires_not_rendered.len()
    );
    assert!(s.files.iter().sum::<usize>() > 0, "no oracle files found");
    assert_eq!(s.files, s.equal);
    assert!(s.hires_not_rendered.is_empty(), "{:?}", &s.hires_not_rendered[..s.hires_not_rendered.len().min(10)]);
    assert!(s.implausible_times.is_empty(), "{:?}", s.implausible_times);
}

fn entries_of(kind: CellKind) -> impl Iterator<Item = (i32, i32)> {
    let len = 1 << kind.shift();
    (0..len).flat_map(move |z| (0..len).map(move |x| (x, z)))
}

fn tile_times(c: &TileInfoRegion) -> Vec<i32> {
    entries_of(CellKind::Tiles).map(|(x, z)| c.get(x, z).render_time).collect()
}

fn int_values(get: impl Fn(i32, i32) -> i32, kind: CellKind) -> Vec<i32> {
    entries_of(kind).map(|(x, z)| get(x, z)).collect()
}

fn hires_tiles(map: &Path) -> HashSet<Tile> {
    let root = map.join("tiles/0");
    let mut files = Vec::new();
    files_under(&root, &mut files);
    let rel = |p: &Path| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
    files.iter().map(|p| rel(p)).filter(|r| r.contains(".prbm")).filter_map(|r| parse_tile_path(&r)).collect()
}

fn check_tiles(map: &Path, hires: &HashSet<Tile>, (cx, cz): Tile, c: &TileInfoRegion, s: &mut Stats) {
    let shift = CellKind::Tiles.shift();
    for (x, z) in entries_of(CellKind::Tiles) {
        let tile = ((cx << shift) + x, (cz << shift) + z);
        let info = c.get(tile.0, tile.1);
        s.states[TileState::ALL.iter().position(|&t| t == info.state).unwrap()] += 1;
        let has_hires = hires.contains(&tile);
        let rendered = matches!(info.state, TileState::Rendered | TileState::RenderedEdge);
        s.tiles_with_hires += usize::from(has_hires);
        s.rendered_without_hires += usize::from(rendered && !has_hires);
        if has_hires && !rendered {
            s.hires_not_rendered.push(format!("{} {tile:?} {:?}", map.display(), info.state));
        }
    }
}
