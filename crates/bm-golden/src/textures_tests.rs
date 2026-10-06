use super::*;

fn encode(w: u32, h: u32, color: png::ColorType, palette: Option<&[u8]>, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    if let Some(p) = palette {
        enc.set_palette(p.to_vec());
    }
    enc.write_header().unwrap().write_image_data(data).unwrap();
    out
}

fn rgba(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
    let data: Vec<u8> = (0..w * h * 4).map(|i| i as u8).collect();
    (encode(w, h, png::ColorType::Rgba, None, &data), data)
}

fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(Cursor::new(png_bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn data_uri(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

fn texture(png: &[u8], animation: Option<serde_json::Value>) -> Texture {
    Texture {
        resource_path: "minecraft:block/test".into(),
        color: [1.0; 4],
        half_transparent: false,
        texture: data_uri(png),
        animation,
    }
}

const JSON: &str = r#"[
  {"resourcePath": "bluemap:block/missing", "color": [0.5, 0.0, 0.5, 1.0], "halfTransparent": false,
   "texture": "data:image/png;base64,AA=="},
  {"resourcePath": "minecraft:block/water_still", "color": [0.1, 0.2, 0.8, 0.7], "halfTransparent": true,
   "texture": "data:image/png;base64,AA==", "animation": {"frametime": 2}},
  {"resourcePath": "minecraft:block/stone", "color": [0.5, 0.5, 0.5, 1.0], "halfTransparent": false,
   "texture": "data:image/png;base64,AA==", "animation": null, "unknownField": 1}
]"#;

#[test]
fn parses_textures_json() {
    let t = parse_textures(JSON.as_bytes()).unwrap();
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].resource_path, "bluemap:block/missing");
    assert!(t[0].animation.is_none(), "absent animation defaults to None");
    assert_eq!(t[1].color, [0.1, 0.2, 0.8, 0.7]);
    assert!(t[1].half_transparent);
    assert_eq!(t[1].animation.as_ref().unwrap()["frametime"], 2);
    assert!(t[2].animation.is_none(), "null animation is None");
}

#[test]
fn names_match_full_parse_by_index() {
    let names = parse_texture_names(JSON.as_bytes()).unwrap();
    assert_eq!(names, ["bluemap:block/missing", "minecraft:block/water_still", "minecraft:block/stone"]);
    assert_eq!(parse_texture_names(br#"[{"resourcePath": "a"}]"#).unwrap(), ["a"]);
}

#[test]
fn malformed_json_errors() {
    for bad in [&b""[..], b"{}", b"[{}]", b"[{\"resourcePath\": 3}]", b"[", b"\xff"] {
        assert!(parse_texture_names(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        assert!(parse_textures(bad).is_err());
    }
    let missing_color = br#"[{"resourcePath": "a", "halfTransparent": false, "texture": ""}]"#;
    assert!(parse_textures(missing_color).is_err());
    assert!(parse_texture_names(missing_color).is_ok(), "names need only resourcePath");
}

#[test]
fn png_decodes_data_uri() {
    let (png, _) = rgba(2, 2);
    assert_eq!(texture(&png, None).png().unwrap(), png);
}

#[test]
fn png_rejects_bad_uris() {
    let mut t = texture(&[], None);
    t.texture = "data:image/jpeg;base64,AA==".into();
    assert!(t.png().unwrap_err().to_string().contains("minecraft:block/test"));
    t.texture = "data:image/png;base64,!!!".into();
    assert!(t.png().is_err());
    t.texture = String::new();
    assert!(t.png().is_err());
}

#[test]
fn still_frame_png_is_unchanged() {
    let (png, _) = rgba(2, 4);
    assert_eq!(texture(&png, None).frame_png().unwrap(), png);
}

#[test]
fn animated_strip_is_cropped_to_top_frame() {
    let (png, data) = rgba(2, 6);
    let (w, h, px) = decode(&texture(&png, Some(serde_json::json!({}))).frame_png().unwrap());
    assert_eq!((w, h), (2, 2));
    assert_eq!(px, data[..2 * 2 * 4]);
}

#[test]
fn animated_palette_strip_is_cropped() {
    let palette = [255, 0, 0, 0, 255, 0];
    let png = encode(1, 2, png::ColorType::Indexed, Some(&palette), &[1, 0]);
    let (w, h, px) = decode(&texture(&png, Some(serde_json::json!({}))).frame_png().unwrap());
    assert_eq!((w, h), (1, 1));
    assert_eq!(px, [0, 255, 0]);
}

#[test]
fn animated_non_png_errors() {
    let t = texture(b"not a png", Some(serde_json::json!({})));
    assert!(format!("{:#}", t.frame_png().unwrap_err()).contains("minecraft:block/test"));
}

#[test]
fn file_stem_is_filesystem_safe() {
    assert_eq!(texture(&[], None).file_stem(), "minecraft_block_test");
}
