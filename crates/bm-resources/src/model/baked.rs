use std::collections::HashMap;

use bm_math::MatrixM4f;

use super::{Direction, Element, Face, Model};
use crate::ResourcePath;

/// `ResourceModelRenderer.BLOCK_SCALE`.
const BLOCK_SCALE: f32 = 1.0 / 16.0;

/// Every model of the library after the parent merge and texture resolution, as the renderer reads them.
#[derive(Debug, Default)]
pub struct BakedModels {
    pub models: HashMap<ResourcePath, BakedModel>,
    /// `(model, parent)` pairs whose parent isn't loaded (`builtin/generated` and friends); upstream ignores them.
    pub missing_parents: Vec<(ResourcePath, ResourcePath)>,
    /// `(model, reference)` per face whose `#reference` resolved to nothing.
    pub unresolved_references: Vec<(ResourcePath, String)>,
}

impl BakedModels {
    pub fn get(&self, key: &ResourcePath) -> Option<&BakedModel> {
        self.models.get(key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BakedModel {
    pub elements: Vec<BakedElement>,
    pub ambient_occlusion: bool,
    /// `Model.culling`: the first full-cube element has six fully opaque faces.
    pub culling: bool,
    /// `Model.occluding`: some element is a full cube.
    pub occluding: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BakedElement {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// The element rotation alone (the renderer rotates face normals by it).
    pub rotation: MatrixM4f,
    /// `rotation · scale(1/16)`: element space (0..16) to block space (0..1).
    pub transform: MatrixM4f,
    pub shade: bool,
    pub light_emission: i32,
    /// Indexed by [`Direction::index`].
    pub faces: [Option<BakedFace>; 6],
}

#[derive(Clone, Debug, PartialEq)]
pub struct BakedFace {
    pub uv: [f32; 4],
    /// `None` when a reference didn't resolve (the texture gallery maps that to id 0).
    pub texture: Option<ResourcePath>,
    pub cullface: Option<Direction>,
    pub rotation: i32,
    /// `tintindex >= 0`; the index itself is ignored by the block renderer.
    pub tinted: bool,
}

impl BakedElement {
    pub fn face(&self, dir: Direction) -> Option<&BakedFace> {
        self.faces[dir.index()].as_ref()
    }
}

impl BakedFace {
    /// Quarter turns the UVs shift by: `floorDiv(rotation, 90) % 4`, made non-negative.
    pub fn uv_rotation_steps(&self) -> usize {
        self.rotation.div_euclid(90).rem_euclid(4) as usize
    }
}

/// Bakes one merged model. `texture_alpha` gives a loaded texture's straight average alpha; `unresolved` collects
/// the names of references that resolved to nothing.
pub(super) fn bake_model(
    model: &Model,
    texture_alpha: &dyn Fn(&ResourcePath) -> Option<f32>,
    unresolved: &mut Vec<String>,
) -> BakedModel {
    let elements = model.elements.as_deref().unwrap_or_default();
    let (occluding, culling) = calculate_properties(model, elements, texture_alpha);
    BakedModel {
        elements: elements.iter().map(|e| bake_element(model, e, unresolved)).collect(),
        ambient_occlusion: model.ambient_occlusion(),
        culling,
        occluding,
    }
}

fn bake_element(model: &Model, element: &Element, unresolved: &mut Vec<String>) -> BakedElement {
    let rotation = *element.rotation.matrix();
    let mut transform = rotation;
    transform.scale(BLOCK_SCALE, BLOCK_SCALE, BLOCK_SCALE);
    BakedElement {
        from: element.from,
        to: element.to,
        rotation,
        transform,
        shade: element.shade,
        light_emission: element.light_emission,
        faces: element.faces.each_ref().map(|f| f.as_ref().map(|f| bake_face(model, f, unresolved))),
    }
}

fn bake_face(model: &Model, face: &Face, unresolved: &mut Vec<String>) -> BakedFace {
    let texture = face.texture.resolve(&model.textures).cloned();
    if texture.is_none()
        && let super::TextureVariable::Reference(name) = &face.texture
    {
        unresolved.push(name.clone());
    }
    BakedFace { uv: face.uv, texture, cullface: face.cullface, rotation: face.rotation, tinted: face.tintindex >= 0 }
}

/// `calculateProperties`: only the first full-cube element counts. Returns `(occluding, culling)`.
fn calculate_properties(
    model: &Model,
    elements: &[Element],
    texture_alpha: &dyn Fn(&ResourcePath) -> Option<f32>,
) -> (bool, bool) {
    let Some(cube) = elements.iter().find(|e| e.is_full_cube()) else { return (false, false) };
    let culling = cube.faces.iter().all(|face| {
        let path = face.as_ref().and_then(|f| f.texture.resolve(&model.textures));
        // `a < 1` rejects, so a NaN alpha still culls
        path.and_then(texture_alpha).is_some_and(|a| a >= 1.0 || a.is_nan())
    });
    (true, culling)
}
