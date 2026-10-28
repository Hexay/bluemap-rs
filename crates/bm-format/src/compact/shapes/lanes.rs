//! Per-vertex float tests on a quad's 4 block-local values of one axis, 4 lanes at once on x86-64 (SSE2 is
//! baseline there), with scalar twins that define the result.

/// `[a, b, c, d]` as one integer: hashing an integer skips the byte-slice path arrays of integers take.
#[inline]
pub(super) fn pack(v: [u32; 4]) -> u128 {
    v.iter().rev().fold(0, |k, &x| k << 32 | u128::from(x))
}

/// The coarse key of a value: `(l * 4096.0) as i32` per vertex, packed. Any deterministic coarse key works; a value
/// straddling a key boundary only costs a duplicate shape.
#[inline]
pub(super) fn key_scalar(local: &[f32; 4]) -> u128 {
    pack(local.map(|l| (l * 4096.0) as i32 as u32))
}

/// `((l * 16.0) as i32) as f32 == l * 16.0` for every vertex.
#[cfg(any(test, not(target_arch = "x86_64")))]
#[inline]
pub(super) fn on_grid_scalar(local: &[f32; 4]) -> bool {
    local.iter().all(|&l| ((l * 16.0) as i32) as f32 == l * 16.0)
}

/// Whether the decoder's `fl32(shape + B)` (or `fl32(fl32(shape + d) + B)` for a hashed shape) gives `want` at
/// every vertex.
#[cfg(any(test, not(target_arch = "x86_64")))]
#[inline]
pub(super) fn fits_scalar(shape: &[u32; 4], d: Option<f32>, b: f32, want: &[u32; 4]) -> bool {
    (0..4).all(|v| {
        let s = f32::from_bits(shape[v]);
        (d.map_or(s, |d| s + d) + b).to_bits() == want[v]
    })
}

#[cfg(target_arch = "x86_64")]
mod sse2 {
    use std::arch::x86_64::*;

    // SAFETY (every unsafe block here): SSE2 intrinsics without memory access; SSE2 is part of x86-64.

    #[inline]
    pub fn fits(shape: &[u32; 4], d: Option<f32>, b: f32, want: &[u32; 4]) -> bool {
        let lanes = |v: &[u32; 4]| unsafe { _mm_setr_epi32(v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32) };
        let (shape, want) = (lanes(shape), lanes(want));
        unsafe {
            let mut s = _mm_castsi128_ps(shape);
            if let Some(d) = d {
                s = _mm_add_ps(s, _mm_set1_ps(d));
            }
            let sum = _mm_castps_si128(_mm_add_ps(s, _mm_set1_ps(b)));
            _mm_movemask_epi8(_mm_cmpeq_epi32(sum, want)) == 0xffff
        }
    }

    #[inline]
    pub fn key(local: &[f32; 4]) -> u128 {
        let (i, overflow) = unsafe {
            let v = _mm_setr_ps(local[0], local[1], local[2], local[3]);
            let i = _mm_cvttps_epi32(_mm_mul_ps(v, _mm_set1_ps(4096.0)));
            // the truncation yields i32::MIN where the saturating cast differs (NaN, out of range)
            (i, _mm_movemask_epi8(_mm_cmpeq_epi32(i, _mm_set1_epi32(i32::MIN))) != 0)
        };
        if overflow {
            return super::key_scalar(local);
        }
        let (lo, hi) = unsafe { (_mm_cvtsi128_si64(i), _mm_cvtsi128_si64(_mm_unpackhi_epi64(i, i))) };
        u128::from(hi as u64) << 64 | u128::from(lo as u64)
    }

    #[inline]
    pub fn on_grid(local: &[f32; 4]) -> bool {
        unsafe {
            let s = _mm_mul_ps(_mm_setr_ps(local[0], local[1], local[2], local[3]), _mm_set1_ps(16.0));
            let back = _mm_cvtepi32_ps(_mm_cvttps_epi32(s));
            // the saturating cast maps +2^31 onto itself too; truncation turns it into -2^31
            let abs = _mm_andnot_ps(_mm_set1_ps(-0.0), s);
            let fits = _mm_or_ps(_mm_cmpeq_ps(back, s), _mm_cmpeq_ps(abs, _mm_set1_ps(2_147_483_648.0)));
            _mm_movemask_ps(fits) == 0xf
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub(super) use sse2::{fits, key, on_grid};

#[cfg(not(target_arch = "x86_64"))]
pub(super) use {fits_scalar as fits, key_scalar as key, on_grid_scalar as on_grid};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_match_scalar() {
        let specials = [0.0, -0.0, 1.0, 0.5, 1.0 / 16.0, 1.0 / 4096.0, 0.3, -0.75, 1e-40, f32::NAN, f32::INFINITY];
        let big = [134_217_728.0, -134_217_728.0, 134_217_744.0, 524_288.0, 524_288.1, -524_288.0, 3e9, -3e9];
        let values: Vec<f32> = specials.iter().chain(&big).flat_map(|&v| [v, -v]).collect();
        for (i, &a) in values.iter().enumerate() {
            for &b in &values[i..] {
                for v in [[a, b, b, b], [b, a, a, a], [a, a, a, a], [0.0, 1.0, a, b]] {
                    assert_eq!(key(&v), key_scalar(&v), "key {v:?}");
                    assert_eq!(on_grid(&v), on_grid_scalar(&v), "on_grid {v:?}");
                    let shape = v.map(f32::to_bits);
                    for (d, b) in [(None, 0.0), (None, 1000.0), (Some(0.3), -7.0), (Some(-0.0), 0.0)] {
                        let want = (d.map_or(v, |d| v.map(|s| s + d))).map(|s| (s + b).to_bits());
                        let off = [want[0] ^ 1, want[1], want[2], want[3]];
                        for w in [want, off] {
                            assert_eq!(fits(&shape, d, b, &w), fits_scalar(&shape, d, b, &w), "fits {v:?}");
                        }
                    }
                }
            }
        }
    }
}
