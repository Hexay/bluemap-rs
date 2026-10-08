//! A box of blocks and the index math of the dense per-tile copies in [`crate::view`].

#[derive(Default)]
pub(crate) struct Bounds {
    origin: [i32; 3],
    pub size: [i32; 3],
}

impl Bounds {
    pub fn new(min: [i32; 3], max: [i32; 3]) -> Self {
        Self { origin: min, size: [max[0] - min[0] + 1, max[1] - min[1] + 1, max[2] - min[2] + 1] }
    }

    pub fn len(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    /// Columns are contiguous in y.
    pub fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let [ox, oy, oz] = self.origin;
        let [w, h, d] = self.size;
        let (dx, dy, dz) = (x.wrapping_sub(ox), y.wrapping_sub(oy), z.wrapping_sub(oz));
        let inside = (dx as u32) < w as u32 && (dy as u32) < h as u32 && (dz as u32) < d as u32;
        inside.then(|| ((dx * d + dz) * h + dy) as usize)
    }

    /// [`Bounds::index`] for positions whose 26 neighbours are inside as well.
    pub fn interior_index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let [ox, oy, oz] = self.origin;
        let [w, h, d] = self.size;
        let (dx, dy, dz) = (x.wrapping_sub(ox), y.wrapping_sub(oy), z.wrapping_sub(oz));
        let interior = |p: i32, s: i32| p >= 1 && p < s - 1;
        (interior(dx, w) && interior(dy, h) && interior(dz, d)).then(|| ((dx * d + dz) * h + dy) as usize)
    }
}
