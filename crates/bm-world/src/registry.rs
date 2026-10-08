//! Process-wide interning of block states and biomes to dense ids, so chunks hold `u32`s and everything per state
//! (properties, baked models) lives in id-indexed tables. BlueMap allocates a `BlockState` per palette entry per
//! section instead (docs/01 §3).

use std::sync::{Arc, RwLock};

use rustc_hash::FxHashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BiomeId(pub u16);

impl StateId {
    pub const AIR: Self = Self(0);
    /// `bluemap:missing`: an unparseable or out-of-range palette entry.
    pub const MISSING: Self = Self(1);
}

#[derive(Debug)]
pub struct BlockState {
    /// Namespaced, e.g. `minecraft:oak_stairs`.
    pub name: Box<str>,
    /// Sorted by key.
    pub properties: Box<[(Box<str>, Box<str>)]>,
    /// BlueMap's `BlockState.toString()`: `minecraft:oak_stairs[facing=east,half=bottom]`, `[]` when empty.
    pub key: Box<str>,
    pub is_air: bool,
    pub is_water: bool,
    pub waterlogged: bool,
}

impl BlockState {
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties.iter().find(|(k, _)| &**k == key).map(|(_, v)| &**v)
    }
}

#[derive(Clone, Default)]
struct StatesInner {
    by_key: FxHashMap<Box<str>, StateId>,
    states: Vec<Arc<BlockState>>,
    /// Block name → its default state, for palettes that name a block without properties (26.3+).
    defaults: FxHashMap<Box<str>, StateId>,
}

pub struct BlockStates {
    inner: RwLock<StatesInner>,
}

impl Default for BlockStates {
    fn default() -> Self {
        let registry = Self { inner: RwLock::default() };
        assert_eq!(registry.intern("minecraft:air", &mut []), StateId::AIR);
        assert_eq!(registry.intern("bluemap:missing", &mut []), StateId::MISSING);
        registry
    }
}

impl BlockStates {
    /// A copy of the registry as it is now; ids stay the same, later interning goes to `self` only.
    pub fn snapshot(&self) -> Self {
        Self { inner: RwLock::new(self.inner.read().unwrap().clone()) }
    }

    /// Id of `name` with `properties` (any order; sorted in place). A name without namespace gets `minecraft:`.
    pub fn intern(&self, name: &str, properties: &mut [(&str, &str)]) -> StateId {
        properties.sort_unstable_by_key(|&(k, _)| k);
        let key = state_key(name, properties);
        if let Some(&id) = self.inner.read().unwrap().by_key.get(key.as_str()) {
            return id;
        }
        let mut inner = self.inner.write().unwrap();
        if let Some(&id) = inner.by_key.get(key.as_str()) {
            return id;
        }
        let id = StateId(inner.states.len() as u32);
        let name = namespaced(name);
        inner.states.push(Arc::new(BlockState {
            is_air: matches!(&*name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"),
            is_water: &*name == "minecraft:water",
            waterlogged: properties.contains(&("waterlogged", "true")),
            properties: properties.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
            name: name.into(),
            key: key.as_str().into(),
        }));
        inner.by_key.insert(key.into(), id);
        id
    }

    /// Parses BlueMap's state string `ns:name[k=v,…]` (brackets optional).
    pub fn intern_str(&self, state: &str) -> Option<StateId> {
        let state = state.trim();
        let (name, props) = match state.find('[') {
            Some(i) if state.ends_with(']') => (&state[..i], &state[i + 1..state.len() - 1]),
            _ => (state, ""),
        };
        let mut properties = Vec::new();
        for kv in props.split(',').filter(|s| !s.trim().is_empty()) {
            properties.push(kv.trim().split_once('=')?);
        }
        Some(self.intern(name.trim(), &mut properties))
    }

    /// The default state of block `name`, or the state without properties when no default is known (as BlueMap).
    pub fn default_state(&self, name: &str) -> StateId {
        let name = namespaced(name);
        if let Some(&id) = self.inner.read().unwrap().defaults.get(&*name) {
            return id;
        }
        self.intern(&name, &mut [])
    }

    /// Registers `state` as its block's default unless one is already set: higher-priority packs load first.
    pub fn add_default(&self, state: StateId) {
        let name = self.get(state).name.clone();
        self.inner.write().unwrap().defaults.entry(name).or_insert(state);
    }

    /// Loads a `defaultBlockstates.json` (`{"minecraft:oak_door": "minecraft:oak_door[facing=north,…]", …}`).
    /// Entries that don't parse are skipped, as BlueMap does.
    pub fn add_defaults_json(&self, json: &str) -> Result<usize, serde_json::Error> {
        let map: FxHashMap<String, String> = serde_json::from_str(json)?;
        let ids: Vec<StateId> = map.values().filter_map(|s| self.intern_str(s)).collect();
        ids.iter().for_each(|&id| self.add_default(id));
        Ok(ids.len())
    }

    pub fn get(&self, id: StateId) -> Arc<BlockState> {
        self.inner.read().unwrap().states[id.0 as usize].clone()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct Biomes {
    inner: RwLock<BiomesInner>,
}

#[derive(Default)]
struct BiomesInner {
    by_name: FxHashMap<Box<str>, BiomeId>,
    names: Vec<Box<str>>,
}

impl BiomeId {
    /// BlueMap's `Biome.DEFAULT` (`bluemap:default`): empty or out-of-range biome palettes.
    pub const DEFAULT: Self = Self(0);
}

impl Default for Biomes {
    fn default() -> Self {
        let registry = Self { inner: RwLock::default() };
        assert_eq!(registry.intern("bluemap:default"), BiomeId::DEFAULT);
        registry
    }
}

impl Biomes {
    pub fn intern(&self, name: &str) -> BiomeId {
        let name = namespaced(name);
        if let Some(&id) = self.inner.read().unwrap().by_name.get(&*name) {
            return id;
        }
        let mut inner = self.inner.write().unwrap();
        if let Some(&id) = inner.by_name.get(&*name) {
            return id;
        }
        let id = BiomeId(u16::try_from(inner.names.len()).expect("more than 65536 biomes"));
        inner.names.push(name.clone().into());
        inner.by_name.insert(name.into(), id);
        id
    }

    pub fn name(&self, id: BiomeId) -> Box<str> {
        self.inner.read().unwrap().names[id.0 as usize].clone()
    }
}

/// `stone` → `minecraft:stone`, like BlueMap's `Key.parse`.
fn namespaced(name: &str) -> std::borrow::Cow<'_, str> {
    match name.find(':') {
        Some(i) if i > 0 => name.into(),
        _ => format!("minecraft:{name}").into(),
    }
}

fn state_key(name: &str, sorted: &[(&str, &str)]) -> String {
    let mut key = String::with_capacity(64);
    key.push_str(&namespaced(name));
    key.push('[');
    for (i, (k, v)) in sorted.iter().enumerate() {
        if i > 0 {
            key.push(',');
        }
        key.push_str(k);
        key.push('=');
        key.push_str(v);
    }
    key.push(']');
    key
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
