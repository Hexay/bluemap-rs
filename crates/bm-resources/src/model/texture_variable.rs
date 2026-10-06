use std::collections::HashMap;

use serde_json::Value;

use super::ModelError;
use super::gson::string_of;
use crate::ResourcePath;

/// `TextureVariable`: a `#name` reference into the owning model's texture map, or a texture path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextureVariable {
    Reference(String),
    Path(ResourcePath),
    /// An explicit JSON `null` (the adapter is null-safe): occupies its key but never resolves.
    Null,
}

impl TextureVariable {
    /// `TextureVariable.Adapter.read`: a string, or `{"sprite": ...}` with other keys skipped.
    pub fn from_value(v: &Value) -> Result<Self, ModelError> {
        match v {
            Value::Null => Ok(Self::Null),
            Value::String(s) => Self::parse(s),
            Value::Object(obj) => {
                let mut result = None;
                for (key, value) in obj {
                    if key == "sprite" {
                        result = Some(Self::parse(&string_of(value, "sprite")?)?);
                    }
                }
                result.ok_or(ModelError::NoSprite)
            }
            _ => Err(ModelError::Type { field: "texture", expected: "a string or an object" }),
        }
    }

    /// `Adapter.fromString`. Quirk: a value with neither `:` nor `/` is a reference even without `#`.
    pub fn parse(s: &str) -> Result<Self, ModelError> {
        if s.is_empty() {
            return Err(ModelError::EmptyTexture);
        }
        if let Some(name) = s.strip_prefix('#') {
            return Ok(Self::Reference(name.to_owned()));
        }
        if !s.contains(':') && !s.contains('/') {
            return Ok(Self::Reference(s.to_owned()));
        }
        Ok(Self::Path(ResourcePath::parse(s)))
    }

    /// `getTexturePath` against a model's merged texture map. A chain that ends on a missing key, a `null` or a
    /// loop resolves to `None`, which is what Java's cycle-guarded, caching resolution produces as well.
    pub fn resolve<'a>(&'a self, textures: &'a HashMap<String, TextureVariable>) -> Option<&'a ResourcePath> {
        let mut current = self;
        let mut lookups = 0;
        loop {
            match current {
                Self::Path(path) => return Some(path),
                Self::Null => return None,
                Self::Reference(name) => {
                    // more lookups than entries means one entry was visited twice
                    if lookups >= textures.len() {
                        return None;
                    }
                    current = textures.get(name)?;
                    lookups += 1;
                }
            }
        }
    }
}
