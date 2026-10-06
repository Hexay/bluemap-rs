use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

use super::baked::bake_model;
use super::{BakedModels, Model, ModelError, TextureVariable, missing_texture};
use crate::{Pack, ResourcePath};

/// The model pool (`ResourcePack.models`): the first pack to provide a key wins.
#[derive(Debug, Default)]
pub struct ModelLibrary {
    models: HashMap<ResourcePath, Model>,
    failures: Vec<(ResourcePath, ModelError)>,
}

impl ModelLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `assets/*/models/**.json` of one pack. Keys already present are skipped unread; a file that fails to
    /// parse is recorded in [`ModelLibrary::failures`] and leaves the key free for later packs, as upstream.
    pub fn load_pack(&mut self, pack: &Pack) {
        let files: Vec<(ResourcePath, String)> = pack
            .list("assets")
            .iter()
            .flat_map(|ns| pack.walk(&format!("assets/{ns}/models")))
            .filter(|path| path.ends_with(".json"))
            .filter_map(|path| Some((ResourcePath::from_file(&path, 1, 3)?, path)))
            .filter(|(key, _)| !self.models.contains_key(key))
            .collect();
        let parsed: Vec<_> = files
            .into_par_iter()
            .map(|(key, path)| {
                let model = pack.read(&path).map_or(Ok(None), |bytes| match String::from_utf8(bytes) {
                    Ok(src) => Model::parse(&src),
                    Err(_) => Err(ModelError::Utf8),
                });
                (key, model)
            })
            .collect();
        for (key, model) in parsed {
            match model {
                Ok(Some(model)) => {
                    self.insert(key, model);
                }
                Ok(None) => {}
                Err(e) => self.failures.push((key, e)),
            }
        }
    }

    /// Adds a model unless the key is taken; returns whether it was added.
    pub fn insert(&mut self, key: ResourcePath, model: Model) -> bool {
        match self.models.entry(key) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(model);
                true
            }
        }
    }

    pub fn get(&self, key: &ResourcePath) -> Option<&Model> {
        self.models.get(key)
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Files that failed to parse, in load order.
    pub fn failures(&self) -> &[(ResourcePath, ModelError)] {
        &self.failures
    }

    /// `collectUsedTextureKeys`: every direct path in any model's texture map (item models included), plus
    /// `bluemap:block/missing`. Quirk: paths written directly on a face are not collected.
    pub fn collect_used_texture_keys(&self) -> HashSet<ResourcePath> {
        let mut keys: HashSet<ResourcePath> = self
            .models
            .values()
            .flat_map(|m| m.textures.values())
            .filter_map(|t| match t {
                TextureVariable::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        keys.insert(missing_texture());
        keys
    }

    /// `applyParent` on every model; returns the `(model, parent)` pairs whose parent isn't loaded.
    pub fn merge_parents(&mut self) -> Vec<(ResourcePath, ResourcePath)> {
        let mut keys: Vec<ResourcePath> = self.models.keys().cloned().collect();
        // Java walks HashMap order, which only matters for parent cycles; sorted keeps it deterministic
        keys.sort_unstable();
        let mut missing = Vec::new();
        for key in &keys {
            self.apply_parent(key, &mut missing);
        }
        missing
    }

    fn apply_parent(&mut self, key: &ResourcePath, missing: &mut Vec<(ResourcePath, ResourcePath)>) {
        // the parent link is cleared before recursing, which is what stops parent cycles
        let Some(parent_key) = self.models.get_mut(key).and_then(|m| m.parent.take()) else { return };
        if !self.models.contains_key(&parent_key) {
            missing.push((key.clone(), parent_key));
            return;
        }
        self.apply_parent(&parent_key, missing);
        let parent = self.models[&parent_key].clone();
        if let Some(model) = self.models.get_mut(key) {
            model.inherit(&parent);
        }
    }

    /// Merges parents, resolves every face texture and computes culling/occlusion. `texture_alpha` returns the
    /// straight average alpha of a loaded texture (`Texture.getColorStraight().a`), `None` if it isn't loaded.
    pub fn bake(mut self, texture_alpha: &(dyn Fn(&ResourcePath) -> Option<f32> + Sync)) -> BakedModels {
        let missing_parents = self.merge_parents();
        let baked: Vec<_> = self
            .models
            .par_iter()
            .map(|(key, model)| {
                let mut unresolved = Vec::new();
                let baked = bake_model(model, texture_alpha, &mut unresolved);
                (key.clone(), baked, unresolved)
            })
            .collect();
        let mut out = BakedModels { missing_parents, ..Default::default() };
        for (key, model, unresolved) in baked {
            out.unresolved_references.extend(unresolved.into_iter().map(|name| (key.clone(), name)));
            out.models.insert(key, model);
        }
        out
    }
}
