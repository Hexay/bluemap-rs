//! Texture atlases (`RP/atlas/*.java`): every pack's `assets/minecraft/atlases/blocks.json` merged in load
//! order. Only `minecraft:blocks` is used by BlueMap.

use serde_json::{Map, Value};

use super::gson::{self, GResult, GsonError, java_double_eq};
use crate::key::ResourcePath;
use crate::vfs::Pack;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Atlas {
    pub sources: Vec<Source>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Single {
        resource: Option<ResourcePath>,
        sprite: Option<ResourcePath>,
    },
    Directory {
        source: Option<String>,
        /// `None` only for an explicit JSON null, which Java concatenates as `"null"`.
        prefix: Option<String>,
    },
    Unstitch {
        resource: Option<ResourcePath>,
        divisor_x: f64,
        divisor_y: f64,
        /// `Set<Region>`: deduplicated, order kept; `None` entries are JSON nulls.
        regions: Option<Vec<Option<Region>>>,
    },
    PalettedPermutations {
        textures: Option<Vec<ResourcePath>>,
        /// `None` only for an explicit JSON null.
        separator: Option<String>,
        palette_key: Option<ResourcePath>,
        permutations: Option<Vec<(String, ResourcePath)>>,
    },
    /// `filter`, an unknown type or no type: loads nothing.
    Other(Option<ResourcePath>),
}

#[derive(Clone, Debug)]
pub struct Region {
    pub sprite: Option<ResourcePath>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PartialEq for Region {
    fn eq(&self, o: &Self) -> bool {
        self.sprite == o.sprite
            && java_double_eq(self.x, o.x)
            && java_double_eq(self.y, o.y)
            && java_double_eq(self.width, o.width)
            && java_double_eq(self.height, o.height)
    }
}

impl Atlas {
    /// The `minecraft:blocks` atlas over `packs` (highest priority first). Unparsable files are skipped.
    pub fn load_blocks(packs: &[Pack]) -> Self {
        let mut atlas = Atlas::default();
        for pack in packs {
            if !pack.list("assets/minecraft/atlases").iter().any(|n| n == "blocks.json") {
                continue;
            }
            let Some(src) = pack.read("assets/minecraft/atlases/blocks.json").and_then(|b| String::from_utf8(b).ok())
            else {
                continue;
            };
            if let Ok(parsed) = Self::parse(&src) {
                atlas.add(parsed);
            }
        }
        atlas
    }

    pub fn parse(src: &str) -> Result<Self, super::Error> {
        let root = crate::json::parse(src)?;
        let sources = gson::object(&root)?.get("sources").ok_or(GsonError("atlas without sources"))?;
        let mut atlas = Atlas::default();
        for source in sources.as_array().ok_or(GsonError("atlas sources must be an array"))? {
            if !source.is_null() {
                atlas.insert(Source::parse(source)?);
            }
        }
        Ok(atlas)
    }

    /// `Atlas.add`: appended in order into a `LinkedHashSet`.
    pub fn add(&mut self, other: Atlas) {
        for source in other.sources {
            self.insert(source);
        }
    }

    /// Only base-class sources (`filter`/unknown) ever compare equal upstream: the subclasses' `equals` calls
    /// `Source.equals`, which rejects any class but `Source`.
    fn insert(&mut self, source: Source) {
        if matches!(source, Source::Other(_)) && self.sources.contains(&source) {
            return;
        }
        self.sources.push(source);
    }
}

impl Source {
    fn parse(v: &Value) -> GResult<Self> {
        let obj = gson::object(v)?;
        let ty = opt(obj, "type", gson::key)?;
        let source = match ty.as_ref().map(ResourcePath::as_str) {
            Some("minecraft:single") => {
                Source::Single { resource: opt(obj, "resource", gson::key)?, sprite: opt(obj, "sprite", gson::key)? }
            }
            Some("minecraft:directory") => Source::Directory {
                source: opt(obj, "source", gson::string)?.flatten(),
                prefix: opt(obj, "prefix", gson::string)?.unwrap_or(Some(String::new())),
            },
            Some("minecraft:unstitch") => Source::Unstitch {
                resource: opt(obj, "resource", gson::key)?,
                divisor_x: primitive_double(obj, "divisor_x")?,
                divisor_y: primitive_double(obj, "divisor_y")?,
                regions: nullable(obj, "regions", |v| parse_set(v, |r| nullable_value(r, Region::parse)))?,
            },
            Some("minecraft:paletted_permutations") => Source::PalettedPermutations {
                textures: nullable(obj, "textures", |v| parse_set(v, gson::key))?,
                separator: opt(obj, "separator", gson::string)?.unwrap_or(Some("_".into())),
                palette_key: opt(obj, "palette_key", gson::key)?,
                permutations: nullable(obj, "permutations", parse_permutations)?,
            },
            _ => Source::Other(ty),
        };
        Ok(source)
    }

    /// The prefix as Java concatenates it.
    pub(crate) fn java_str(s: &Option<String>) -> &str {
        s.as_deref().unwrap_or("null")
    }
}

impl Region {
    fn parse(v: &Value) -> GResult<Self> {
        let obj = gson::object(v)?;
        Ok(Region {
            sprite: opt(obj, "sprite", gson::key)?,
            x: primitive_double(obj, "x")?,
            y: primitive_double(obj, "y")?,
            width: primitive_double(obj, "width")?,
            height: primitive_double(obj, "height")?,
        })
    }
}

/// A member read with `f`; `None` when absent.
fn opt<T>(obj: &Map<String, Value>, name: &str, f: impl Fn(&Value) -> GResult<T>) -> GResult<Option<T>> {
    obj.get(name).map(f).transpose()
}

/// A null-safe (reflective) member: absent or null are both `None`.
fn nullable<T>(obj: &Map<String, Value>, name: &str, f: impl Fn(&Value) -> GResult<T>) -> GResult<Option<T>> {
    obj.get(name).map(|v| nullable_value(v, &f)).transpose().map(Option::flatten)
}

fn nullable_value<T>(v: &Value, f: impl Fn(&Value) -> GResult<T>) -> GResult<Option<T>> {
    if v.is_null() { Ok(None) } else { f(v).map(Some) }
}

/// A primitive `double` field: absent or null keep 0.
fn primitive_double(obj: &Map<String, Value>, name: &str) -> GResult<f64> {
    Ok(nullable(obj, name, gson::double)?.unwrap_or(0.0))
}

fn parse_set<T: PartialEq>(v: &Value, f: impl Fn(&Value) -> GResult<T>) -> GResult<Vec<T>> {
    let mut out: Vec<T> = Vec::new();
    for item in v.as_array().ok_or(GsonError("expected an array"))? {
        let item = f(item)?;
        if !out.contains(&item) {
            out.push(item);
        }
    }
    Ok(out)
}

fn parse_permutations(v: &Value) -> GResult<Vec<(String, ResourcePath)>> {
    gson::object(v)?.iter().map(|(k, v)| Ok((k.clone(), gson::key(v)?))).collect()
}
