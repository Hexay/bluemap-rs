//! Java object shapes as `StateDumper` writes them: `#identity` (`class@hash`), collections as `{size, entries}`
//! capped at 30 entries, maps as `entries: [{key, value}]`, already-dumped objects as `"<<identity>>"`.

use serde_json::{Map, Value, json};

const MAX_ENTRIES: usize = 30;

/// Hands out identity strings; the hash stands in for `System.identityHashCode` (distinct within one dump).
pub struct Ids(u32);

impl Default for Ids {
    fn default() -> Self {
        Self(0x1b6d_3586)
    }
}

impl Ids {
    pub fn identity(&mut self, class: &str) -> String {
        self.0 = self.0.wrapping_mul(0x9E37_79B9).wrapping_add(0x7F4A_7C15) >> 1;
        format!("{class}@{:x}", self.0)
    }

    pub fn object(&mut self, class: &str, fields: Vec<(&str, Value)>) -> Value {
        let mut map = Map::new();
        map.insert("#identity".into(), self.identity(class).into());
        map.extend(fields.into_iter().map(|(k, v)| (k.to_owned(), v)));
        Value::Object(map)
    }

    pub fn list(&mut self, class: &str, entries: Vec<Value>) -> Value {
        let size = entries.len();
        let fields = vec![("size", json!(size)), ("entries", capped(entries, size))];
        self.object(class, fields)
    }

    pub fn map(&mut self, class: &str, entries: Vec<(Value, Value)>) -> Value {
        let size = entries.len();
        let entries = entries.into_iter().map(|(key, value)| json!({"key": key, "value": value})).collect();
        let fields = vec![("size", json!(size)), ("entries", capped(entries, size))];
        self.object(class, fields)
    }
}

fn capped(mut entries: Vec<Value>, size: usize) -> Value {
    if size > MAX_ENTRIES {
        entries.truncate(MAX_ENTRIES);
        entries.push(format!("<<{} more elements>>", size - MAX_ENTRIES).into());
    }
    Value::Array(entries)
}

/// A reference to an object dumped elsewhere.
pub fn seen(identity: &str) -> Value {
    format!("<<{identity}>>").into()
}

/// `Registry.REGISTRIES` in upstream's order: value class, keys in `ConcurrentHashMap` order, extra fields per key.
#[allow(clippy::type_complexity)]
const REGISTRIES: &[(&str, &[(&str, &[(&str, &str)])])] = &[
    (
        "core.map.hires.block.BlockRendererType$Impl",
        &[("bluemap:missing", &[]), ("bluemap:liquid", &[]), ("bluemap:default", &[])],
    ),
    (
        "core.world.mca.blockentity.BlockEntityType$Impl",
        &[
            ("minecraft:skull", &[]),
            ("minecraft:sign", &[]),
            ("minecraft:banner", &[]),
            ("minecraft:hanging_sign", &[]),
        ],
    ),
    (
        "core.world.biome.GrassColorModifier$Impl",
        &[("minecraft:none", &[]), ("minecraft:dark_forest", &[]), ("minecraft:swamp", &[])],
    ),
    (
        "common.config.ConfigLoader$Impl",
        &[("bluemap:json", &[("fileSuffix", ".json")]), ("bluemap:hocon", &[("fileSuffix", ".conf")])],
    ),
    (
        "core.resources.pack.resourcepack.atlas.SourceType$Impl",
        &[
            ("minecraft:single", &[]),
            ("minecraft:filter", &[]),
            ("minecraft:unstitch", &[]),
            ("minecraft:paletted_permutations", &[]),
            ("minecraft:directory", &[]),
        ],
    ),
    ("core.map.hires.entity.EntityRendererType$Impl", &[("bluemap:missing", &[]), ("bluemap:default", &[])]),
    ("core.world.WorldLoaderType$Impl", &[("bluemap:anvil", &[])]),
    (
        "common.config.mask.MaskType$Impl",
        &[
            ("bluemap:ellipse", &[]),
            ("bluemap:blur", &[]),
            ("bluemap:box", &[]),
            ("bluemap:polygon", &[]),
            ("bluemap:circle", &[]),
        ],
    ),
    (
        "common.rendermanager.TileUpdateStrategy$Impl",
        &[("bluemap:force_none", &[]), ("bluemap:force_edge", &[]), ("bluemap:force_all", &[])],
    ),
    ("core.world.mca.region.RegionType$Impl", &[("bluemap:mca", &[])]),
    (
        "core.storage.compression.BufferedCompression",
        &[
            ("bluemap:zstd", &[("id", "zstd"), ("fileSuffix", ".zst")]),
            ("bluemap:lz4", &[("id", "lz4"), ("fileSuffix", ".lz4")]),
            ("bluemap:gzip", &[("id", "gzip"), ("fileSuffix", ".gz")]),
            ("bluemap:deflate", &[("id", "deflate"), ("fileSuffix", ".deflate")]),
            ("bluemap:none", &[("id", "none"), ("fileSuffix", "")]),
        ],
    ),
    ("common.config.storage.StorageType$Impl", &[("bluemap:file", &[]), ("bluemap:sql", &[])]),
    (
        "core.map.renderstate.TileState$Impl",
        &[
            ("bluemap:render-error", &[]),
            ("bluemap:unknown", &[]),
            ("bluemap:chunk-error", &[]),
            ("bluemap:rendered", &[]),
            ("bluemap:rendered-edge", &[]),
            ("bluemap:out-of-bounds", &[]),
            ("bluemap:not-generated", &[]),
            ("bluemap:missing-light", &[]),
            ("bluemap:low-inhabited-time", &[]),
        ],
    ),
    ("core.map.hires.RenderPassType$Impl", &[("bluemap:entities", &[]), ("bluemap:blocks", &[])]),
    (
        "core.map.hires.block.color.BlockColorCalculatorType$Impl",
        &[
            ("minecraft:water", &[]),
            ("minecraft:grass", &[]),
            ("minecraft:redstone", &[]),
            ("minecraft:foliage", &[]),
            ("minecraft:dry_foliage", &[]),
        ],
    ),
];

/// The `registries` array: every `Registry` with its entries, key set and value collection.
pub fn registries(ids: &mut Ids) -> Vec<Value> {
    REGISTRIES
        .iter()
        .map(|&(class, entries)| {
            let registry = ids.identity("de.bluecolored.bluemap.core.util.Registry");
            let mut values = Vec::new();
            let pairs: Vec<(Value, Value)> = entries
                .iter()
                .map(|&(key, extra)| {
                    // the one registry whose values differ in class
                    let impl_class = if key == "bluemap:none" && class.ends_with("BufferedCompression") {
                        "de.bluecolored.bluemap.core.storage.compression.NoCompression".to_owned()
                    } else {
                        format!("de.bluecolored.bluemap.{class}")
                    };
                    let mut fields = Vec::new();
                    if class.ends_with("TileState$Impl") {
                        fields.push(("#toString", json!(key)));
                    }
                    fields.push(("key", json!(key)));
                    fields.extend(extra.iter().map(|&(k, v)| (k, json!(v))));
                    let value = ids.object(&impl_class, fields);
                    values.push(seen(value["#identity"].as_str().unwrap_or_default()));
                    (json!(key), value)
                })
                .collect();
            let keys = entries.iter().map(|(k, _)| json!(k)).collect();
            let mut map = Map::new();
            map.insert("#identity".into(), registry.into());
            map.insert("entries".into(), ids.map("java.util.concurrent.ConcurrentHashMap", pairs));
            map.insert("keys".into(), ids.list("java.util.Collections$UnmodifiableSet", keys));
            map.insert("values".into(), ids.list("java.util.Collections$UnmodifiableCollection", values));
            Value::Object(map)
        })
        .collect()
}
