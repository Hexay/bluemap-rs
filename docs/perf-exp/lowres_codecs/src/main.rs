//! Lowres tile codec comparison; results in docs/results/lowres-codecs-2026-11-01.md.
//! Usage: lowres_codecs <tiles dir> <out.tsv> [--n a,b,c] [--slow-n N] [--only substr[,substr]] [--emit dir]
//! `--n`: tiles per LOD, evenly spaced by PNG size (default all). `--slow-n`: the same for slow candidates.
//! Every candidate's decoder output is compared with the source RGBA on every tile it encodes; a mismatch panics.

mod planes;
mod webp;

use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bm_format::lowres::LowresTile;
use planes::Layout;

struct Tile {
    lod: u32,
    rel: String,
    png: Vec<u8>,
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    tile: LowresTile,
}

type Enc = Box<dyn Fn(&Tile) -> Vec<u8>>;
type Dec = Box<dyn Fn(&[u8]) -> Vec<u8>>;

struct Candidate {
    name: String,
    slow: bool,
    enc: Enc,
    dec: Dec,
}

/// Any PNG colour type to RGBA.
fn decode_png(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let c = info.color_type.samples();
    if c == 4 {
        buf.truncate(w * h * 4);
        return (w, h, buf);
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for p in buf[..w * h * c].chunks_exact(c) {
        out.extend_from_slice(&match *p {
            [v] => [v, v, v, 255],
            [v, a] => [v, v, v, a],
            [r, g, b] => [r, g, b, 255],
            _ => unreachable!(),
        });
    }
    (w, h, out)
}

fn oxi(preset: u8, zopfli: bool) -> Enc {
    let mut o = oxipng::Options::from_preset(preset);
    if zopfli {
        o.deflater = oxipng::Deflater::Zopfli(oxipng::ZopfliOptions {
            iteration_count: NonZeroU64::new(5).unwrap(),
            ..Default::default()
        });
    }
    Box::new(move |t| oxipng::optimize_from_memory(&t.png, &o).unwrap())
}

fn png_dec() -> Dec {
    Box::new(|b| decode_png(b).2)
}

fn candidates() -> Vec<Candidate> {
    let mut v = vec![Candidate {
        name: "product encode_png".into(),
        slow: false,
        enc: Box::new(|t| {
            let mut out = Vec::new();
            t.tile.encode_png(&mut out).unwrap();
            out
        }),
        dec: png_dec(),
    }];
    for p in [1u8, 2, 4, 6] {
        v.push(Candidate { name: format!("oxipng o{p}"), slow: p >= 4, enc: oxi(p, false), dec: png_dec() });
    }
    v.push(Candidate { name: "oxipng o4 zopfli i5".into(), slow: true, enc: oxi(4, true), dec: png_dec() });
    for z in [0, 1, 3, 6, 7, 8, 9] {
        v.push(Candidate {
            name: format!("webp z{z} exact"),
            slow: z >= 9,
            enc: Box::new(move |t| webp::encode(&t.rgba, t.w, t.h, z, true)),
            dec: Box::new(webp::decode),
        });
    }
    let layouts = [
        ("raw", Layout::Raw),
        ("split", Layout::Split),
        ("pixels+height", Layout::PixelsHeight),
        ("predicted", Layout::Predicted),
        ("palette", Layout::Palette),
    ];
    for (name, layout) in layouts {
        for level in [3, 9, 19] {
            v.push(Candidate {
                name: format!("zstd{level} {name}"),
                slow: level == 19,
                enc: Box::new(move |t| planes::encode(layout, level, &t.rgba, t.w, t.h, false)),
                dec: Box::new(planes::decode),
            });
        }
    }
    v
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "png") {
            out.push(p);
        }
    }
}

/// `n` tiles evenly spaced in size order (all if `n` covers them).
fn pick<T>(mut by_size: Vec<T>, n: usize) -> Vec<T> {
    if n >= by_size.len() {
        return by_size;
    }
    let len = by_size.len();
    let keep: Vec<usize> = (0..n).map(|i| i * len / n + len / (2 * n)).collect();
    let mut i = 0;
    by_size.retain(|_| {
        i += 1;
        keep.contains(&(i - 1))
    });
    by_size
}

fn load(root: &Path, lod: u32, n: usize, totals: &mut impl Write) -> Vec<Tile> {
    let mut files = Vec::new();
    walk(&root.join(lod.to_string()), &mut files);
    let mut sized: Vec<(u64, PathBuf)> = files.into_iter().map(|p| (std::fs::metadata(&p).unwrap().len(), p)).collect();
    sized.sort();
    let total: u64 = sized.iter().map(|s| s.0).sum();
    writeln!(totals, "#total\t{lod}\t{}\t{total}", sized.len()).unwrap();
    pick(sized, n)
        .into_iter()
        .map(|(_, p)| {
            let png = std::fs::read(&p).unwrap();
            let (w, h, rgba) = decode_png(&png);
            let tile = LowresTile::decode_png(&png, [w - 1, h / 2 - 1]).unwrap();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            Tile { lod, rel, png, w, h, rgba, tile }
        })
        .collect()
}

fn stats(tiles: &[Tile]) {
    let mut c = [0u64; 8];
    let mut distinct = 0u64;
    for t in tiles {
        let half = t.rgba.len() / 2;
        let mut colours = std::collections::HashSet::new();
        for p in t.rgba[..half].chunks_exact(4) {
            colours.insert([p[0], p[1], p[2], p[3]]);
            match p[3] {
                0 => {
                    c[0] += 1;
                    c[1] += (p[..3] != [0, 0, 0]) as u64;
                }
                255 => c[3] += 1,
                _ => c[2] += 1,
            }
        }
        distinct += colours.len() as u64;
        for p in t.rgba[half..].chunks_exact(4) {
            match p[3] {
                0 => {
                    c[4] += 1;
                    c[5] += (p[..3] != [0, 0, 0]) as u64;
                }
                255 => c[6] += 1,
                _ => c[7] += 1,
            }
        }
    }
    let lod = tiles.first().map_or(0, |t| t.lod);
    println!(
        "lod {lod} {} tiles: colour a=0 {} (rgb!=0 {}), 0<a<255 {}, a=255 {}; meta a=0 {} (rgb!=0 {}), a=255 {}, other {}; \
         avg distinct colours/tile {}",
        tiles.len(), c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], distinct / tiles.len().max(1) as u64
    );
}

fn stored_png(t: &Tile) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, t.w as u32, t.h as u32);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_deflate_compression(png::DeflateCompression::Level(0));
    e.set_filter(png::Filter::NoFilter);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(&t.rgba).unwrap();
    wr.finish().unwrap();
    out
}

/// What real tiles lack: RGB under alpha 0 (top half) and every partial alpha (bottom half).
fn synthetic() -> Tile {
    let (w, h) = (256, 64);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let a = if y < h / 2 { 0 } else { x as u8 };
            rgba.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, (x + y) as u8, a]);
        }
    }
    let mut t = Tile { lod: 0, rel: "syn".into(), png: Vec::new(), w, h, rgba, tile: LowresTile::new([w - 1, h / 2 - 1]) };
    t.png = stored_png(&t);
    t
}

/// Files for the browser test: source PNG, raw RGBA, and each deliverable re-encoding of it.
fn emit(dir: &Path, t: &Tile, id: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let w = |ext: &str, b: &[u8]| std::fs::write(dir.join(format!("{id}.{ext}")), b).unwrap();
    w("rgba", &t.rgba);
    w("dims", format!("{} {}", t.w, t.h).as_bytes());
    w("src.png", &t.png);
    w("oxi.png", &oxi(2, false)(t));
    w("stored.png", &stored_png(t));
    w("exact.webp", &webp::encode(&t.rgba, t.w, t.h, 6, true));
    w("inexact.webp", &webp::encode(&t.rgba, t.w, t.h, 6, false));
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).map(|i| args[i + 1].clone())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let mut out = std::fs::File::create(&args[2]).unwrap();
    let ns: Vec<usize> = arg(&args, "--n")
        .map_or(vec![usize::MAX; 3], |s| s.split(',').map(|x| x.parse().unwrap()).collect());
    let slow_n: usize = arg(&args, "--slow-n").map_or(8, |s| s.parse().unwrap());
    let only = arg(&args, "--only");
    let lods: Vec<Vec<Tile>> = (1..=3).map(|l| load(&root, l, ns[l as usize - 1], &mut out)).collect();
    lods.iter().for_each(|t| stats(t));

    if let Some(dir) = arg(&args, "--emit") {
        for (id, t) in lods.iter().flat_map(|l| pick(l.iter().collect(), 3)).enumerate() {
            emit(Path::new(&dir), t, &id.to_string());
        }
        emit(Path::new(&dir), &synthetic(), "syn");
        return;
    }
    for lod in &lods {
        for t in lod {
            planes::encode(Layout::Predicted, 9, &t.rgba, t.w, t.h, true);
        }
        let totals: Vec<String> = planes::STREAMS
            .iter()
            .zip(&planes::BREAKDOWN)
            .map(|(s, b)| format!("{s} {}", b.swap(0, std::sync::atomic::Ordering::Relaxed)))
            .collect();
        println!("lod {} streams alone at zstd 9: {}", lod.first().map_or(0, |t| t.lod), totals.join(", "));
    }
    for c in candidates() {
        if only.as_ref().is_some_and(|o| !o.split(',').any(|s| c.name.contains(s))) {
            continue;
        }
        for lod in &lods {
            let tiles: Vec<&Tile> = if c.slow { pick(lod.iter().collect(), slow_n) } else { lod.iter().collect() };
            for t in tiles {
                let t0 = Instant::now();
                let blob = (c.enc)(t);
                let enc = t0.elapsed();
                let t0 = Instant::now();
                let back = (c.dec)(&blob);
                let dec = t0.elapsed();
                assert!(back == t.rgba, "{}: {} decodes to different pixels", c.name, t.rel);
                writeln!(
                    out, "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    c.name, t.lod, t.rel, t.png.len(), blob.len(), enc.as_micros(), dec.as_micros()
                )
                .unwrap();
            }
        }
        println!("done {}", c.name);
        std::io::stdout().flush().unwrap();
    }
}
