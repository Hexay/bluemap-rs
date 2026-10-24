//! Bundle states of one store, least recently used dropped past [`CACHE_CAP`]: a map of ~340k bundles is served
//! from a bounded working set instead of a cache that empties whenever it fills.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bm_format::grid::Tile;

use super::Bundle;

/// Cached bundle states; each index holds ≤ 256 entries (~10 KB), so the cache stays under ~40 MB.
pub(super) const CACHE_CAP: usize = 4096;

#[derive(Default)]
pub(super) struct BundleCache {
    map: HashMap<Tile, (Arc<Mutex<Bundle>>, u64)>,
    tick: u64,
}

impl BundleCache {
    pub fn get_or_insert(&mut self, bundle: Tile) -> Arc<Mutex<Bundle>> {
        self.tick += 1;
        if let Some((state, used)) = self.map.get_mut(&bundle) {
            *used = self.tick;
            return state.clone();
        }
        if self.map.len() >= CACHE_CAP {
            self.evict();
        }
        let state = Arc::new(Mutex::new(Bundle::default()));
        self.map.insert(bundle, (state.clone(), self.tick));
        state
    }

    /// The cached state, without counting as a use.
    pub fn get(&self, bundle: Tile) -> Option<Arc<Mutex<Bundle>>> {
        self.map.get(&bundle).map(|(state, _)| state.clone())
    }

    pub fn drain(&mut self) -> Vec<Arc<Mutex<Bundle>>> {
        self.map.drain().map(|(_, (state, _))| state).collect()
    }

    /// Drops the least recently used eighth, skipping states someone holds right now (an append or read in
    /// progress); a dropped state closes its write handle and is rebuilt from the file on next use.
    fn evict(&mut self) {
        let mut idle: Vec<(u64, Tile)> =
            self.map.iter().filter(|(_, (s, _))| Arc::strong_count(s) == 1).map(|(&t, &(_, used))| (used, t)).collect();
        let n = (CACHE_CAP / 8).min(idle.len());
        if n == 0 {
            return;
        }
        idle.select_nth_unstable(n - 1);
        for (_, t) in &idle[..n] {
            self.map.remove(t);
        }
    }
}
