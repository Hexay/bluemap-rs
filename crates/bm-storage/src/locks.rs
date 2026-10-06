//! Exact-key exclusive locks: one writer per cell/item without striping (striped locks can self-deadlock when a
//! caller holds two keys that share a stripe, e.g. a lowres LOD cascade).

use std::collections::HashSet;
use std::sync::{Condvar, Mutex, PoisonError};

use bm_format::grid::Tile;

use crate::key::{GridKey, ItemKey};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LockKey {
    Grid(GridKey, Tile),
    Item(ItemKey),
}

#[derive(Debug, Default)]
pub struct KeyLocks {
    held: Mutex<HashSet<LockKey>>,
    released: Condvar,
}

/// Held until dropped.
#[must_use = "the lock is released when the guard is dropped"]
#[derive(Debug)]
pub struct KeyLock<'a> {
    locks: &'a KeyLocks,
    key: Option<LockKey>,
}

impl KeyLocks {
    pub fn lock(&self, key: LockKey) -> KeyLock<'_> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        while held.contains(&key) {
            held = self.released.wait(held).unwrap_or_else(PoisonError::into_inner);
        }
        held.insert(key.clone());
        KeyLock { locks: self, key: Some(key) }
    }

    pub fn try_lock(&self, key: LockKey) -> Option<KeyLock<'_>> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.insert(key.clone()).then(|| KeyLock { locks: self, key: Some(key) })
    }
}

impl Drop for KeyLock<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.locks.held.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
            self.locks.released.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn excludes_same_key_only() {
        let locks = KeyLocks::default();
        let a = locks.lock(LockKey::Grid(GridKey::Lowres(1), (0, 0)));
        assert!(locks.try_lock(LockKey::Grid(GridKey::Lowres(1), (0, 0))).is_none());
        let _b = locks.lock(LockKey::Grid(GridKey::Lowres(2), (0, 0)));
        drop(a);
        assert!(locks.try_lock(LockKey::Grid(GridKey::Lowres(1), (0, 0))).is_some());
    }

    #[test]
    fn serialises_writers() {
        let locks = Arc::new(KeyLocks::default());
        let inside = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (locks, inside) = (locks.clone(), inside.clone());
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        let _g = locks.lock(LockKey::Item(ItemKey::Settings));
                        assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
    }
}
