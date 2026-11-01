//! Throwaway custom codec: colour half and metadata half (light, 16-bit height, alpha) as separate streams with
//! optional per-plane prediction, in one zstd frame.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    /// RGBA scanlines, as an unfiltered PNG holds them
    Raw,
    /// all 8 planes, unpredicted
    Split,
    /// colour as whole RGBA pixels, height predicted
    PixelsHeight,
    /// colour as A, G, R−G, B−G, every plane with its own predictor picked by trial compression
    Predicted,
    /// colour as a frequency-sorted palette and 16-bit indices, height predicted
    Palette,
}

/// Palette RGBA bytes and (low, high) index planes; `None` past 65536 colours.
fn palettize(colour: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut counts = std::collections::HashMap::new();
    for p in colour.chunks_exact(4) {
        *counts.entry(u32::from_be_bytes(p.try_into().unwrap())).or_insert(0u32) += 1;
    }
    if counts.len() > 65536 {
        return None;
    }
    let mut order: Vec<(u32, u32)> = counts.iter().map(|(&c, &n)| (n, c)).collect();
    order.sort_unstable_by(|a, b| b.cmp(a));
    let index: std::collections::HashMap<u32, u16> =
        order.iter().enumerate().map(|(i, &(_, c))| (c, i as u16)).collect();
    let (mut lo, mut hi) = (Vec::with_capacity(colour.len() / 4), Vec::with_capacity(colour.len() / 4));
    for p in colour.chunks_exact(4) {
        let i = index[&u32::from_be_bytes(p.try_into().unwrap())];
        lo.push(i as u8);
        hi.push((i >> 8) as u8);
    }
    Some((order.iter().flat_map(|&(_, c)| c.to_be_bytes()).collect(), lo, hi))
}

const NONE: u8 = 0;
const LEFT: u8 = 1;
const UP: u8 = 2;
const MED: u8 = 3;

fn med(l: i32, u: i32, ul: i32) -> i32 {
    if ul >= l.max(u) {
        l.min(u)
    } else if ul <= l.min(u) {
        l.max(u)
    } else {
        l + u - ul
    }
}

fn predict(kind: u8, s: &[i32], w: usize, i: usize) -> i32 {
    let l = if i % w > 0 { s[i - 1] } else { 0 };
    let u = if i >= w { s[i - w] } else { 0 };
    match kind {
        NONE => 0,
        LEFT => l,
        UP => u,
        _ => med(l, u, if i % w > 0 && i >= w { s[i - w - 1] } else { 0 }),
    }
}

fn filter8(kind: u8, p: &[u8], w: usize) -> Vec<u8> {
    let s: Vec<i32> = p.iter().map(|&v| v as i32).collect();
    (0..p.len()).map(|i| p[i].wrapping_sub(predict(kind, &s, w, i) as u8)).collect()
}

fn unfilter8(kind: u8, r: &[u8], w: usize) -> Vec<u8> {
    let mut s = vec![0i32; r.len()];
    for i in 0..r.len() {
        s[i] = r[i].wrapping_add(predict(kind, &s, w, i) as u8) as i32;
    }
    s.iter().map(|&v| v as u8).collect()
}

fn best_filter(p: &[u8], w: usize, kinds: &[u8]) -> (u8, Vec<u8>) {
    kinds
        .iter()
        .map(|&k| {
            let f = filter8(k, p, w);
            (zstd::bulk::compress(&f, 1).unwrap().len(), k, f)
        })
        .min_by_key(|c| c.0)
        .map(|(_, k, f)| (k, f))
        .unwrap()
}

/// Residuals of the height as signed 16-bit, zigzagged, split into (high, low) planes.
fn filter_height(kind: u8, h: &[i16], w: usize) -> (Vec<u8>, Vec<u8>) {
    let s: Vec<i32> = h.iter().map(|&v| v as i32).collect();
    let (mut hi, mut lo) = (Vec::with_capacity(h.len()), Vec::with_capacity(h.len()));
    for i in 0..h.len() {
        let d = (s[i] - predict(kind, &s, w, i)) as i16;
        let z = ((d as i32) << 1 ^ (d as i32) >> 31) as u16;
        hi.push((z >> 8) as u8);
        lo.push(z as u8);
    }
    (hi, lo)
}

fn unfilter_height(kind: u8, hi: &[u8], lo: &[u8], w: usize) -> Vec<i16> {
    let mut s = vec![0i32; hi.len()];
    for i in 0..hi.len() {
        let z = (hi[i] as u16) << 8 | lo[i] as u16;
        let d = (z >> 1) as i32 ^ -((z & 1) as i32);
        s[i] = ((predict(kind, &s, w, i) + d) as i16) as i32;
    }
    s.iter().map(|&v| v as i16).collect()
}

fn best_height(h: &[i16], w: usize) -> (u8, Vec<u8>, Vec<u8>) {
    [LEFT, UP, MED]
        .iter()
        .map(|&k| {
            let (hi, lo) = filter_height(k, h, w);
            let n = zstd::bulk::compress(&[&hi[..], &lo[..]].concat(), 1).unwrap().len();
            (n, k, hi, lo)
        })
        .min_by_key(|c| c.0)
        .map(|(_, k, hi, lo)| (k, hi, lo))
        .unwrap()
}

/// `rgba`'s bytes at `(band)` of every pixel in `range` of pixels.
fn band(rgba: &[u8], range: std::ops::Range<usize>, b: usize) -> Vec<u8> {
    range.map(|i| rgba[i * 4 + b]).collect()
}

/// Compressed bytes per stream name, summed over tiles: written by `encode` when `BREAKDOWN` is on.
pub static BREAKDOWN: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
pub const STREAMS: [&str; 8] = ["A", "G", "R-G", "B-G", "light", "height hi", "height lo", "meta alpha"];

fn height_plane(rgba: &[u8], n: usize) -> Vec<i16> {
    (n..2 * n).map(|i| i16::from_be_bytes([rgba[i * 4 + 1], rgba[i * 4 + 2]])).collect()
}

pub fn encode(layout: Layout, level: i32, rgba: &[u8], w: usize, h: usize, breakdown: bool) -> Vec<u8> {
    let n = w * h / 2;
    let mut head = vec![0u8; 5];
    head[0..2].copy_from_slice(&(w as u16).to_le_bytes());
    head[2..4].copy_from_slice(&(h as u16).to_le_bytes());
    head[4] = layout as u8;
    let body: Vec<u8> = match layout {
        Layout::Raw => rgba.to_vec(),
        Layout::Split => {
            let mut b = Vec::with_capacity(rgba.len());
            for r in [0..n, n..2 * n] {
                for c in [3, 0, 1, 2] {
                    b.extend(band(rgba, r.clone(), c));
                }
            }
            b
        }
        Layout::PixelsHeight => {
            let mut b = rgba[..n * 4].to_vec();
            let (k, hi, lo) = best_height(&height_plane(rgba, n), w);
            head.push(k);
            b.extend(band(rgba, n..2 * n, 3));
            b.extend(band(rgba, n..2 * n, 0));
            b.extend(hi);
            b.extend(lo);
            b
        }
        Layout::Palette => {
            let (palette, lo, hi) = palettize(&rgba[..n * 4]).expect("over 65536 colours in a tile");
            let (k, h_hi, h_lo) = best_height(&height_plane(rgba, n), w);
            head.push(k);
            head.extend(((palette.len() / 4) as u32).to_le_bytes());
            let mut b = palette;
            b.extend(lo);
            b.extend(hi);
            b.extend(band(rgba, n..2 * n, 3));
            b.extend(band(rgba, n..2 * n, 0));
            b.extend(h_hi);
            b.extend(h_lo);
            b
        }
        Layout::Predicted => {
            let (r, g, bl) = (band(rgba, 0..n, 0), band(rgba, 0..n, 1), band(rgba, 0..n, 2));
            let rg: Vec<u8> = r.iter().zip(&g).map(|(a, b)| a.wrapping_sub(*b)).collect();
            let bg: Vec<u8> = bl.iter().zip(&g).map(|(a, b)| a.wrapping_sub(*b)).collect();
            let all = [NONE, LEFT, UP, MED];
            let mut streams = Vec::new();
            for p in [band(rgba, 0..n, 3), g, rg, bg, band(rgba, n..2 * n, 0)] {
                let (k, f) = best_filter(&p, w, &all);
                head.push(k);
                streams.push(f);
            }
            let (k, hi, lo) = best_height(&height_plane(rgba, n), w);
            head.push(k);
            streams.push(hi);
            streams.push(lo);
            streams.push(band(rgba, n..2 * n, 3));
            if breakdown {
                for (s, total) in streams.iter().zip(&BREAKDOWN) {
                    total.fetch_add(zstd::bulk::compress(s, level).unwrap().len() as u64, Ordering::Relaxed);
                }
            }
            streams.concat()
        }
    };
    head.extend(zstd::bulk::compress(&body, level).unwrap());
    head
}

pub fn decode(blob: &[u8]) -> Vec<u8> {
    let w = u16::from_le_bytes([blob[0], blob[1]]) as usize;
    let h = u16::from_le_bytes([blob[2], blob[3]]) as usize;
    let n = w * h / 2;
    let size = w * h * 4;
    let layout = blob[4];
    let extra = [0, 0, 1, 6, 5][layout as usize];
    // a palette tile's body exceeds the raw size only by its palette
    let body = zstd::bulk::decompress(&blob[5 + extra..], size + 65536 * 4).unwrap();
    let kinds = &blob[5..5 + extra];
    if layout == Layout::Raw as u8 {
        return body;
    }
    let mut out = vec![0u8; size];
    let mut put = |range: std::ops::Range<usize>, b: usize, p: &[u8]| {
        for (i, &v) in range.zip(p) {
            out[i * 4 + b] = v;
        }
    };
    if layout == Layout::Split as u8 {
        for (i, (r, c)) in
            [(0..n, 3), (0..n, 0), (0..n, 1), (0..n, 2), (n..2 * n, 3), (n..2 * n, 0), (n..2 * n, 1), (n..2 * n, 2)]
                .into_iter()
                .enumerate()
        {
            put(r, c, &body[i * n..(i + 1) * n]);
        }
        return out;
    }
    let meta_at = |base: usize, height_kind: u8| {
        let hs = unfilter_height(height_kind, &body[base..base + n], &body[base + n..base + 2 * n], w);
        hs
    };
    if layout == Layout::Palette as u8 {
        let colours = u32::from_le_bytes(kinds[1..5].try_into().unwrap()) as usize * 4;
        let (palette, rest) = body.split_at(colours);
        let hs = unfilter_height(kinds[0], &rest[4 * n..5 * n], &rest[5 * n..6 * n], w);
        for i in 0..n {
            let at = (rest[n + i] as usize) << 8 | rest[i] as usize;
            let c = &palette[at * 4..at * 4 + 4];
            out[i * 4..i * 4 + 4].copy_from_slice(c);
            let [hi, lo] = hs[i].to_be_bytes();
            let m = n + i;
            out[m * 4..m * 4 + 4].copy_from_slice(&[rest[3 * n + i], hi, lo, rest[2 * n + i]]);
        }
        return out;
    }
    if layout == Layout::PixelsHeight as u8 {
        let base = n * 4;
        let hs = meta_at(base + 2 * n, kinds[0]);
        out[..base].copy_from_slice(&body[..base]);
        for i in 0..n {
            let [hi, lo] = hs[i].to_be_bytes();
            let m = n + i;
            out[m * 4..m * 4 + 4].copy_from_slice(&[body[base + n + i], hi, lo, body[base + i]]);
        }
        return out;
    }
    let plane = |i: usize| unfilter8(kinds[i], &body[i * n..(i + 1) * n], w);
    let (a, g, rg, bg, l) = (plane(0), plane(1), plane(2), plane(3), plane(4));
    let hs = meta_at(5 * n, kinds[5]);
    let ma = &body[7 * n..8 * n];
    for i in 0..n {
        out[i * 4..i * 4 + 4].copy_from_slice(&[rg[i].wrapping_add(g[i]), g[i], bg[i].wrapping_add(g[i]), a[i]]);
        let [hi, lo] = hs[i].to_be_bytes();
        let m = n + i;
        out[m * 4..m * 4 + 4].copy_from_slice(&[l[i], hi, lo, ma[i]]);
    }
    out
}
