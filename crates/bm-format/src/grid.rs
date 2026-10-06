//! Tile grids and BlueMap's digit-split tile paths (`FileGridStorage.java`, webapp `pathFromCoords`).

pub type Tile = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub size: [i32; 2],
    pub offset: [i32; 2],
}

impl Grid {
    pub fn tile_of(&self, x: i32, z: i32) -> Tile {
        ((x - self.offset[0]).div_euclid(self.size[0]), (z - self.offset[1]).div_euclid(self.size[1]))
    }

    pub fn tile_min(&self, (tx, tz): Tile) -> (i32, i32) {
        (tx * self.size[0] + self.offset[0], tz * self.size[1] + self.offset[1])
    }

    /// Tiles overlapping the block rectangle [x0, x0+w) × [z0, z0+h).
    pub fn tiles_in(&self, x0: i32, z0: i32, w: i32, h: i32) -> impl Iterator<Item = Tile> {
        let (ax, az) = self.tile_of(x0, z0);
        let (bx, bz) = self.tile_of(x0 + w - 1, z0 + h - 1);
        (ax..=bx).flat_map(move |tx| (az..=bz).map(move |tz| (tx, tz)))
    }
}

/// Tile file relative to the map dir, without the compression suffix; lod 0 = hires `.prbm`, else lowres `.png`.
pub fn tile_file(lod: u32, t: Tile) -> String {
    let ext = if lod == 0 { "prbm" } else { "png" };
    format!("tiles/{lod}/{}.{ext}", tile_path(t))
}

/// `x=-12, z=345` → `x-1/2/z3/4/5`.
pub fn tile_path((x, z): Tile) -> String {
    let mut s = String::from("x");
    push_split(&mut s, x);
    s.push('z');
    push_split(&mut s, z);
    s.pop();
    s
}

fn push_split(s: &mut String, n: i32) {
    if n < 0 {
        s.push('-');
    }
    for c in n.unsigned_abs().to_string().chars() {
        s.push(c);
        s.push('/');
    }
}

/// Inverse of [`tile_path`]: separators are ignored and anything after the z digits (extension) is allowed,
/// like BlueMap's `x(-?\d+)z(-?\d+)` match.
pub fn parse_tile_path(path: &str) -> Option<Tile> {
    let flat: String = path.chars().filter(|&c| c != '/' && c != '\\').collect();
    let rest = flat.strip_prefix('x')?;
    let (x, rest) = split_int(rest)?;
    let (z, _) = split_int(rest.strip_prefix('z')?)?;
    Some((x, z))
}

fn split_int(s: &str) -> Option<(i32, &str)> {
    let sign = usize::from(s.starts_with('-'));
    let end = sign + s[sign..].find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len() - sign);
    Some((s[..end].parse().ok()?, &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_split_paths() {
        assert_eq!(tile_path((-12, 345)), "x-1/2/z3/4/5");
        assert_eq!(tile_path((0, 7)), "x0/z7");
        assert_eq!(tile_path((-3, -1)), "x-3/z-1");
        assert_eq!(tile_file(0, (103, -7)), "tiles/0/x1/0/3/z-7.prbm");
    }

    #[test]
    fn paths_parse_back() {
        for t in [(-12, 345), (0, 7), (-3, -1), (i32::MIN + 1, i32::MAX)] {
            assert_eq!(parse_tile_path(&tile_path(t)), Some(t));
        }
        assert_eq!(parse_tile_path("x1\\0\\3\\z-7.prbm.gz"), Some((103, -7)));
        assert_eq!(parse_tile_path("x-/z1"), None);
        assert_eq!(parse_tile_path("settings.json"), None);
    }

    #[test]
    fn hires_grid_offset() {
        let g = Grid { size: [32, 32], offset: [2, 2] };
        assert_eq!(g.tile_of(2, 33), (0, 0));
        assert_eq!(g.tile_of(1, 34), (-1, 1));
        assert_eq!(g.tile_of(-64, 63), (-3, 1));
        assert_eq!(g.tile_min((-3, 1)), (-94, 34));
    }

    #[test]
    fn tiles_in_rect() {
        let g = Grid { size: [500, 500], offset: [0, 0] };
        let v: Vec<_> = g.tiles_in(-25, 475, 50, 50).collect();
        assert_eq!(v, vec![(-1, 0), (-1, 1), (0, 0), (0, 1)]);
    }
}
