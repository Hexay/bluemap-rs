//! Lowres PNG encoder settings vs size and single-thread speed, on real lowres tiles (pixels re-encoded losslessly).
//! Usage: cargo run -p bm-format --profile profiling --example png_bench -- <webroot maps dir>...

use std::path::{Path, PathBuf};
use std::time::Instant;

use bm_format::lowres::LowresTile;
use png::{DeflateCompression as D, Filter as F};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "png") && p.to_string_lossy().replace('\\', "/").contains("/tiles/") {
            out.push(p);
        }
    }
}

fn encode(w: u32, h: u32, rgba: &[u8], d: D, f: F, out: &mut Vec<u8>) {
    out.clear();
    let mut e = png::Encoder::new(&mut *out, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_deflate_compression(d);
    e.set_filter(f);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(rgba).unwrap();
    wr.finish().unwrap();
}

fn main() {
    let mut files = Vec::new();
    for d in std::env::args().skip(1) {
        walk(Path::new(&d), &mut files);
    }
    files.sort();
    // (stored bytes, w, h, rgba, tile, original)
    let mut tiles = Vec::new();
    let mut decode_ns = 0u128;
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let mut dec = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
        dec.set_transformations(png::Transformations::EXPAND);
        let mut r = dec.read_info().unwrap();
        let mut rgba = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut rgba).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        let size = [info.width as usize - 1, info.height as usize / 2 - 1];
        let t = Instant::now();
        let tile = LowresTile::decode_png(&bytes, size).unwrap();
        decode_ns += t.elapsed().as_nanos();
        tiles.push((bytes.len(), info.width, info.height, rgba, tile));
    }
    let stored: usize = tiles.iter().map(|t| t.0).sum();
    println!("{} lowres PNGs, stored {stored} B; product decode_png {:.2} ms/tile", tiles.len(), decode_ns as f64 / 1e6 / tiles.len() as f64);

    let mut out = Vec::new();
    let t = Instant::now();
    let mut bytes = 0;
    for (.., tile) in &tiles {
        tile.encode_png(&mut out).unwrap();
        bytes += out.len();
    }
    let ns = t.elapsed().as_nanos();
    println!("| setting | bytes | vs stored | enc ms/tile |");
    println!("|---|---|---|---|");
    let n = tiles.len() as f64;
    println!("| product encode_png (zlib 9, Adaptive) | {bytes} | {:.3} | {:.2} |", bytes as f64 / stored as f64, ns as f64 / 1e6 / n);
    let variants: [(&str, D, F); 9] = [
        ("fdeflate, Up (png Fastest)", D::FdeflateUltraFast, F::Up),
        ("fdeflate, Adaptive (png Fast)", D::FdeflateUltraFast, F::Adaptive),
        ("zlib 1, None", D::Level(1), F::NoFilter),
        ("zlib 4, None", D::Level(4), F::NoFilter),
        ("zlib 6, None", D::Level(6), F::NoFilter),
        ("zlib 6, Adaptive (png Balanced)", D::Level(6), F::Adaptive),
        ("zlib 9, None", D::Level(9), F::NoFilter),
        ("zlib 9, Up", D::Level(9), F::Up),
        ("zlib 9, Adaptive", D::Level(9), F::Adaptive),
    ];
    for (name, d, f) in variants {
        let t = Instant::now();
        let mut bytes = 0;
        for (_, w, h, rgba, _) in &tiles {
            encode(*w, *h, rgba, d, f, &mut out);
            bytes += out.len();
        }
        let ns = t.elapsed().as_nanos();
        println!("| {name} | {bytes} | {:.3} | {:.2} |", bytes as f64 / stored as f64, ns as f64 / 1e6 / n);
    }
}
