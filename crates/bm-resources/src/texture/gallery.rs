//! The per-map material table written as `textures.json` (`core/.../map/TextureGallery.java`): ids are the
//! PRBM material indices and stay stable across restarts by reading the previous file first.

use std::borrow::Cow;
use std::collections::HashMap;

use bm_math::Color;
use serde_json::Value;

use super::gson::{self, GsonError};
use super::{AnimationMeta, Error, MISSING_TEXTURE, Texture, TexturePool};
use crate::key::ResourcePath;

#[derive(Clone, Debug, Default)]
pub struct TextureGallery {
    mappings: HashMap<ResourcePath, Mapping>,
    next_id: u32,
}

#[derive(Clone, Debug)]
struct Mapping {
    id: u32,
    texture: Option<Texture>,
}

impl TextureGallery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.mappings.clear();
        self.next_id = 0;
    }

    /// Number of slots in the written file.
    pub fn len(&self) -> u32 {
        self.next_id
    }

    pub fn is_empty(&self) -> bool {
        self.next_id == 0
    }

    /// The material id of `key` (`None` means the missing texture); 0 when unknown.
    pub fn get(&self, key: Option<&ResourcePath>) -> u32 {
        let missing;
        let key = match key {
            Some(k) => k,
            None => {
                missing = ResourcePath::key(MISSING_TEXTURE);
                &missing
            }
        };
        self.mappings.get(key).map_or(0, |m| m.id)
    }

    /// A new key takes the next id; an existing one keeps its id and takes `texture` unless it is `None`.
    pub fn put(&mut self, key: ResourcePath, texture: Option<Texture>) {
        match self.mappings.get_mut(&key) {
            Some(mapping) => {
                if texture.is_some() {
                    mapping.texture = texture;
                }
            }
            None => {
                self.mappings.insert(key, Mapping { id: self.next_id, texture });
                self.next_id += 1;
            }
        }
    }

    /// The missing texture first, then the pool with opaque textures before translucent ones, each by key.
    pub fn put_pool(&mut self, pool: &TexturePool) {
        self.put(ResourcePath::key(MISSING_TEXTURE), None);
        let mut entries: Vec<(&ResourcePath, &Texture)> = pool.iter().collect();
        // String.compareTo orders by UTF-16 units
        entries.sort_by(|a, b| {
            (a.1.color_premultiplied().a < 1.0)
                .cmp(&(b.1.color_premultiplied().a < 1.0))
                .then_with(|| a.0.as_str().encode_utf16().cmp(b.0.as_str().encode_utf16()))
        });
        for (key, texture) in entries {
            self.put(key.clone(), Some(texture.clone()));
        }
    }

    /// `readTexturesFile`. Null slots still take an id; a repeated key keeps its first id.
    pub fn read_textures_file(json: &[u8]) -> Result<Self, Error> {
        let root = crate::json::parse(&String::from_utf8_lossy(json))?;
        let items = match &root {
            Value::Null => return Err(Error::Gallery("Texture data is empty!")),
            Value::Array(items) => items,
            _ => return Err(GsonError("expected an array").into()),
        };
        let mut gallery = Self { mappings: HashMap::new(), next_id: items.len() as u32 };
        for (id, item) in items.iter().enumerate() {
            if item.is_null() {
                continue;
            }
            let texture = read_texture(item)?;
            gallery.mappings.entry(texture.key.clone()).or_insert(Mapping { id: id as u32, texture: Some(texture) });
        }
        Ok(gallery)
    }

    /// `writeTexturesFile`: a compact Gson array; unused ids are `Texture.MISSING`.
    pub fn write_textures_file(&self, out: &mut String) {
        let missing = Texture::missing();
        let mut slots: Vec<Cow<'_, Texture>> = vec![Cow::Borrowed(&missing); self.next_id as usize];
        for (key, mapping) in &self.mappings {
            slots[mapping.id as usize] = match &mapping.texture {
                Some(t) => Cow::Borrowed(t),
                None => Cow::Owned(Texture::missing_for(key.clone())),
            };
        }
        out.push('[');
        for (i, texture) in slots.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_texture(texture, out);
        }
        out.push(']');
    }
}

/// Gson's reflective read over the private no-arg constructor, which starts from `Texture.MISSING`'s values.
fn read_texture(v: &Value) -> Result<Texture, Error> {
    let mut t = Texture::missing();
    for (name, value) in gson::object(v)? {
        match name.as_str() {
            "resourcePath" => t.key = gson::key(value)?,
            "color" => t.color = read_color(value)?,
            "halfTransparent" => {
                if let Some(b) = gson::lenient_boolean(value)? {
                    t.half_transparent = b;
                }
            }
            "texture" => t.texture = gson::string(value)?.map(Into::into),
            "animation" if value.is_null() => t.animation = None,
            "animation" => t.animation = Some(AnimationMeta::from_mcmeta(value)?),
            _ => {}
        }
    }
    Ok(t)
}

/// `ColorAdapter.read`. It doesn't consume a null token, which fails the whole file upstream.
fn read_color(v: &Value) -> Result<Color, Error> {
    let mut c = Color::default();
    match v {
        Value::Array(items) => {
            if !(3..=4).contains(&items.len()) {
                return Err(GsonError("color array must have 3 or 4 entries").into());
            }
            let ch = |i: usize| items.get(i).map_or(Ok(1.0), |v| gson::double(v).map(|d| d as f32));
            c.set(ch(0)?, ch(1)?, ch(2)?, ch(3)?, false);
        }
        Value::Object(members) => {
            c.a = 1.0;
            for (name, value) in members {
                let v = gson::double(value)? as f32;
                match name.as_str() {
                    "r" => c.r = v,
                    "g" => c.g = v,
                    "b" => c.b = v,
                    "a" => c.a = v,
                    _ => {}
                }
            }
        }
        Value::String(s) => {
            c.parse(s).map_err(|_| GsonError("invalid color string"))?;
        }
        Value::Number(_) => {
            let mut argb = gson::int(v)?;
            if argb as u32 & 0xFF00_0000 == 0 {
                argb |= 0xFF00_0000u32 as i32;
            }
            c.set_int(argb);
        }
        _ => return Err(GsonError("unexpected token for a color").into()),
    }
    Ok(c)
}

/// Field order is `Texture`'s declaration order; null fields are omitted. `ColorAdapter.write` straightens.
fn write_texture(t: &Texture, out: &mut String) {
    out.push_str("{\"resourcePath\":");
    gson::write_string(out, t.key.as_str());
    out.push_str(",\"color\":[");
    let mut c = t.color;
    c.straight();
    for (i, ch) in [c.r, c.g, c.b, c.a].into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        gson::write_double(out, f64::from(ch));
    }
    out.push_str("],\"halfTransparent\":");
    out.push_str(if t.half_transparent { "true" } else { "false" });
    if let Some(data) = &t.texture {
        out.push_str(",\"texture\":");
        gson::write_string(out, data);
    }
    if let Some(animation) = &t.animation {
        out.push_str(",\"animation\":");
        animation.write_json(out);
    }
    out.push('}');
}
