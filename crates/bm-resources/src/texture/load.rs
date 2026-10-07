//! Loading the atlas' textures from every pack (`ResourcePack.loadResources` texture pass, `Source.load`)
//! and baking generated sprites (`Source.bake`).
//!
//! Upstream loads sequentially into a first-wins pool, where a file that fails to load leaves the key open for
//! lower-priority packs. Here every key's candidate files are collected in that same order first, then each
//! key is decoded in parallel taking its first candidate that loads, which gives the same pool.

use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

use super::atlas::{Atlas, Source};
use super::image::{DecodedPng, decode_png};
use super::{AnimationMeta, Texture};
use crate::key::ResourcePath;
use crate::vfs::Pack;

pub type TexturePool = HashMap<ResourcePath, Texture>;

/// Every texture `atlas` yields from `packs` (highest priority first) whose key passes `used`, plus the
/// unfiltered inputs of unstitch and paletted-permutation sources, then the baked sprites.
pub fn load_textures(packs: &[Pack], atlas: &Atlas, used: &(dyn Fn(&ResourcePath) -> bool + Sync)) -> TexturePool {
    let mut keys: Vec<(ResourcePath, Vec<(usize, String)>)> = Vec::new();
    let mut index: HashMap<ResourcePath, usize> = HashMap::new();
    let mut push = |key: ResourcePath, pack: usize, file: String| {
        let i = *index.entry(key.clone()).or_insert_with(|| {
            keys.push((key, Vec::new()));
            keys.len() - 1
        });
        keys[i].1.push((pack, file));
    };
    for (pi, pack) in packs.iter().enumerate() {
        for source in &atlas.sources {
            collect(source, pack, used, &mut |key, file| push(key, pi, file));
        }
    }

    let bake_inputs = bake_inputs(atlas);
    let loaded: Vec<(ResourcePath, Texture, Option<DecodedPng>)> = keys
        .into_par_iter()
        .filter_map(|(key, candidates)| {
            let (texture, image) = candidates.iter().find_map(|(pi, file)| load_file(&packs[*pi], file, &key))?;
            let image = bake_inputs.contains(&key).then_some(image);
            Some((key, texture, image))
        })
        .collect();

    let mut pool = TexturePool::with_capacity(loaded.len());
    let mut images = HashMap::new();
    for (key, texture, image) in loaded {
        if let Some(image) = image {
            images.insert(key.clone(), image);
        }
        pool.insert(key, texture);
    }
    for source in &atlas.sources {
        super::bake::bake(source, &mut pool, &mut images, used);
    }
    pool
}

fn texture_file(key: &ResourcePath) -> String {
    format!("assets/{}/textures/{}.png", key.namespace(), key.path())
}

/// `Source.load` for one pack: the (key, file) pairs it would try, in order.
fn collect(
    source: &Source,
    pack: &Pack,
    used: &dyn Fn(&ResourcePath) -> bool,
    push: &mut dyn FnMut(ResourcePath, String),
) {
    match source {
        Source::Single { resource: Some(resource), sprite } => {
            let sprite = sprite.as_ref().unwrap_or(resource);
            if used(sprite) {
                push(sprite.clone(), texture_file(resource));
            }
        }
        Source::Directory { source: Some(dir), prefix } => {
            let prefix = Source::java_str(prefix);
            let dir = dir.trim_matches('/');
            for namespace in pack.list("assets") {
                let base = if dir.is_empty() {
                    format!("assets/{namespace}/textures/")
                } else {
                    format!("assets/{namespace}/textures/{dir}/")
                };
                for file in pack.walk(&base) {
                    let Some(name) = file.strip_prefix(&base).and_then(|f| f.strip_suffix(".png")) else { continue };
                    let key = ResourcePath::key(&format!("{namespace}:{prefix}{name}"));
                    if used(&key) {
                        push(key, file);
                    }
                }
            }
        }
        Source::Unstitch { resource: Some(resource), regions: Some(regions), .. } if !regions.is_empty() => {
            push(resource.clone(), texture_file(resource));
        }
        Source::PalettedPermutations {
            textures: Some(textures),
            palette_key: Some(palette_key),
            permutations: Some(permutations),
            ..
        } if !permutations.is_empty() => {
            let all = textures.iter().chain([palette_key]).chain(permutations.iter().map(|(_, k)| k));
            for key in all {
                push(key.clone(), texture_file(key));
            }
        }
        _ => {}
    }
}

/// Keys whose decoded pixels a bake step reads.
fn bake_inputs(atlas: &Atlas) -> HashSet<ResourcePath> {
    let mut out = HashSet::new();
    for source in &atlas.sources {
        match source {
            Source::Unstitch { resource: Some(r), .. } => {
                out.insert(r.clone());
            }
            Source::PalettedPermutations { textures, palette_key, permutations, .. } => {
                out.extend(textures.iter().flatten().cloned());
                out.extend(palette_key.iter().cloned());
                out.extend(permutations.iter().flatten().map(|(_, k)| k.clone()));
            }
            _ => {}
        }
    }
    out
}

/// `Source.loadTexture`: `None` where upstream gets null or throws (missing file, bad PNG or mcmeta).
fn load_file(pack: &Pack, file: &str, key: &ResourcePath) -> Option<(Texture, DecodedPng)> {
    let decoded = decode_png(&pack.read(file)?).ok()?;
    let animation = match pack.read(&format!("{file}.mcmeta")) {
        Some(meta) => AnimationMeta::parse_mcmeta(&meta).ok()?,
        None => None,
    };
    Some((Texture::from_image(key.clone(), &decoded, animation), decoded))
}
