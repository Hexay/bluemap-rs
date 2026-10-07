//! A map's render mask as the mesher reads it (`RenderSettings.isInsideRenderBoundaries`).

use std::sync::Arc;

use bm_map::mask::{Mask, Tristate};
use bm_render::RenderMask;

struct MaskView(Mask);

impl RenderMask for MaskView {
    fn test(&self, x: i32, y: i32, z: i32) -> bool {
        self.0.test(x, y, z)
    }

    fn test_column(&self, x: i32, z: i32) -> bool {
        self.0.is_column_inside(x, z)
    }

    fn test_area(&self, min: [i32; 2], max: [i32; 2]) -> Option<bool> {
        match self.0.test_area(min[0], i32::MIN, min[1], max[0], i32::MAX, max[1]) {
            Tristate::True => Some(true),
            Tristate::False => Some(false),
            Tristate::Undefined => None,
        }
    }
}

/// `None` when the mask lets every block through, so the mesher skips mask tests entirely.
pub fn render_mask(mask: &Mask) -> Option<Arc<dyn RenderMask>> {
    let everything = mask.test_area(i32::MIN, i32::MIN, i32::MIN, i32::MAX, i32::MAX, i32::MAX) == Tristate::True;
    (!everything).then(|| Arc::new(MaskView(mask.clone())) as Arc<dyn RenderMask>)
}
