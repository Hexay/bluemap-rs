use std::collections::HashMap;

use serde_json::Value;

use super::gson::{self, non_null, opt_bool, string_of};
use super::{Element, ModelError, TextureVariable};
use crate::ResourcePath;

/// A model as read from JSON (`Model.java`), before or after [`ModelLibrary`](super::ModelLibrary) merged its
/// parents into it. `display` and `gui_light` are ignored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub parent: Option<ResourcePath>,
    pub textures: HashMap<String, TextureVariable>,
    /// `None` when absent, so a parent's elements get copied in; `Some(empty)` keeps them out.
    pub elements: Option<Vec<Element>>,
    pub ambientocclusion: Option<bool>,
}

impl Model {
    /// Parses a model file; `Ok(None)` for an empty file or a top-level `null` (Gson returns null, the pool skips it).
    pub fn parse(src: &str) -> Result<Option<Self>, ModelError> {
        if src.trim_start_matches('\u{feff}').trim().is_empty() {
            return Ok(None);
        }
        match crate::json::parse(src)? {
            Value::Null => Ok(None),
            v => Self::from_value(&v).map(Some),
        }
    }

    pub fn from_value(v: &Value) -> Result<Self, ModelError> {
        let obj = gson::object(v, "model")?;
        let parent = non_null(obj, "parent").map(|p| string_of(p, "parent")).transpose()?;
        let mut textures = HashMap::new();
        if let Some(map) = non_null(obj, "textures") {
            for (key, value) in gson::object(map, "textures")? {
                textures.insert(key.clone(), TextureVariable::from_value(value)?);
            }
        }
        let elements = match non_null(obj, "elements") {
            None => None,
            Some(Value::Array(items)) => Some(
                // null slots are skipped by every consumer upstream
                items.iter().filter(|e| !e.is_null()).map(Element::from_value).collect::<Result<_, _>>()?,
            ),
            Some(_) => return Err(ModelError::Type { field: "elements", expected: "an array" }),
        };
        Ok(Self {
            parent: parent.map(|p| ResourcePath::parse(&p)),
            textures,
            elements,
            ambientocclusion: opt_bool(obj, "ambientocclusion")?,
        })
    }

    /// `isAmbientocclusion`: true unless set (here or by a parent).
    pub fn ambient_occlusion(&self) -> bool {
        self.ambientocclusion.unwrap_or(true)
    }

    /// The merge half of `applyParent`, with `parent` already merged with its own ancestors.
    pub(super) fn inherit(&mut self, parent: &Model) {
        if self.ambientocclusion.is_none() {
            self.ambientocclusion = parent.ambientocclusion;
        }
        for (key, value) in &parent.textures {
            self.textures.entry(key.clone()).or_insert_with(|| value.clone());
        }
        if self.elements.is_none() {
            self.elements.clone_from(&parent.elements);
        }
    }
}
