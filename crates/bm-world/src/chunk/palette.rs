//! Palette entries resolved by their NBT bytes: the same few dozen entries repeat in every section of every chunk,
//! and resolving one means UTF-8 checks, a state key string and a lookup under the registry lock.

use std::cell::RefCell;

use bm_nbt::Tag;
use rustc_hash::FxHashMap;

use crate::registry::StateId;

/// Entries held before the cache starts over; bounds it on worlds with many states.
const MAX_ENTRIES: usize = 1 << 14;

/// One per thread (see [`super::ChunkContext::palette`]). Valid while the registry's block defaults stay as they
/// are, i.e. for one area load.
#[derive(Default)]
pub struct PaletteCache {
    compounds: RefCell<FxHashMap<Box<[u8]>, StateId>>,
    names: RefCell<FxHashMap<Box<[u8]>, StateId>>,
}

impl PaletteCache {
    pub(super) fn get_or(&self, tag: Tag, resolve: impl FnOnce() -> StateId) -> StateId {
        let (map, key) = match tag {
            Tag::Compound(c) => (&self.compounds, c.bytes()),
            Tag::String(s) => (&self.names, s),
            _ => return resolve(),
        };
        if let Some(&id) = map.borrow().get(key) {
            return id;
        }
        let id = resolve();
        let mut map = map.borrow_mut();
        if map.len() >= MAX_ENTRIES {
            map.clear();
        }
        map.insert(key.into(), id);
        id
    }
}
