use bm_math::Color;

use super::super::*;

fn tex(key: &str, a: f32) -> Texture {
    Texture {
        key: ResourcePath::key(key),
        color: Color { r: 0.25, g: 0.5, b: 1.0, a, premultiplied: false },
        half_transparent: a < 1.0,
        texture: Some("data:image/png;base64,AA==".into()),
        animation: None,
    }
}

fn pool(entries: &[(&str, f32)]) -> TexturePool {
    entries.iter().map(|(k, a)| (ResourcePath::key(k), tex(k, *a))).collect()
}

fn ids(g: &TextureGallery, keys: &[&str]) -> Vec<u32> {
    keys.iter().map(|k| g.get(Some(&ResourcePath::key(k)))).collect()
}

fn written(g: &TextureGallery) -> String {
    let mut s = String::new();
    g.write_textures_file(&mut s);
    s
}

#[test]
fn fresh_gallery_puts_missing_then_opaque_then_translucent() {
    let mut g = TextureGallery::new();
    g.put_pool(&pool(&[("m:z", 1.0), ("m:glass", 0.5), ("m:a", 1.0), ("m:B", 1.0), ("m:water", 0.9)]));
    assert_eq!(ids(&g, &[MISSING_TEXTURE, "m:B", "m:a", "m:z", "m:glass", "m:water"]), [0, 1, 2, 3, 4, 5]);
    assert_eq!((g.get(Some(&ResourcePath::key("m:unknown"))), g.get(None)), (0, 0));
    assert_eq!(g.len(), 6);
}

#[test]
fn ids_stay_stable_across_reload() {
    let mut g = TextureGallery::new();
    g.put_pool(&pool(&[("m:b", 1.0), ("m:c", 0.5)]));
    let file = written(&g);
    let mut g = TextureGallery::read_textures_file(file.as_bytes()).unwrap();
    g.put_pool(&pool(&[("m:a", 1.0), ("m:c", 1.0), ("m:d", 1.0)]));
    assert_eq!(ids(&g, &[MISSING_TEXTURE, "m:b", "m:c", "m:a", "m:d"]), [0, 1, 2, 3, 4]);
    let reread = TextureGallery::read_textures_file(written(&g).as_bytes()).unwrap();
    assert_eq!(written(&reread), written(&g));
}

#[test]
fn gaps_and_null_slots_become_missing() {
    let file = r##"[null, {"resourcePath":"m:x","color":[1,0,0],"halfTransparent":"TRUE","texture":"t"},
                   {"resourcePath":"m:x","color":"#ff0000"}, {"color":{"r":0.5}}]"##;
    let mut g = TextureGallery::read_textures_file(file.as_bytes()).unwrap();
    assert_eq!(g.len(), 4);
    assert_eq!(ids(&g, &["m:x", "bluemap:missing"]), [1, 3]);
    g.put(ResourcePath::key("m:new"), None);
    let out = written(&g);
    // `~` stands for a backslash
    let missing = r#"{"resourcePath":"bluemap:missing","color":[0.5,0.0,0.5,1.0],"halfTransparent":false,"texture":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAPklEQVR4Xu3MsQkAMAwDQe2/tFPnBB4gpLhG8MpkZpNkZ6AKZKAKZKAKZKAKZKAKZKAKZKAKWg0XD/UPnjg4MbX+EDdeTUwAAAAASUVORK5CYII~u003d"}"#.replace('~', "\\");
    let x = r#"{"resourcePath":"m:x","color":[1.0,0.0,0.0,1.0],"halfTransparent":true,"texture":"t"}"#;
    let partial = missing.replacen("[0.5,0.0,0.5,1.0]", "[0.5,0.0,0.0,1.0]", 1);
    let new = missing.replacen("bluemap:missing", "m:new", 1);
    assert_eq!(out, format!("[{missing},{x},{missing},{partial},{new}]"));
}

#[test]
fn bad_files_are_errors() {
    for bad in ["null", "{}", "[1]", r#"[{"resourcePath":null}]"#, r#"[{"color":null}]"#, r#"[{"color":[1,2]}]"#] {
        assert!(TextureGallery::read_textures_file(bad.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn writes_animation_and_floats_like_gson() {
    let mut t = tex("m:lava", 1.0);
    t.color = Color { r: 0.1, g: 0.0001, b: 0.0, a: 1.0, premultiplied: false };
    t.animation = Some(AnimationMeta { interpolate: true, frametime: 2, ..Default::default() });
    let mut g = TextureGallery::new();
    g.put(t.key.clone(), Some(t));
    let expected = r#"[{"resourcePath":"m:lava","color":[0.10000000149011612,9.999999747378752E-5,0.0,1.0],"halfTransparent":false,"texture":"data:image/png;base64,AA~u003d~u003d","animation":{"interpolate":true,"width":1,"height":1,"frametime":2}}]"#
        .replace('~', "\\");
    assert_eq!(written(&g), expected);
}
