//! `maps/<id>/tile-versions/<lod>,<x0>,<z0>,<x1>,<z1>/…`: the storage versions of whole rectangles of tiles in one
//! reply, so the client script checks a view after a browser reload in one round trip instead of one per tile
//! (docs/18). Answers one `<lod> <x> <z> <version>` line per stored tile; `-` for a tile stored without a version.

use std::fmt::Write;

use bm_storage::{GridKey, MapStorage, Tile};

const MAX_RECTS: usize = 8;
/// About the most the webapp's sliders can show; each one costs what an `If-None-Match` costs.
const MAX_TILES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    lod: u32,
    x: (i32, i32),
    z: (i32, i32),
}

impl Rect {
    fn tiles(&self) -> Option<u64> {
        let span = |(a, b): (i32, i32)| u64::from(b.abs_diff(a)) + 1;
        span(self.x).checked_mul(span(self.z))
    }

    fn grid(&self) -> GridKey {
        if self.lod == 0 { GridKey::Hires } else { GridKey::Lowres(self.lod) }
    }

    fn cells(self) -> impl Iterator<Item = Tile> {
        (self.x.0..=self.x.1).flat_map(move |x| (self.z.0..=self.z.1).map(move |z| (x, z)))
    }
}

fn parse_rect(spec: &str) -> Option<Rect> {
    let mut parts = spec.split(',');
    let lod = parts.next()?.parse().ok()?;
    let mut int = || parts.next()?.parse::<i32>().ok();
    let (x0, z0, x1, z1) = (int()?, int()?, int()?, int()?);
    (parts.next().is_none() && x0 <= x1 && z0 <= z1).then_some(Rect { lod, x: (x0, x1), z: (z0, z1) })
}

/// `None` if malformed or over the limits.
pub(crate) fn parse(specs: &str) -> Option<Vec<Rect>> {
    let rects = specs.split('/').map(parse_rect).collect::<Option<Vec<_>>>()?;
    let total = rects.iter().try_fold(0u64, |sum, r| sum.checked_add(r.tiles()?))?;
    (rects.len() <= MAX_RECTS && total <= MAX_TILES).then_some(rects)
}

/// Blocking.
pub(crate) fn body(storage: &dyn MapStorage, rects: &[Rect]) -> Result<String, bm_storage::Error> {
    let mut out = String::new();
    for rect in rects {
        let grid = rect.grid();
        for tile @ (x, z) in rect.cells() {
            let line = match storage.grid_version(grid, tile)? {
                Some(version) => writeln!(out, "{} {x} {z} {version}", rect.lod),
                // no version doesn't mean no tile: a missing line makes the script answer 204
                None if storage.grid_exists(grid, tile)? => writeln!(out, "{} {x} {z} -", rect.lod),
                None => Ok(()),
            };
            line.expect("writing to a String");
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rectangles_within_the_limits() {
        assert_eq!(parse("0,-1,-2,1,2").unwrap(), [Rect { lod: 0, x: (-1, 1), z: (-2, 2) }]);
        assert_eq!(parse("0,0,0,0,0/2,1,1,3,3").unwrap().len(), 2);
        for bad in [
            "",
            "0",
            "0,0,0,0",
            "0,0,0,0,0,0",
            "x,0,0,0,0",
            "0,1,0,0,0",
            "0,0,1,0,0",
            "0,0,0,63,63/0,0,0,0,0",
            "0,-2147483648,-2147483648,2147483647,2147483647",
            "0,0,0,0,0/0,0,0,0,0/0,0,0,0,0/0,0,0,0,0/0,0,0,0,0/0,0,0,0,0/0,0,0,0,0/0,0,0,0,0/0,0,0,0,0",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert!(parse("0,0,0,63,63").is_some());
    }
}
