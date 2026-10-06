//! `java.util.Random`: a 48-bit LCG. BlueMap seeds one with 2345 for swamp foliage noise.

const MULT: u64 = 0x5_DEEC_E66D;
const ADD: u64 = 0xB;
const MASK: u64 = (1 << 48) - 1;

#[derive(Clone, Copy, Debug)]
pub struct JavaRandom {
    state: u64,
}

impl JavaRandom {
    /// `new Random(seed)`: scrambles the seed.
    pub fn new(seed: i64) -> Self {
        Self { state: (seed as u64 ^ MULT) & MASK }
    }

    pub fn next(&mut self, bits: u32) -> i32 {
        self.state = self.state.wrapping_mul(MULT).wrapping_add(ADD) & MASK;
        (self.state >> (48 - bits)) as i32
    }

    pub fn next_int(&mut self, bound: i32) -> i32 {
        if (bound as u32).is_power_of_two() {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            // Java's rejection of the biased tail: the sum overflows i32
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as i64) << 32).wrapping_add(self.next(32) as i64)
    }

    pub fn next_double(&mut self) -> f64 {
        (((self.next(26) as i64) << 27) + self.next(27) as i64) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

/// Java `String.hashCode`.
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_openjdk() {
        // new Random(0).nextInt(), new Random(0).nextLong(), new Random(42).nextInt(10), new Random(0).nextDouble()
        assert_eq!(JavaRandom::new(0).next(32), -1155484576);
        assert_eq!(JavaRandom::new(0).next_long(), -4962768465676381896);
        assert_eq!(JavaRandom::new(42).next_int(10), 0);
        assert_eq!(JavaRandom::new(0).next_double(), 0.730967787376657);
        assert_eq!(string_hash("hello"), 99162322);
    }
}
