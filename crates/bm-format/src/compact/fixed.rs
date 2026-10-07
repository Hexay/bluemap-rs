//! Fixed-point quad geometry: f32 → i16 on a 2^-g grid, parallelogram-predicted, with exact f32-bit escapes.

use super::CompactError;
use super::bytes::{Reader, put_planes, stream};

/// Grid exponents a tile may use; the one with the fewest escapes wins (ties: the coarser).
const GRIDS: [u8; 3] = [4, 5, 8];

fn grid_value(q: i16, g: u8) -> f32 {
    f32::from(q) / (1u32 << g) as f32
}

/// Nearest grid value (saturated to i16) and whether it reproduces `bits` exactly (-0.0 and NaN never do).
fn quantize(bits: u32, g: u8) -> (i16, bool) {
    let v = f32::from_bits(bits);
    let q = if v.is_finite() { (f64::from(v) * f64::from(1u32 << g)).round().clamp(-32768.0, 32767.0) as i16 } else { 0 };
    (q, grid_value(q, g).to_bits() == bits)
}

pub(super) fn pick_grid(values: &[u32]) -> u8 {
    let escapes = |g: u8| values.iter().filter(|&&b| !quantize(b, g).1).count();
    GRIDS.into_iter().min_by_key(|&g| escapes(g)).unwrap_or(GRIDS[0])
}

/// Writes the residual stream and the escape stream for `values` (f32 bits, `[quad][4 vertices][k]`).
pub(super) fn encode(body: &mut Vec<u8>, values: &[u32], k: usize, g: u8, q: &mut Vec<i16>) {
    q.clear();
    let mut escapes = Vec::new();
    for (i, &bits) in values.iter().enumerate() {
        let (v, exact) = quantize(bits, g);
        q.push(v);
        if !exact {
            escapes.push((i as u32, bits.wrapping_sub(grid_value(v, g).to_bits())));
        }
    }
    stream(body, |out| {
        let mut prev = [0i16; 3];
        for quad in q.chunks_exact(4 * k) {
            let v = |vert: usize, c: usize| quad[vert * k + c];
            for (c, p) in prev.iter_mut().enumerate().take(k) {
                out.extend(v(0, c).wrapping_sub(*p).to_le_bytes());
                *p = v(0, c);
            }
            for c in 0..k {
                out.extend(v(1, c).wrapping_sub(v(0, c)).to_le_bytes());
            }
            for c in 0..k {
                out.extend(v(2, c).wrapping_sub(v(1, c)).to_le_bytes());
            }
            for c in 0..k {
                let predicted = v(0, c).wrapping_add(v(2, c)).wrapping_sub(v(1, c));
                out.extend(v(3, c).wrapping_sub(predicted).to_le_bytes());
            }
        }
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
        let i = index.map_or(Some(gap as usize), |i| i.checked_add(gap as usize)).ok_or(CompactError::Corrupt("escape"))?;
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
    fn grid_and_escapes_are_exact() {
        assert_eq!(quantize(1.5f32.to_bits(), 4), (24, true));
        assert_eq!(quantize((-0.0f32).to_bits(), 4), (0, false));
        assert_eq!(quantize(f32::NAN.to_bits(), 4).1, false);
        assert_eq!(quantize(1e9f32.to_bits(), 4), (32767, false));
        let floats = [0.0, 1.0, 1.0, 0.05, 2.0, -0.0, 3.0, f32::INFINITY, 0.5, 1e9, -7.25, 0.1];
        let bits: Vec<u32> = floats.iter().map(|f: &f32| f.to_bits()).collect();
        let mut body = Vec::new();
        encode(&mut body, &bits, 3, 4, &mut Vec::new());
        let mut out = Vec::new();
        decode(&mut Reader::new(&body), 1, 3, 4, &mut out).unwrap();
        assert_eq!(out, bits);
    }
}
