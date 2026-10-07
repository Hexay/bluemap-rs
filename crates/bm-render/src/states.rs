//! Everything the mesher needs per block state, resolved once per [`StateId`] into a dense table: properties,
//! the matching variant sets with their models, and per-face material ids. Only weighted picks stay per position.

use std::sync::Arc;

use bm_math::Color;
use bm_resources::ResourcePath;
use bm_resources::blockstate::{RendererType, Variant, VariantSet, hash_to_float};
use bm_resources::color::BlockProperties;
use bm_resources::model::BakedModel;
use bm_resources::resource_pack::ResourcePack;
use bm_resources::texture::TextureGallery;
use bm_world::{BlockState, BlockStates, StateId};

use crate::flags::{Flags, Hidden};
use crate::relative::{Offset, RelativeOffsets};

/// Resolved `BlockProperties` (undefined reads as false).
#[derive(Clone, Copy, Debug, Default)]
pub struct Props {
    pub culling: bool,
    pub occluding: bool,
    pub always_waterlogged: bool,
    pub random_offset: bool,
    pub culling_identical: bool,
}

impl From<BlockProperties> for Props {
    fn from(p: BlockProperties) -> Self {
        Self {
            culling: p.is_culling(),
            occluding: p.is_occluding(),
            always_waterlogged: p.is_always_waterlogged(),
            random_offset: p.is_random_offset(),
            culling_identical: p.is_culling_identical(),
        }
    }
}

/// A texture as a face uses it: gallery id, and the premultiplied average colour for the lowres map colour
/// (`None` when the path is unresolved or the texture isn't loaded).
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub id: u32,
    pub color: Option<Color>,
}

pub struct VariantInfo<'a> {
    pub variant: &'a Variant,
    pub renderer: RendererType,
    pub model: Option<&'a BakedModel>,
    /// Per element, indexed by `Direction::index`.
    pub materials: Vec<[Material; 6]>,
    /// The liquid renderer's `still` and `flow` textures.
    pub still: Material,
    pub flow: Material,
    pub relative: RelativeOffsets,
}

pub struct SetInfo<'a> {
    pub variants: Vec<VariantInfo<'a>>,
    /// The variants' weights, contiguous for the pick loop.
    weights: Box<[f64]>,
    total_weight: f64,
    /// One variant the pick always takes: a finite non-negative weight wins the loop's first step.
    only: bool,
}

impl SetInfo<'_> {
    /// `VariantSet.pick` as an index.
    pub fn pick(&self, x: i32, y: i32, z: i32) -> Option<&VariantInfo<'_>> {
        if self.only {
            return self.variants.first();
        }
        let mut selection = f64::from(hash_to_float(x, y, z)) * self.total_weight;
        let i = self.weights.iter().position(|w| {
            selection -= w;
            selection <= 0.0
        })?;
        Some(&self.variants[i])
    }
}

pub struct StateInfo<'a> {
    pub state: Arc<BlockState>,
    pub props: Props,
    /// `BlockState.getLiquidLevel`.
    pub liquid_level: i32,
    /// The matching sets in `forEach` order; empty when there is no blockstate file.
    pub sets: Vec<SetInfo<'a>>,
    /// Copies of the `state` bits read per block, so they don't chase the `Arc`.
    air: bool,
    water: bool,
    renders_water: bool,
    pub(crate) hidden: Hidden,
}

impl StateInfo<'_> {
    pub fn is_air(&self) -> bool {
        self.air
    }

    pub fn is_water(&self) -> bool {
        self.water
    }

    /// Rendered with an extra water model.
    pub fn renders_water(&self) -> bool {
        self.renders_water
    }
}

/// Dense per-state table. Build after the chunks to render are loaded: chunk decoding interns new states.
pub struct StateCache<'a> {
    pack: &'a ResourcePack,
    gallery: &'a TextureGallery,
    infos: Vec<StateInfo<'a>>,
    flags: Vec<Flags>,
    /// `BlockState.WATER`: `minecraft:water` without properties.
    pub water: StateId,
}

impl<'a> StateCache<'a> {
    pub fn new(pack: &'a ResourcePack, gallery: &'a TextureGallery, registry: &BlockStates) -> Self {
        let water = registry.intern("minecraft:water", &mut []);
        let mut cache = Self { pack, gallery, infos: Vec::new(), flags: Vec::new(), water };
        cache.update(registry);
        cache
    }

    /// Resolves every state interned since the last call.
    pub fn update(&mut self, registry: &BlockStates) {
        for id in self.infos.len()..registry.len() {
            let info = self.resolve(registry.get(StateId(id as u32)));
            let p = info.props;
            let watery = info.is_water() || info.renders_water();
            self.flags.push(Flags::new(info.is_air(), p.culling, p.culling_identical, p.occluding, watery));
            self.infos.push(info);
        }
    }

    pub fn len(&self) -> usize {
        self.infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }

    pub fn get(&self, id: StateId) -> &StateInfo<'a> {
        &self.infos[id.0 as usize]
    }

    pub(crate) fn flags(&self, id: StateId) -> Flags {
        self.flags[id.0 as usize]
    }

    /// Whether `neighbor` culls a face of `own` it covers.
    pub(crate) fn culls(&self, neighbor: StateId, own: StateId) -> bool {
        let flags = self.flags(neighbor);
        flags.culling() || (flags.culling_identical() && neighbor == own)
    }

    fn resolve(&self, state: Arc<BlockState>) -> StateInfo<'a> {
        let pack = self.pack;
        let mut sets = Vec::new();
        if let Some(def) = pack.blockstate(&state) {
            // `BlockStateDef::resolve`, unrolled so the sets borrow the pack only
            sets.extend(def.variants.as_ref().and_then(|v| v.resolve(&state)).map(|s| self.set_info(s)));
            let parts = def.multipart.iter().flat_map(|m| &m.parts);
            sets.extend(parts.filter(|p| p.condition.matches(&state)).map(|s| self.set_info(s)));
        }
        let props: Props = pack.block_properties(&state).into();
        let renders_water = state.waterlogged || props.always_waterlogged;
        StateInfo {
            air: state.is_air,
            water: state.is_water,
            renders_water,
            hidden: if renders_water { Hidden::Never } else { hidden(&sets, state.is_water) },
            props,
            liquid_level: liquid_level(&state),
            sets,
            state,
        }
    }

    fn set_info(&self, set: &'a VariantSet) -> SetInfo<'a> {
        let weights: Box<[f64]> = set.variants.iter().map(|v| v.weight).collect();
        SetInfo {
            variants: set.variants.iter().map(|v| self.variant_info(v)).collect(),
            only: matches!(*weights, [w] if w >= 0.0 && w.is_finite()),
            weights,
            total_weight: set.total_weight(),
        }
    }

    fn variant_info(&self, variant: &'a Variant) -> VariantInfo<'a> {
        let model = self.pack.model(&variant.model);
        let materials = model.map_or_else(Vec::new, |m| {
            m.elements
                .iter()
                .map(|e| e.faces.each_ref().map(|f| self.material(f.as_ref().and_then(|f| f.texture.as_ref()))))
                .collect()
        });
        let named = |name: &str| self.material(model.and_then(|m| m.textures.get(name)));
        VariantInfo {
            variant,
            // `bluemap:missing` falls back to the default renderer: core registers no fallback renderer types
            renderer: match RendererType::from_key(&variant.renderer) {
                Some(RendererType::Liquid) => RendererType::Liquid,
                _ => RendererType::Default,
            },
            model,
            materials,
            still: named("still"),
            flow: named("flow"),
            relative: RelativeOffsets::new(variant),
        }
    }

    fn material(&self, path: Option<&ResourcePath>) -> Material {
        Material {
            id: self.gallery.get(path),
            color: path.and_then(|p| self.pack.textures.get(p)).map(|t| t.color_premultiplied()),
        }
    }
}

/// [`Hidden`] for a state not rendered with extra water. A face without an in-range cullface, or a liquid variant
/// mixed with others or of another liquid, could render whatever the neighbours are.
fn hidden(sets: &[SetInfo], is_water: bool) -> Hidden {
    let mut variants = sets.iter().flat_map(|s| &s.variants).peekable();
    let liquid = |v: &VariantInfo| v.renderer == RendererType::Liquid;
    if is_water && variants.peek().is_some() && variants.clone().all(liquid) {
        return Hidden::Water;
    }
    let mut slots = 0u32;
    for v in variants {
        if liquid(v) {
            return Hidden::Never;
        }
        for face in v.model.iter().flat_map(|m| &m.elements).flat_map(|e| e.faces.iter().flatten()) {
            let Some(cull) = face.cullface else { return Hidden::Never };
            let slot = v.relative.get(cull.to_vector()).slot;
            if slot == Offset::FAR {
                return Hidden::Never;
            }
            slots |= 1 << slot;
        }
    }
    Hidden::Cullfaces(slots)
}

/// `level` clamped to 0..=15; absent or unparseable is 0.
fn liquid_level(state: &BlockState) -> i32 {
    state.property("level").and_then(|l| l.parse::<i32>().ok()).map_or(0, |l| l.clamp(0, 15))
}
