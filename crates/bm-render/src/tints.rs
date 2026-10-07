//! Blended tints of blocks whose 75 blend samples all lie in one biome: the same for every such block of a state.

use std::cell::RefCell;

use bm_math::Color;
use bm_world::{BiomeId, StateId};

/// Per tile, by state and biome; `None` where the state's tint has no uniform form.
#[derive(Default)]
pub(crate) struct UniformTints(RefCell<Vec<(StateId, BiomeId, Option<Color>)>>);

impl UniformTints {
    pub fn clear(&mut self) {
        self.0.get_mut().clear();
    }

    pub fn get_or_insert_with(
        &self,
        state: StateId,
        biome: BiomeId,
        compute: impl FnOnce() -> Option<Color>,
    ) -> Option<Color> {
        if let Some(&(.., c)) = self.0.borrow().iter().find(|(s, b, _)| *s == state && *b == biome) {
            return c;
        }
        let c = compute();
        self.0.borrow_mut().push((state, biome, c));
        c
    }
}
