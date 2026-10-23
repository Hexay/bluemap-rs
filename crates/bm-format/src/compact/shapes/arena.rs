//! Candidate shapes of one axis in one form, grouped by a coarse key of their block-local values.

use rustc_hash::FxHashMap;

use super::lanes::key;

pub(super) const NONE: u32 = u32::MAX;

/// Candidate `k < finest.len()` is the finest value of key id `k` and the head of that key's candidates.
#[derive(Default)]
pub(super) struct Arena {
    pub values: Vec<[u32; 4]>,
    /// Next candidate with the same key.
    next: Vec<u32>,
    /// Key → key id, in order of first sight.
    ids: FxHashMap<u128, u32>,
    /// The last key offered and its id: neighbouring quads often share a key.
    last: (u128, u32),
    /// Per key id, the local value seen at the smallest |B| (most fraction bits) and that |B|.
    finest: Vec<(f32, [u32; 4])>,
    /// Blocks that used the shape, and the last of them.
    pub blocks: Vec<u32>,
    last_block: Vec<u32>,
    /// Id in the final table, once a quad took the shape.
    pub table_id: Vec<u32>,
}

impl Arena {
    pub fn clear(&mut self) {
        self.values.clear();
        self.next.clear();
        self.ids.clear();
        self.last = (0, NONE);
        self.finest.clear();
        self.blocks.clear();
        self.last_block.clear();
        self.table_id.clear();
    }

    /// Records `local`, derived at block coordinate `b`, as a candidate head for its key; returns the key id.
    #[inline]
    pub fn offer(&mut self, local: &[f32; 4], b: f32) -> u32 {
        let k = key(local);
        let id = if self.last.1 != NONE && self.last.0 == k {
            self.last.1
        } else {
            let next = self.finest.len() as u32;
            let id = *self.ids.entry(k).or_insert(next);
            if id == next {
                self.finest.push((f32::INFINITY, [0; 4]));
            }
            self.last = (k, id);
            id
        };
        let entry = &mut self.finest[id as usize];
        if b.abs() < entry.0 {
            *entry = (b.abs(), local.map(f32::to_bits));
        }
        id
    }

    /// Makes every key's finest value the first candidate tried, so one shape serves the key wherever it can.
    pub fn plant(&mut self) {
        for i in 0..self.finest.len() {
            self.push(self.finest[i].1, NONE);
        }
    }

    fn push(&mut self, bits: [u32; 4], next: u32) {
        self.values.push(bits);
        self.next.push(next);
        self.blocks.push(0);
        self.last_block.push(NONE);
        self.table_id.push(NONE);
    }

    /// The candidate of key id `head` that `fits`, adding `local` itself (behind the head) if none does and it
    /// fits.
    #[inline]
    pub fn find(&mut self, head: u32, local: [f32; 4], block: u32, fits: impl Fn(&[u32; 4]) -> bool) -> u32 {
        let mut id = head;
        while id != NONE && !fits(&self.values[id as usize]) {
            id = self.next[id as usize];
        }
        if id == NONE {
            let bits = local.map(f32::to_bits);
            if !fits(&bits) {
                return NONE;
            }
            id = self.values.len() as u32;
            self.push(bits, self.next[head as usize]);
            self.next[head as usize] = id;
        }
        if self.last_block[id as usize] != block {
            self.last_block[id as usize] = block;
            self.blocks[id as usize] += 1;
        }
        id
    }
}
