//! Layered `CombinedMask` (last matching layer wins) and the jittering `BlurMask` around one.

use super::{Mask, Tristate};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CombinedMask {
    layers: Vec<(Mask, bool)>,
}

impl CombinedMask {
    /// A subtracting first layer implies "everything" below it, so an `All` layer is inserted first.
    pub fn add(&mut self, mask: Mask, value: bool) {
        if !value && self.layers.is_empty() {
            self.layers.push((Mask::All, true));
        }
        self.layers.push((mask, value));
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    pub fn layers(&self) -> &[(Mask, bool)] {
        &self.layers
    }

    pub fn test(&self, x: i32, y: i32, z: i32) -> bool {
        match self.layers.iter().rev().find(|(mask, _)| mask.test(x, y, z)) {
            Some(&(_, value)) => value,
            None => self.layers.is_empty(),
        }
    }

    pub fn test_area(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Tristate {
        for (mask, value) in self.layers.iter().rev() {
            match mask.test_area(min_x, min_y, min_z, max_x, max_y, max_z) {
                Tristate::False => continue,
                Tristate::Undefined => return Tristate::Undefined,
                Tristate::True => return Tristate::from_bool(*value),
            }
        }
        Tristate::from_bool(self.layers.is_empty())
    }

    pub fn is_edge(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        self.layers.iter().rev().any(|(mask, _)| mask.is_edge(min_x, min_z, max_x, max_z))
    }

    /// Drops layers that miss the area; the first layer is always kept (Java's quirk), each kept one is submasked.
    pub fn submask(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Mask {
        match self.test_area(min_x, min_y, min_z, max_x, max_y, max_z) {
            Tristate::True => return Mask::All,
            Tristate::False => return Mask::None,
            Tristate::Undefined => {}
        }
        let mut optimized = CombinedMask::default();
        for (mask, value) in &self.layers {
            if !optimized.is_empty() && mask.test_area(min_x, min_y, min_z, max_x, max_y, max_z) == Tristate::False {
                continue;
            }
            optimized.add(mask.submask(min_x, min_y, min_z, max_x, max_y, max_z), *value);
        }
        Mask::Combined(optimized)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlurMask {
    pub masks: CombinedMask,
    pub size: i32,
}

impl BlurMask {
    pub fn test(&self, x: i32, y: i32, z: i32) -> bool {
        self.masks.test(
            x.wrapping_add(self.random_offset(x, y, z, 23948)),
            y.wrapping_add(self.random_offset(x, y, z, 53242)),
            z.wrapping_add(self.random_offset(x, y, z, 75654)),
        )
    }

    pub fn test_area(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Tristate {
        let s = self.size;
        self.masks.test_area(
            min_x.wrapping_sub(s),
            min_y.wrapping_sub(s),
            min_z.wrapping_sub(s),
            max_x.wrapping_add(s),
            max_y.wrapping_add(s),
            max_z.wrapping_add(s),
        )
    }

    pub fn is_edge(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        let s = self.size;
        self.masks.is_edge(min_x.wrapping_sub(s), min_z.wrapping_sub(s), max_x.wrapping_add(s), max_z.wrapping_add(s))
    }

    /// Hash noise in `(-size, size)`, truncated toward zero; float math as in Java.
    fn random_offset(&self, x: i32, y: i32, z: i32, seed: i64) -> i32 {
        let hash = (x as i64).wrapping_mul(73428767)
            ^ (y as i64).wrapping_mul(4382893)
            ^ (z as i64).wrapping_mul(2937119)
            ^ seed.wrapping_mul(457);
        let noise = (hash.wrapping_mul(hash.wrapping_add(456149)) & 0x00ff_ffff) as f32 / 0x0100_0000 as f32;
        ((noise - 0.5) * 2.0 * self.size as f32) as i32
    }
}
