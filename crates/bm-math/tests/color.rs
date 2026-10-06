//! Colour parsing and blending against Java BlueMap (see `data/color.rs`).

#[rustfmt::skip]
#[path = "data/color.rs"]
mod data;

use bm_math::Color;

fn col(c: &Color) -> [u32; 5] {
    [c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits(), c.premultiplied as u32]
}

fn colors() -> Vec<Color> {
    data::SINGLE.iter().map(|&(argb, pm, ..)| *Color::default().set_int_premultiplied(argb, pm)).collect()
}

#[test]
fn parse() {
    for (input, ok, int, expected) in data::PARSE {
        let mut c = Color::default();
        assert_eq!(c.parse(input).is_ok(), *ok, "'{input}'");
        if !ok {
            c = Color::default();
        }
        assert_eq!((c.get_int(), col(&c)), (*int, *expected), "'{input}'");
    }
}

#[test]
fn single_ops() {
    for (c, (argb, pm, int, [div3, div7, flat, straight, premul])) in colors().iter().zip(data::SINGLE) {
        assert_eq!(c.get_int(), *int, "{argb:x} {pm}");
        assert_eq!(col({ *c }.div(3)), *div3, "div 3 {argb:x} {pm}");
        assert_eq!(col({ *c }.div(7)), *div7, "div 7 {argb:x} {pm}");
        assert_eq!(col({ *c }.flatten()), *flat, "flatten {argb:x} {pm}");
        assert_eq!(col({ *c }.straight()), *straight, "straight {argb:x} {pm}");
        assert_eq!(col({ *c }.premultiplied()), *premul, "premultiplied {argb:x} {pm}");
    }
}

#[test]
fn blending() {
    let colors = colors();
    for &(i, j, [overlay, underlay, add, multiply]) in data::PAIR {
        let (a, b) = (colors[i], colors[j]);
        let bp = *{ b }.premultiplied();
        assert_eq!(col({ a }.overlay(&bp)), overlay, "{a:?} overlay {bp:?}");
        assert_eq!(col({ a }.underlay(&bp)), underlay, "{a:?} underlay {bp:?}");
        assert_eq!(col({ a }.add(&bp)), add, "{a:?} add {bp:?}");
        assert_eq!(col({ a }.multiply(&b)), multiply, "{a:?} multiply {b:?}");
    }
}

#[test]
#[should_panic(expected = "premultiplied")]
fn overlay_rejects_straight_alpha() {
    let translucent = *Color::default().set_int(0x80ff_0000u32 as i32);
    Color::default().overlay(&translucent);
}
