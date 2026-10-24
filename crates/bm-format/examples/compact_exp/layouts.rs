//! Alternative stream layouts of a model body, built from the shipped body (size-only: no decoder reads them).

use std::collections::HashMap;

use bm_format::compact::{BODY_HEADER, STREAMS};

pub struct Body {
    pub quads: usize,
    pub bytes: Vec<u8>,
}

/// `(start, len)` of every stream's payload.
fn streams(body: &[u8]) -> Vec<(usize, usize)> {
    let mut at = BODY_HEADER;
    (0..STREAMS.len())
        .map(|_| {
            let len = u32::from_le_bytes(body[at..at + 4].try_into().unwrap()) as usize;
            at += 4 + len;
            (at - len, len)
        })
        .collect()
}

/// The body cut at every stream boundary (for block flushes).
pub fn cut(b: &Body) -> Vec<Vec<u8>> {
    let s = streams(&b.bytes);
    let mut parts = vec![b.bytes[..BODY_HEADER].to_vec()];
    parts.extend(s.iter().map(|&(at, len)| b.bytes[at - 4..at + len].to_vec()));
    parts
}

/// Streams 6 (ids) and 7 (cells) replaced by one record stream `id, step, dy` per quad.
pub fn interleaved(b: &Body) -> Vec<Vec<u8>> {
    let s = streams(&b.bytes);
    let (ids, cells) = (&b.bytes[s[6].0..][..s[6].1], &b.bytes[s[7].0..][..s[7].1]);
    let width = ids.len() / b.quads.max(1);
    let mut rec = Vec::with_capacity(ids.len() + cells.len());
    for q in 0..b.quads {
        rec.extend(&ids[q * width..][..width]);
        rec.extend(&cells[2 * q..][..2]);
    }
    vec![b.bytes[..s[6].0 - 4].to_vec(), rec, b.bytes[s[8].0 - 4..].to_vec()]
}

/// The cell stream split into a step plane and a dy plane.
pub fn cell_planes(b: &Body) -> Vec<Vec<u8>> {
    let s = streams(&b.bytes);
    let cells = &b.bytes[s[7].0..][..s[7].1];
    let planes: Vec<u8> = cells.iter().step_by(2).chain(cells.iter().skip(1).step_by(2)).copied().collect();
    vec![b.bytes[..s[7].0].to_vec(), planes, b.bytes[s[7].0 + s[7].1..].to_vec()]
}

/// `(column, y)` of every quad and which quads start a group, from the shipped cell streams.
fn walk(b: &Body, s: &[(usize, usize)]) -> (Vec<(i64, i64)>, Vec<bool>) {
    let mut starts = vec![false; b.quads + 1];
    let mut at = 0;
    for g in b.bytes[s[0].0..][..s[0].1].as_chunks::<8>().0 {
        starts[at] = true;
        at += u32::from_le_bytes(g[4..].try_into().unwrap()) as usize;
    }
    let (cells, mut wide) = (&b.bytes[s[7].0..][..s[7].1], b.bytes[s[8].0..][..s[8].1].as_chunks::<4>().0.iter());
    let mut next = || i64::from(i32::from_le_bytes(*wide.next().unwrap()));
    let (mut column, mut y, mut run_top) = (0i64, 0i64, 0i64);
    let mut out = Vec::with_capacity(b.quads);
    for q in 0..b.quads {
        let (st, dy) = (cells[2 * q], cells[2 * q + 1] as i8);
        let step = if st == 255 { next() } else { i64::from(st) };
        let dy = if dy == 127 { next() } else { i64::from(dy) };
        let c = if starts[q] { 0 } else { column } + step;
        let run = starts[q] || c != column;
        y = if run { run_top } else { y } + dy;
        if run {
            run_top = y;
        }
        column = c;
        out.push((c, y));
    }
    (out, starts)
}

/// Cells re-coded: a column run's first dy is relative to a 2D prediction from the same group's run tops at
/// (x, z-1), (x-1, z), (x-1, z-1) (MED when all three exist) instead of the previous run's top.
pub fn cells_2d(b: &Body) -> Vec<Vec<u8>> {
    let s = streams(&b.bytes);
    let depth = i64::from(i32::from_le_bytes(b.bytes[21..25].try_into().unwrap()));
    let (decoded, starts) = walk(b, &s);
    let (mut out, mut esc) = (Vec::with_capacity(2 * b.quads), Vec::new());
    let mut tops: HashMap<i64, i64> = HashMap::new();
    let (mut prev_c, mut prev_y, mut prev_top) = (0i64, 0i64, 0i64);
    for (q, &(c, y)) in decoded.iter().enumerate() {
        if starts[q] {
            tops.clear();
        }
        let step = c - if starts[q] { 0 } else { prev_c };
        let run = starts[q] || c != prev_c;
        let base = if run {
            let up = tops.get(&(c - 1)).filter(|_| c % depth != 0);
            match (up, tops.get(&(c - depth)), tops.get(&(c - depth - 1))) {
                (Some(&a), Some(&l), Some(&d)) if d >= a.max(l) => a.min(l),
                (Some(&a), Some(&l), Some(&d)) if d <= a.min(l) => a.max(l),
                (Some(&a), Some(&l), Some(&d)) => a + l - d,
                (Some(&a), _, _) => a,
                (None, Some(&l), _) => l,
                _ => prev_top,
            }
        } else {
            prev_y
        };
        if run {
            tops.insert(c, y);
            prev_top = y;
        }
        let dy = y - base;
        match u8::try_from(step).ok().filter(|&v| v != 255) {
            Some(v) => out.push(v),
            None => {
                out.push(255);
                esc.extend((step as i32).to_le_bytes());
            }
        }
        match i8::try_from(dy).ok().filter(|&v| v != 127) {
            Some(v) => out.push(v as u8),
            None => {
                out.push(127);
                esc.extend((dy as i32).to_le_bytes());
            }
        }
        (prev_c, prev_y) = (c, y);
    }
    let tail = b.bytes[s[8].0 + s[8].1..].to_vec();
    vec![b.bytes[..s[7].0].to_vec(), out, (esc.len() as u32).to_le_bytes().to_vec(), esc, tail]
}
