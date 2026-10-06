//! Render masks (`core/.../map/mask/**`): which blocks of the world a map renders.
//!
//! Areas are inclusive block bounds. `test_area` may answer `Undefined` for areas it cannot cheaply decide;
//! `is_edge` decides whether a rendered tile is marked as a map edge (re-rendered when that answer changes).

mod combined;
mod config;
mod shapes;

pub use combined::{BlurMask, CombinedMask};
pub use config::{MaskConfig, MaskConfigError, MaskShape, build_render_mask};
pub use shapes::{BoxMask, EllipseMask, PolygonMask};

use bm_format::grid::{Grid, Tile};

/// BlueMap's `util.Tristate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tristate {
    True,
    Undefined,
    False,
}

impl Tristate {
    pub fn from_bool(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    pub fn negated(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::Undefined => Self::Undefined,
            Self::False => Self::True,
        }
    }

    /// `and(Supplier)`: `other` is only evaluated when `self` is not `False`.
    pub fn and(self, other: impl FnOnce() -> Tristate) -> Self {
        match self {
            Self::True => other(),
            Self::Undefined if other() == Self::False => Self::False,
            Self::Undefined => Self::Undefined,
            Self::False => Self::False,
        }
    }

    pub fn get_or(self, undefined: bool) -> bool {
        match self {
            Self::True => true,
            Self::Undefined => undefined,
            Self::False => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mask {
    /// `Mask.NONE`: matches nothing.
    None,
    /// `Mask.ALL` (`NONE.inverted()`): matches everything, never an edge.
    All,
    Inverted(Box<Mask>),
    Box(BoxMask),
    Ellipse(EllipseMask),
    Polygon(PolygonMask),
    Blur(BlurMask),
    Combined(CombinedMask),
}

impl Mask {
    pub fn test(&self, x: i32, y: i32, z: i32) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Inverted(m) => !m.test(x, y, z),
            Self::Box(m) => m.test(x, y, z),
            Self::Ellipse(m) => m.test(x, y, z),
            Self::Polygon(m) => m.test(x, y, z),
            Self::Blur(m) => m.test(x, y, z),
            Self::Combined(m) => m.test(x, y, z),
        }
    }

    /// Java's `test(minX, minY, minZ, maxX, maxY, maxZ)`.
    pub fn test_area(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Tristate {
        match self {
            Self::None => Tristate::False,
            Self::All => Tristate::True,
            Self::Inverted(m) => m.test_area(min_x, min_y, min_z, max_x, max_y, max_z).negated(),
            Self::Box(m) => m.test_area(min_x, min_y, min_z, max_x, max_y, max_z),
            Self::Ellipse(m) => m.test_area(min_x, min_y, min_z, max_x, max_y, max_z),
            Self::Polygon(m) => m.test_area(min_x, min_y, min_z, max_x, max_y, max_z),
            Self::Blur(m) => m.test_area(min_x, min_y, min_z, max_x, max_y, max_z),
            Self::Combined(m) => m.test_area(min_x, min_y, min_z, max_x, max_y, max_z),
        }
    }

    pub fn is_edge(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        match self {
            Self::None | Self::All => false,
            Self::Inverted(m) => m.is_edge(min_x, min_z, max_x, max_z),
            Self::Box(m) => m.is_edge(min_x, min_z, max_x, max_z),
            Self::Ellipse(m) => m.is_edge(min_x, min_z, max_x, max_z),
            Self::Polygon(m) => m.is_edge(min_x, min_z, max_x, max_z),
            Self::Blur(m) => m.is_edge(min_x, min_z, max_x, max_z),
            Self::Combined(m) => m.is_edge(min_x, min_z, max_x, max_z),
        }
    }

    /// A mask equal to this one inside the area, simplified where possible (`All`/`None`, pruned layers).
    pub fn submask(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Mask {
        if let Self::Combined(m) = self {
            return m.submask(min_x, min_y, min_z, max_x, max_y, max_z);
        }
        match self.test_area(min_x, min_y, min_z, max_x, max_y, max_z) {
            Tristate::True => Self::All,
            Tristate::False => Self::None,
            Tristate::Undefined => self.clone(),
        }
    }

    pub fn inverted(self) -> Mask {
        match self {
            Self::None => Self::All,
            Self::All => Self::None,
            Self::Inverted(m) => *m,
            m => Self::Inverted(Box::new(m)),
        }
    }

    /// `RenderSettings.isInsideRenderBoundaries(x, z)`: the whole column, undecided counts as inside.
    pub fn is_column_inside(&self, x: i32, z: i32) -> bool {
        self.test_area(x, i32::MIN, z, x, i32::MAX, z).get_or(true)
    }

    /// `RenderSettings.isInsideRenderBoundaries(cell, grid, allowPartiallyIncludedCells)`.
    pub fn is_cell_inside(&self, grid: &Grid, (cx, cz): Tile, allow_partial: bool) -> bool {
        let min = |cell: i32, axis: usize| cell.wrapping_mul(grid.size[axis]).wrapping_add(grid.offset[axis]);
        let (min_x, min_z) = (min(cx, 0), min(cz, 1));
        let (max_x, max_z) = (min(cx.wrapping_add(1), 0).wrapping_sub(1), min(cz.wrapping_add(1), 1).wrapping_sub(1));
        self.test_area(min_x, i32::MIN, min_z, max_x, i32::MAX, max_z).get_or(allow_partial)
    }
}

impl Default for Mask {
    /// A map without `render-mask` entries renders everything.
    fn default() -> Self {
        Self::Combined(CombinedMask::default())
    }
}
