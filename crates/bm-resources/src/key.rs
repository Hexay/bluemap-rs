//! `namespace:path` resource keys (BlueMap's `Key` and `ResourcePath`).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer};

/// A namespaced key, cheap to clone. Equality is exact (case-sensitive), as in BlueMap.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourcePath(Arc<str>);

impl ResourcePath {
    /// `ResourcePath(String)`: **lowercased**, `minecraft` namespace when none (or a leading `:`). Every
    /// resource reference read from JSON goes through this.
    pub fn parse(s: &str) -> Self {
        Self::key(&s.to_lowercase())
    }

    /// `Key.parse`: like [`ResourcePath::parse`] but keeps case (block ids, registry keys).
    pub fn key(s: &str) -> Self {
        Self::key_in(s, "minecraft")
    }

    /// [`ResourcePath::key`] with a different default namespace (e.g. `bluemap` for renderer types).
    pub fn key_in(s: &str, default_namespace: &str) -> Self {
        match s.find(':') {
            Some(i) if i > 0 => Self(s.into()),
            _ => Self(format!("{default_namespace}:{s}").into()),
        }
    }

    /// Key of a pack file: `assets/minecraft/models/block/stone.json` with segments (1, 3) → `minecraft:block/stone`.
    /// Not lowercased, so a file with capitals can't be referenced from JSON (upstream behaviour).
    pub fn from_file(path: &str, namespace_segment: usize, value_segment: usize) -> Option<Self> {
        let segments: Vec<&str> = path.split('/').collect();
        let namespace = segments.get(namespace_segment)?;
        let value = segments.get(value_segment..).filter(|v| !v.is_empty())?.join("/");
        let value = value.rfind('.').map_or(value.as_str(), |dot| &value[..dot]);
        Some(Self(format!("{namespace}:{value}").into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        self.0.split_once(':').map_or("", |(ns, _)| ns)
    }

    pub fn path(&self) -> &str {
        self.0.split_once(':').map_or(&self.0, |(_, p)| p)
    }
}

impl fmt::Display for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &*self.0)
    }
}

/// Deserializes as a lowercased [`ResourcePath::parse`] reference.
impl<'de> Deserialize<'de> for ResourcePath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::parse(&String::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_namespaces() {
        assert_eq!(ResourcePath::parse("Block/Stone").as_str(), "minecraft:block/stone");
        assert_eq!(ResourcePath::parse(":x").as_str(), "minecraft::x");
        assert_eq!(ResourcePath::key("Mod:Thing").as_str(), "Mod:Thing");
        assert_eq!(ResourcePath::key_in("liquid", "bluemap").as_str(), "bluemap:liquid");
        let k = ResourcePath::parse("ns:a/b");
        assert_eq!((k.namespace(), k.path()), ("ns", "a/b"));
    }

    #[test]
    fn file_keys_keep_case_and_drop_the_extension() {
        let k = ResourcePath::from_file("assets/mymod/textures/block/Ore.v2.png", 1, 3).unwrap();
        assert_eq!(k.as_str(), "mymod:block/Ore.v2");
        let biome = ResourcePath::from_file("data/minecraft/worldgen/biome/plains.json", 1, 4).unwrap();
        assert_eq!(biome.as_str(), "minecraft:plains");
        assert_eq!(ResourcePath::from_file("assets/x", 1, 3), None);
    }
}
