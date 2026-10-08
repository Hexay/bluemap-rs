use std::io::{Cursor, Write};
use std::sync::Arc;

use serde_json::json;
use zip::write::SimpleFileOptions;

use super::*;
use crate::vfs::Pack;

fn png(
    w: u32,
    h: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    setup: impl FnOnce(&mut png::Encoder<&mut Vec<u8>>),
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    setup(&mut enc);
    enc.write_header().unwrap().write_image_data(data).unwrap();
    out
}

fn rgba(w: u32, h: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
    png(w, h, png::ColorType::Rgba, png::BitDepth::Eight, pixels.concat().as_slice(), |_| {})
}

fn pixels(bytes: &[u8]) -> Vec<[u8; 4]> {
    decode_png(bytes).unwrap().image.pixels.as_chunks::<4>().0.to_vec()
}

#[test]
fn decodes_every_color_type_to_rgba8() {
    use png::{BitDepth::*, ColorType::*};
    // 8-bit gray is linear gray to Java: getRGB maps it through the sRGB LUT (tests/png_ref.rs covers all of it)
    assert_eq!(pixels(&png(2, 1, Grayscale, Eight, &[0, 200], |_| {})), [[0, 0, 0, 255], [229, 229, 229, 255]]);
    // 4-bit gray 0b0001_1111: 1*17 and 15*17
    assert_eq!(pixels(&png(2, 1, Grayscale, Four, &[0x1F], |_| {})), [[17, 17, 17, 255], [255, 255, 255, 255]]);
    assert_eq!(pixels(&png(1, 1, GrayscaleAlpha, Eight, &[9, 128], |_| {})), [[53, 53, 53, 128]]);
    assert_eq!(pixels(&png(1, 1, Rgb, Eight, &[1, 2, 3], |_| {})), [[1, 2, 3, 255]]);
    let rgb_trns = png(2, 1, Rgb, Eight, &[1, 2, 3, 4, 5, 6], |e| e.set_trns(vec![0, 1, 0, 2, 0, 3]));
    assert_eq!(pixels(&rgb_trns), [[1, 2, 3, 0], [4, 5, 6, 255]]);
    let indexed = png(2, 1, Indexed, Eight, &[0, 1], |e| {
        e.set_palette(vec![10, 20, 30, 40, 50, 60]);
        e.set_trns(vec![77]);
    });
    assert_eq!(pixels(&indexed), [[10, 20, 30, 77], [40, 50, 60, 255]]);
    let sixteen = png(1, 1, Rgba, Sixteen, &[0xFF, 0xFF, 0x80, 0x00, 0x00, 0x00, 0x7F, 0x7F], |_| {});
    assert_eq!(pixels(&sixteen), [[255, 128, 0, 127]]);
    let gray_trns = png(2, 1, Grayscale, Eight, &[7, 8], |e| e.set_trns(vec![0, 7]));
    assert_eq!(pixels(&gray_trns), [[46, 46, 46, 0], [50, 50, 50, 255]]);
}

#[test]
fn reencodes_like_imageio() {
    use png::{BitDepth::*, ColorType::*};
    let managed = png(1, 1, Rgb, Eight, &[1, 2, 3], |e| e.set_source_gamma(png::ScaledFloat::new(0.45455)));
    let t = Texture::from_image(ResourcePath::key("a"), &decode_png(&managed).unwrap(), None);
    let bytes = t.png_bytes().unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"gAMA"), "colour management chunks are dropped");
    assert_eq!(t.decode_image().unwrap().pixels, [1, 2, 3, 255]);
    let indexed = png(2, 1, Indexed, Four, &[0x10], |e| e.set_palette(vec![10, 20, 30, 40, 50, 60]));
    let t = Texture::from_image(ResourcePath::key("a"), &decode_png(&indexed).unwrap(), None);
    let bytes = t.png_bytes().unwrap();
    assert_eq!(bytes[24..26], [4, 3], "palette images keep their bit depth");
    assert_eq!(decode_png(&bytes).unwrap().image.pixels, [40, 50, 60, 255, 10, 20, 30, 255]);
}

#[test]
fn half_transparency_needs_a_partial_alpha_pixel() {
    let img = |a: u8| decode_png(&rgba(2, 1, &[[1, 1, 1, 0], [1, 1, 1, a]])).unwrap().image;
    assert!(!img(255).half_transparent());
    assert!(img(254).half_transparent());
    assert!(img(1).half_transparent());
}

#[test]
fn average_color_is_the_premultiplied_mean_stored_straight() {
    let img = decode_png(&rgba(1, 2, &[[255, 0, 0, 255], [0, 0, 255, 0]])).unwrap();
    let t = Texture::from_image(ResourcePath::key("a"), &img, None);
    assert_eq!((t.color.r, t.color.g, t.color.b, t.color.a), (1.0, 0.0, 0.0, 0.5));
    assert!(!t.color.premultiplied);
    let img = decode_png(&rgba(1, 2, &[[51, 102, 0, 51], [255, 255, 255, 255]])).unwrap().image;
    let c = img.average_color();
    let (a1, a2) = (51.0f32 / 255.0, 1.0f32);
    assert_eq!(c.a, (a1 + a2) * 0.5);
    assert_eq!(c.r, (51.0f32 / 255.0 * a1 + 1.0) * 0.5);
}

#[test]
fn animation_meta() {
    let src = br#"{"animation": {"interpolate": true, "frametime": 2.9, "width": "3",
        "frames": [1, {"index": 2, "time": 7}, {"index": 3}]}, "other": [1]}"#;
    let meta = AnimationMeta::parse_mcmeta(src).unwrap().unwrap();
    let frames =
        vec![FrameMeta { index: 1, time: 2 }, FrameMeta { index: 2, time: 7 }, FrameMeta { index: 3, time: 2 }];
    assert_eq!(meta, AnimationMeta { interpolate: true, width: 3, height: 1, frametime: 2, frames: Some(frames) });
    assert_eq!(AnimationMeta::parse_mcmeta(b"  \n").unwrap(), None);
    assert_eq!(AnimationMeta::parse_mcmeta(b"{}").unwrap(), Some(AnimationMeta::default()));
    assert_eq!(AnimationMeta::parse_mcmeta(br#"{"animation":{"frames":[]}}"#).unwrap().unwrap().frames, None);
    assert!(AnimationMeta::parse_mcmeta(br#"{"animation":{"frames":["1"]}}"#).is_err());
    assert!(AnimationMeta::parse_mcmeta(br#"{"animation":{"width":1.5}}"#).is_err());
    // textures.json shape read through the mcmeta adapter: upstream drops the values
    let inner = json!({"interpolate": true, "width": 1, "height": 1, "frametime": 10});
    assert_eq!(AnimationMeta::from_mcmeta(&inner).unwrap(), AnimationMeta::default());
    assert_eq!(AnimationMeta::from_fields(&inner).unwrap().frametime, 10);
    let mut out = String::new();
    AnimationMeta::parse_mcmeta(src).unwrap().unwrap().write_json(&mut out);
    let expected = r#"{"interpolate":true,"width":3,"height":1,"frametime":2,"frames":[{"index":1,"time":2},{"index":2,"time":7},{"index":3,"time":2}]}"#;
    assert_eq!(out, expected);
}

#[test]
fn atlas_sources() {
    let atlas = Atlas::parse(
        r#"{"sources": [
            {"type": "single", "resource": "Mod:Block/X"},
            {"type": "minecraft:directory", "source": "block", "prefix": "block/"},
            {"type": "directory", "source": "block", "prefix": "block/"},
            {"type": "filter", "pattern": {}}, {"type": "minecraft:filter"},
            {"type": "unstitch", "resource": "a", "regions": [{"sprite": "b", "x": 1}, {"sprite": "b", "x": 1}, null]},
            {"type": "paletted_permutations", "textures": ["t", "t"], "palette_key": "k", "permutations": {"s": "p"}},
            {"type": "custom:thing"}
        ]}"#,
    )
    .unwrap();
    let s = &atlas.sources;
    assert_eq!(s.len(), 7, "subclass sources never dedupe, filters do");
    assert_eq!(s[0], Source::Single { resource: Some(ResourcePath::key("Mod:Block/X")), sprite: None });
    assert_eq!(s[1], s[2]);
    let Source::Unstitch { regions: Some(regions), divisor_x, .. } = &s[4] else { panic!() };
    assert_eq!((regions.len(), *divisor_x), (2, 0.0));
    let Source::PalettedPermutations { textures: Some(t), separator, .. } = &s[5] else { panic!() };
    assert_eq!((t.len(), separator.as_deref()), (1, Some("_")));
    assert_eq!(s[6], Source::Other(Some(ResourcePath::key("custom:thing"))));
    assert!(Atlas::parse(r#"{"sources":[{"type":"single","resource":null}]}"#).is_err());
}

fn zip_pack(files: &[(&str, Vec<u8>)]) -> Pack {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in files {
        w.start_file(*name, SimpleFileOptions::default()).unwrap();
        w.write_all(body).unwrap();
    }
    Pack::zip(Arc::from(w.finish().unwrap().into_inner()), "test".into()).unwrap()
}

#[test]
fn loads_and_bakes_like_upstream() {
    let red = rgba(1, 1, &[[255, 0, 0, 255]]);
    let strip = rgba(2, 2, &[[1, 0, 0, 255], [2, 0, 0, 255], [3, 0, 0, 255], [4, 0, 0, 128]]);
    let key = rgba(2, 1, &[[10, 10, 10, 255], [20, 20, 20, 255]]);
    let value = rgba(2, 1, &[[0, 99, 0, 255], [0, 0, 99, 128]]);
    let src = rgba(2, 1, &[[10, 10, 10, 128], [5, 5, 5, 255]]);
    let atlas = br#"{"sources":[
        {"type":"directory","source":"block","prefix":"b/"},
        {"type":"single","resource":"minecraft:misc/red","sprite":"minecraft:renamed"},
        {"type":"unstitch","resource":"minecraft:misc/strip","divisor_x":2,"divisor_y":2,
         "regions":[{"sprite":"minecraft:cut","x":1,"y":1,"width":1,"height":1},
                    {"sprite":"minecraft:outside","x":1,"y":1,"width":2,"height":1}]},
        {"type":"paletted_permutations","textures":["minecraft:trim/src"],"palette_key":"minecraft:trim/key",
         "permutations":{"gold":"minecraft:trim/value"}}]}"#;
    let high = zip_pack(&[
        ("assets/minecraft/atlases/blocks.json", atlas.to_vec()),
        ("assets/minecraft/textures/block/stone.png", b"not a png".to_vec()),
        ("assets/minecraft/textures/block/anim.png", red.clone()),
        ("assets/minecraft/textures/block/anim.png.mcmeta", br#"{"animation":{"frametime":3}}"#.to_vec()),
    ]);
    let low = zip_pack(&[
        ("assets/minecraft/textures/block/stone.png", red.clone()),
        ("assets/minecraft/textures/block/unused.png", red.clone()),
        ("assets/minecraft/textures/misc/red.png", red.clone()),
        ("assets/minecraft/textures/misc/strip.png", strip),
        ("assets/minecraft/textures/trim/key.png", key),
        ("assets/minecraft/textures/trim/value.png", value),
        ("assets/minecraft/textures/trim/src.png", src),
    ]);
    let packs = [high, low];
    let atlas = Atlas::load_blocks(&packs);
    assert_eq!(atlas.sources.len(), 4);
    let inputs = [
        "minecraft:b/unused",
        "minecraft:misc/strip",
        "minecraft:trim/key",
        "minecraft:trim/value",
        "minecraft:trim/src",
    ];
    let used = |k: &ResourcePath| !inputs.contains(&k.as_str());
    let pool = load_textures(&packs, &atlas, &used);
    let mut keys: Vec<&str> = pool.keys().map(ResourcePath::as_str).collect();
    keys.sort();
    let expected = [
        "minecraft:b/anim",
        "minecraft:b/stone",
        "minecraft:cut",
        "minecraft:misc/strip",
        "minecraft:renamed",
        "minecraft:trim/key",
        "minecraft:trim/src",
        "minecraft:trim/src_gold",
        "minecraft:trim/value",
    ];
    assert_eq!(keys, expected, "bad PNG falls through to the lower pack; bake inputs load unfiltered");
    let id = |k: &str| ResourcePath::key(k);
    assert_eq!(pool[&id("minecraft:b/anim")].animation.as_ref().unwrap().frametime, 3);
    assert_eq!(pool[&id("minecraft:b/stone")].decode_image().unwrap().pixels, [255, 0, 0, 255]);
    let cut = pool[&id("minecraft:cut")].decode_image().unwrap();
    assert_eq!(cut.pixels, [4, 0, 0, 128]);
    let gold = pool[&id("minecraft:trim/src_gold")].decode_image().unwrap();
    // 10 -> (0,99,0,255) times source alpha 128; 5 is not in the palette and keeps its colour
    let alpha = ((128.0f32 / 255.0) * 255.0) as u8;
    assert_eq!(gold.pixels, [0, 99, 0, alpha, 5, 5, 5, 255]);
}

#[test]
fn missing_texture() {
    let m = Texture::missing();
    assert_eq!(m.key.as_str(), "bluemap:missing");
    let img = m.decode_image().unwrap();
    assert_eq!((img.width, img.height), (16, 16));
    assert_eq!(MISSING_TEXTURE, "bluemap:block/missing");
}

#[path = "gallery_tests.rs"]
mod gallery_tests;
