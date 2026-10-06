//! Iteration order of a `java.util.HashMap<String, _>`: BlueMap streams map configs out of one, so ties in a later
//! stable sort keep this order.

use crate::string_hash;

/// Keys in the order `new HashMap<>()` iterates them after inserting `keys` in the given order (duplicates keep
/// their first position). Ignores treeified bins (8+ colliding keys in one bucket).
pub fn hash_map_order<S: AsRef<str>>(keys: &[S]) -> Vec<&str> {
    let mut unique: Vec<&str> = Vec::with_capacity(keys.len());
    for k in keys {
        if !unique.contains(&k.as_ref()) {
            unique.push(k.as_ref());
        }
    }
    let mut capacity = 16usize;
    while unique.len() > capacity / 4 * 3 {
        capacity *= 2;
    }
    let bucket = |k: &str| {
        let h = string_hash(k) as u32;
        (h ^ (h >> 16)) as usize & (capacity - 1)
    };
    // resizes split buckets but keep the relative order inside each, so insertion order breaks bucket ties
    unique.sort_by_key(|k| bucket(k));
    unique
}
