//! Paletted-container index arrays. 1.16+ pads each long (no value spans two longs); 1.13–1.15 packs values
//! back to back across long boundaries ("spanning").

/// The layout of a padded array; the longs live wherever the caller keeps them (a chunk's flat word array, or the
/// NBT bytes read in place).
#[derive(Clone, Copy, Debug)]
pub struct Padded {
    bits: u8,
    per_long: u8,
    mask: u64,
}

impl Padded {
    pub fn new(bits: u8) -> Self {
        let bits = bits.clamp(1, 32);
        Self { bits, per_long: 64 / bits, mask: (1 << bits) - 1 }
    }

    /// Value `i` of `data`; 0 past the end of the data, as BlueMap.
    pub fn get(self, data: &[u64], i: usize) -> u32 {
        self.get_with(|l| data.get(l).copied(), i)
    }

    /// [`Padded::get`] over longs `long(index)` returns.
    pub fn get_with(self, long: impl FnOnce(usize) -> Option<u64>, i: usize) -> u32 {
        let (l, slot) = (i / self.per_long as usize, i % self.per_long as usize);
        long(l).map_or(0, |l| ((l >> (slot * self.bits as usize)) & self.mask) as u32)
    }

    /// Values `start`, `start + stride`, … (`count` of them), [`Padded::get`] each, with no division per value.
    pub fn strided(self, data: &[u64], start: usize, stride: usize, count: usize) -> impl Iterator<Item = u32> + '_ {
        let per = self.per_long as usize;
        let (step_long, step_slot) = (stride / per, stride % per);
        let (mut long, mut slot) = (start / per, start % per);
        (0..count).map(move |_| {
            let v = data.get(long).map_or(0, |&l| ((l >> (slot * self.bits as usize)) & self.mask) as u32);
            (long, slot) = (long + step_long, slot + step_slot);
            if slot >= per {
                (long, slot) = (long + 1, slot - per);
            }
            v
        })
    }

    /// Whether `longs` longs hold `count` values, i.e. the array isn't truncated.
    pub fn holds(self, longs: usize, count: usize) -> bool {
        longs * self.per_long as usize >= count
    }
}

/// Value `i` of a spanning array of `bits`-wide values (longs from `long(index)`); 0 past the end.
pub fn spanning_get(long: impl Fn(usize) -> Option<u64>, bits: u32, i: usize) -> u32 {
    let start = i * bits as usize;
    let (at, offset) = (start / 64, (start % 64) as u32);
    let Some(lo) = long(at) else { return 0 };
    let mut v = lo >> offset;
    if offset + bits > 64 {
        v |= long(at + 1).map_or(0, |hi| hi << (64 - offset));
    }
    (v & ((1u64 << bits) - 1)) as u32
}

pub fn ceil_log2(n: usize) -> u32 {
    if n <= 1 { 0 } else { usize::BITS - (n - 1).leading_zeros() }
}

/// Bits per block of a padded block-state array: what Minecraft wrote for this palette size, unless the data length
/// disagrees, then BlueMap's inference from the length. (BlueMap always infers, which misreads 11-bit palettes.)
pub fn padded_block_bits(palette_len: usize, longs: usize) -> u8 {
    let bits = ceil_log2(palette_len).max(4);
    let per_long = 64 / bits as usize;
    if longs == 4096usize.div_ceil(per_long) { bits as u8 } else { (longs * 64 / 4096).clamp(1, 32) as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_values_do_not_span_longs() {
        // 5 bits: 12 values per long, top 4 bits unused
        let data = [(0..12).fold(0, |l, i| l | (i as u64) << (i * 5)), 31];
        let p = Padded::new(5);
        assert_eq!((0..13).map(|i| p.get(&data, i)).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 31]);
        assert_eq!(p.get(&data, 10_000), 0);
        assert!(p.holds(data.len(), 24) && !p.holds(data.len(), 25));
    }

    #[test]
    fn strided_reads_match_get() {
        let data: Vec<u64> = (0..400u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15)).collect();
        for bits in [1, 4, 5, 7, 11, 13, 32] {
            let p = Padded::new(bits);
            for (start, stride) in [(0, 256), (37, 256), (255, 256), (3, 1), (100, 61)] {
                let want: Vec<u32> = (0..16).map(|k| p.get(&data, start + k * stride)).collect();
                assert_eq!(p.strided(&data, start, stride, 16).collect::<Vec<_>>(), want, "bits {bits} start {start}");
            }
        }
    }

    #[test]
    fn spanning_values_cross_longs() {
        // 5-bit value 0b10110 at index 12: bits 60..65, split 4 + 1
        let data = [0b0110 << 60, 0b1];
        let long = |l: usize| data.get(l).copied();
        assert_eq!(spanning_get(long, 5, 12), 0b10110);
        assert_eq!(spanning_get(long, 5, 1000), 0);
    }

    #[test]
    fn block_bits_follow_palette_size_when_the_length_agrees() {
        assert_eq!(padded_block_bits(2, 256), 4);
        assert_eq!(padded_block_bits(20, 342), 5);
        assert_eq!(padded_block_bits(1500, 820), 11);
        assert_eq!(padded_block_bits(3000, 820), 12);
        // length disagrees with the palette: BlueMap's inference
        assert_eq!(padded_block_bits(2, 342), 5);
    }
}
