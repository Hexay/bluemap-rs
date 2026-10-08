//! Fixed-point quad geometry: f32 → i16 on a 2^-g grid, parallelogram-predicted, with exact f32-bit escapes.

use super::CompactError;
use super::bytes::{Reader, put_planes, stream};
use super::view::QV;

/// Grid exponents a tile may use; the one with the fewest escapes wins (ties: the coarser).
const GRIDS: [u8; 3] = [4, 5, 8];

fn grid_value(q: i16, g: u8) -> f32 {
    // the reciprocal of a power of two is exact, so this equals the division
    f32::from(q) * (1.0 / (1u32 << g) as f32)
}

/// Nearest grid value (saturated to i16) and whether it reproduces `bits` exactly (-0.0 and NaN never do).
fn quantize(bits: u32, g: u8) -> (i16, bool) {
    // scaling by 2^g is exact in f32, so a value on the grid (the common case) is an integer in range here
    let x = f32::from_bits(bits) * (1u32 << g) as f32;
    if (-32768.0..=32767.0).contains(&x) && f32::from(x as i16) == x && bits != (-0.0f32).to_bits() {
        debug_assert_eq!((x as i16, true), quantize_rounded(bits, g));
        return (x as i16, true);
    }
    quantize_rounded(bits, g)
}

fn quantize_rounded(bits: u32, g: u8) -> (i16, bool) {
    let v = f32::from_bits(bits);
    let scaled = f64::from(v) * f64::from(1u32 << g);
    let q = if v.is_finite() { round_half_away(scaled.clamp(-32768.0, 32767.0)) } else { 0 };
    debug_assert!(!v.is_finite() || q == scaled.round().clamp(-32768.0, 32767.0) as i16);
    (q, grid_value(q, g).to_bits() == bits)
}

/// `f64::round` for values in i16 range, without the libm call baseline x86-64 makes for it.
fn round_half_away(x: f64) -> i16 {
    let t = x as i32;
    let frac = x - f64::from(t);
    (t + i32::from(frac >= 0.5) - i32::from(frac <= -0.5)) as i16
}

/// Calls `f(quad, vertex, component, bits)` for the stored values of a PRBM f32 attribute with `K` components, in
/// `[quad][4 vertices QV][K]` order.
#[allow(clippy::chunks_exact_to_as_chunks)] // `as_chunks::<{ 24 * K }>` needs generic_const_exprs
fn for_each_stored<const K: usize>(attr: &[u8], mut f: impl FnMut(usize, usize, usize, u32)) {
    for (q, quad) in attr.chunks_exact(24 * K).enumerate() {
        for (vert, prbm) in QV.into_iter().enumerate() {
            let values = quad[4 * K * prbm..4 * K * (prbm + 1)].as_chunks::<4>().0;
            values.iter().enumerate().for_each(|(c, &b)| f(q, vert, c, u32::from_le_bytes(b)));
        }
    }
}

pub(super) fn pick_grid<const K: usize>(attr: &[u8]) -> u8 {
    let mut escapes = [0usize; GRIDS.len()];
    for_each_stored::<K>(attr, |_, _, _, bits| {
        for (n, g) in escapes.iter_mut().zip(GRIDS) {
            *n += usize::from(!quantize(bits, g).1);
        }
    });
    GRIDS.into_iter().zip(escapes).min_by_key(|&(_, n)| n).map_or(GRIDS[0], |(g, _)| g)
}

/// Writes the residual stream and the escape stream for the `K`-component f32 attribute `attr` of a quad-shaped PRBM.
pub(super) fn encode<const K: usize>(body: &mut Vec<u8>, attr: &[u8], g: u8) {
    let mut escapes = Vec::new();
    stream(body, |out| {
        let mut prev = [0i16; 3];
        let mut quad = [[0i16; 3]; 4];
        for_each_stored::<K>(attr, |q, vert, c, bits| {
            let (v, exact) = quantize(bits, g);
            if !exact {
                escapes.push((((4 * q + vert) * K + c) as u32, bits.wrapping_sub(grid_value(v, g).to_bits())));
            }
            quad[vert][c] = v;
            if vert == 3 && c == K - 1 {
                put_residuals(out, &quad, &mut prev, K);
            }
        });
    });
    stream(body, |out| {
        out.extend((escapes.len() as u32).to_le_bytes());
        let mut last = None;
        let gaps = escapes.iter().map(move |&(i, _)| {
            let gap = last.map_or(i, |l: u32| i - l);
            last = Some(i);
            gap
        });
        put_planes(out, gaps);
        put_planes(out, escapes.iter().map(|&(_, r)| r));
    });
}

/// One quad's 24-byte (k = 3) or 16-byte (k = 2) residual record.
fn put_residuals(out: &mut Vec<u8>, v: &[[i16; 3]; 4], prev: &mut [i16; 3], k: usize) {
    let parallelogram: [i16; 3] = std::array::from_fn(|c| v[0][c].wrapping_add(v[2][c]).wrapping_sub(v[1][c]));
    for (value, base) in [(&v[0], &*prev), (&v[1], &v[0]), (&v[2], &v[1]), (&v[3], &parallelogram)] {
        value[..k].iter().zip(base).for_each(|(x, y)| out.extend(x.wrapping_sub(*y).to_le_bytes()));
    }
    *prev = v[0];
}

/// Inverse of [`encode`]: `quads × 4 × k` f32 bit patterns into `values`.
pub(super) fn decode(r: &mut Reader, quads: usize, k: usize, g: u8, values: &mut Vec<u32>) -> Result<(), CompactError> {
    if !GRIDS.contains(&g) {
        return Err(CompactError::Corrupt("grid"));
    }
    let n = quads * 4 * k;
    let mut res = r.stream(Some(n * 2))?;
    values.clear();
    values.reserve(n);
    let mut prev = [0i16; 3];
    let mut v = [[0i16; 3]; 4];
    for _ in 0..quads {
        for vert in 0..4 {
            for c in 0..k {
                let d = i16::from_le_bytes(res.take(2)?.try_into().unwrap());
                v[vert][c] = match vert {
                    0 => prev[c].wrapping_add(d),
                    3 => v[0][c].wrapping_add(v[2][c]).wrapping_sub(v[1][c]).wrapping_add(d),
                    _ => v[vert - 1][c].wrapping_add(d),
                };
            }
        }
        prev = v[0];
        for vert in v {
            values.extend(vert[..k].iter().map(|&q| grid_value(q, g).to_bits()));
        }
    }
    let mut esc = r.stream(None)?;
    let count = esc.u32()? as usize;
    let gaps = esc.planes(count)?;
    let residuals = esc.planes(count)?;
    let mut index: Option<usize> = None;
    for (gap, residual) in gaps.zip(residuals) {
        let i =
            index.map_or(Some(gap as usize), |i| i.checked_add(gap as usize)).ok_or(CompactError::Corrupt("escape"))?;
        let slot = values.get_mut(i).ok_or(CompactError::Corrupt("escape index"))?;
        *slot = slot.wrapping_add(residual);
        index = Some(i);
    }
    if esc.is_empty() { Ok(()) } else { Err(CompactError::Corrupt("escape stream")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_f64_round() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut samples = vec![0.5, -0.5, 1.5, -1.5, 2.5, 0.49999999999999994, -0.49999999999999994, 32767.4, -32767.6];
        samples.extend((0..100_000).map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64 * 70_000.0 - 35_000.0
        }));
        for v in samples {
            let v = v.clamp(-32768.0, 32767.0);
            assert_eq!(round_half_away(v), v.round() as i16, "{v}");
        }
        for q in [i16::MIN, -255, -1, 0, 1, 77, i16::MAX] {
            for g in GRIDS {
                assert_eq!(grid_value(q, g), f32::from(q) / (1u32 << g) as f32);
            }
        }
    }

    #[test]
    fn grid_and_escapes_are_exact() {
        assert_eq!(quantize(1.5f32.to_bits(), 4), (24, true));
        assert_eq!(quantize((-0.0f32).to_bits(), 4), (0, false));
        assert!(!quantize(f32::NAN.to_bits(), 4).1);
        assert_eq!(quantize(1e9f32.to_bits(), 4), (32767, false));
        let floats = [0.0, 1.0, 1.0, 0.05, 2.0, -0.0, 3.0, f32::INFINITY, 0.5, 1e9, -7.25, 0.1];
        let bits: Vec<u32> = floats.iter().map(|f: &f32| f.to_bits()).collect();
        // PRBM vertices 0,1,2,5 hold the quad; 3 and 4 are never read
        let mut attr = vec![0xAB; 6 * 12];
        for (i, b) in bits.iter().enumerate() {
            let at = 4 * (3 * QV[i / 3] + i % 3);
            attr[at..at + 4].copy_from_slice(&b.to_le_bytes());
        }
        let mut body = Vec::new();
        encode::<3>(&mut body, &attr, 4);
        let mut out = Vec::new();
        decode(&mut Reader::new(&body), 1, 3, 4, &mut out).unwrap();
        assert_eq!(out, bits);
    }
}
