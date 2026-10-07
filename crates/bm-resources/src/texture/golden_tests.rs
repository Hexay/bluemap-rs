//! Parity with Java BlueMap 5.28 on MC 26.3: the textures it loaded for `work/bluemap/vanilla`, and the
//! `textures.json` it wrote.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use super::*;
use crate::client_jar::MinecraftVersion;

fn read_gz(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap()).read_to_end(&mut out).unwrap();
    out
}

fn decode_url(url: &str) -> RgbaImage {
    let t = Texture { texture: Some(url.into()), ..Texture::missing() };
    t.decode_image().unwrap()
}

fn first_diff(a: &str, b: &str) -> String {
    let i = a.bytes().zip(b.bytes()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let ctx = |s: &str| s.get(i.saturating_sub(80)..(i + 80).min(s.len())).unwrap_or("").to_owned();
    format!("at byte {i} (lengths {} vs {}):\n ours: {}\n java: {}", a.len(), b.len(), ctx(a), ctx(b))
}

#[test]
#[ignore = "needs work/bluemap/vanilla (client jar 26.3 and BlueMap's textures.json.gz)"]
fn textures_match_java_bluemap_26_3() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // `work/` is git-ignored, so a worktree finds it in an enclosing checkout
    let vanilla =
        repo.ancestors().map(|d| d.join("work/bluemap/vanilla")).find(|d| d.is_dir()).expect("work/bluemap/vanilla");
    let offline = Err(crate::Error::Http("offline".into()));
    let mc = MinecraftVersion::load_with(offline, Some("26.3"), &vanilla.join("data"), false).unwrap();
    let mut packs = crate::packs::expand_root(&repo.join("assets/resourceExtensions"), mc.resource_pack_version);
    packs.extend(crate::packs::expand_root(&mc.resource_pack, mc.resource_pack_version));

    let java_bytes = read_gz(&vanilla.join("web/maps/vanilla/textures.json.gz"));
    let java_text = String::from_utf8(java_bytes.clone()).unwrap();
    let java: Vec<Value> = serde_json::from_slice(&java_bytes).unwrap();
    let java_by_key: HashMap<&str, &Value> = java.iter().map(|t| (t["resourcePath"].as_str().unwrap(), t)).collect();
    let used: HashSet<ResourcePath> = java_by_key.keys().map(|k| ResourcePath::key(k)).collect();

    let atlas = Atlas::load_blocks(&packs);
    let started = std::time::Instant::now();
    let pool = load_textures(&packs, &atlas, &|k| used.contains(k));
    eprintln!("loaded {} textures in {:?}", pool.len(), started.elapsed());

    let missing: Vec<&str> =
        java_by_key.keys().copied().filter(|k| !pool.contains_key(&ResourcePath::key(k))).collect();
    let extra: Vec<&str> = pool.keys().map(ResourcePath::as_str).filter(|k| !java_by_key.contains_key(k)).collect();
    let (mut color, mut alpha, mut anim, mut pixels, mut urls) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0);
    let mut url_diffs = Vec::new();
    for (key, ours) in &pool {
        let Some(j) = java_by_key.get(key.as_str()) else { continue };
        let jc: Vec<f64> = j["color"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let oc = [ours.color.r, ours.color.g, ours.color.b, ours.color.a].map(f64::from);
        if jc.iter().zip(oc).any(|(j, o)| (j - o).abs() > 1e-6) {
            color.push(format!("{key}: ours {oc:?} java {jc:?}"));
        }
        if j["halfTransparent"].as_bool().unwrap() != ours.half_transparent {
            alpha.push(key.to_string());
        }
        let janim = j.get("animation").map(|a| AnimationMeta::from_fields(a).unwrap());
        if janim != ours.animation {
            anim.push(format!("{key}: ours {:?} java {janim:?}", ours.animation));
        }
        let jurl = j["texture"].as_str().unwrap();
        urls += 1;
        if ours.texture.as_deref() != Some(jurl) {
            url_diffs.push(key.to_string());
        }
        if decode_url(jurl) != ours.decode_image().unwrap() {
            pixels.push(key.to_string());
        }
    }
    eprintln!(
        "keys: java {} ours {} missing {missing:?} extra {extra:?}\ncolor {color:#?}\nhalfTransparent {alpha:?}\n\
         animation {anim:#?}\npixels {pixels:?}\nidentical data URLs {}/{urls}, differing {:?}",
        java_by_key.len(),
        pool.len(),
        urls - url_diffs.len(),
        &url_diffs[..url_diffs.len().min(20)]
    );

    let mut fresh = TextureGallery::new();
    fresh.put_pool(&pool);
    let mut ours_fresh = String::new();
    fresh.write_textures_file(&mut ours_fresh);
    let mut reloaded = TextureGallery::read_textures_file(&java_bytes).unwrap();
    reloaded.put_pool(&pool);
    let mut ours_reloaded = String::new();
    reloaded.write_textures_file(&mut ours_reloaded);

    assert!(missing.is_empty() && extra.is_empty(), "key sets differ");
    assert!(color.is_empty() && alpha.is_empty() && anim.is_empty() && pixels.is_empty(), "texture data differs");
    assert!(url_diffs.is_empty(), "data URLs differ");
    assert!(ours_fresh == java_text, "fresh gallery: {}", first_diff(&ours_fresh, &java_text));
    assert!(ours_reloaded == java_text, "reloaded gallery: {}", first_diff(&ours_reloaded, &java_text));
}
