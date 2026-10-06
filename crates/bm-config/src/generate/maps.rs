//! `maps/*.conf` generation: BlueMap's per-dimension presets and auto-config ids (`BlueMapConfigManager` 228-301).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use super::format_path;
use crate::key::Key;
use crate::template::ConfigTemplate;

const MAP: &str = include_str!("../../templates/bluemap/maps/map.conf");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimensionPreset {
    Overworld,
    Nether,
    End,
}

/// A world the server reports at first start.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerWorld {
    pub world_folder: PathBuf,
    pub dimension: Key,
    /// `None`: same as `dimension`.
    pub dimension_type: Option<Key>,
}

pub fn map_conf(
    preset: DimensionPreset,
    name: &str,
    world_folder: &Path,
    dimension: &Key,
    dimension_type: &Key,
    sorting: i32,
    cwd: &Path,
) -> String {
    let (sky, void, ambient, caves) = match preset {
        DimensionPreset::Overworld => ("#7dabff", "#000000", "0.1", "55"),
        DimensionPreset::Nether => ("#290000", "#150000", "0.6", "-10000"),
        DimensionPreset::End => ("#080010", "#080010", "0.6", "-10000"),
    };
    ConfigTemplate::new(MAP)
        .var("name", name)
        .var("sorting", &sorting.to_string())
        .var("world", &format_path(world_folder, cwd))
        .var("dimension", &dimension.formatted())
        .conditional("display-dimension-type", dimension != dimension_type)
        .var("dimension-type", &dimension_type.formatted())
        .var("sky-color", sky)
        .var("void-color", void)
        .var("ambient-light", ambient)
        .var("remove-caves-below-y", caves)
        .conditional("remove-nether-ceiling", preset == DimensionPreset::Nether)
        .build()
}

/// No server worlds (CLI): `overworld`, `nether` and `end` maps of `./world`. Returns `(map id, file content)`.
pub fn default_map_configs(cwd: &Path) -> Vec<(String, String)> {
    let world = Path::new("world");
    [
        ("overworld", "Overworld", DimensionPreset::Overworld, "overworld", 0),
        ("nether", "Nether", DimensionPreset::Nether, "the_nether", 100),
        ("end", "End", DimensionPreset::End, "the_end", 200),
    ]
    .into_iter()
    .map(|(id, name, preset, dim, sorting)| {
        let key = Key::minecraft(dim);
        (id.to_owned(), map_conf(preset, name, world, &key, &key, sorting, cwd))
    })
    .collect()
}

/// Java `replaceAll("\\W", "_")` (ASCII word characters only).
pub(crate) fn sanitise_map_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// One map per server world, overworlds first; ids from the world folder name, made unique with the dimension
/// and a counter. Returns `(map id, file content)` in generation order.
pub fn auto_map_configs(worlds: &[ServerWorld], cwd: &Path) -> Vec<(String, String)> {
    let overworld = Key::minecraft("overworld");
    let mut sorted: Vec<&ServerWorld> = worlds.iter().collect();
    sorted.sort_by_key(|w| w.dimension != overworld);

    let mut ids = HashSet::new();
    let mut out = Vec::new();
    for world in sorted {
        let folder = normalize(&world.world_folder);
        let folder_name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dimension = &world.dimension;
        let dimension_type = world.dimension_type.as_ref().unwrap_or(dimension);
        let dimension_name =
            if dimension.namespace() == Key::MINECRAFT { dimension.value().to_owned() } else { dimension.formatted() };

        let mut id = sanitise_map_id(&folder_name).to_lowercase();
        if ids.contains(&id) {
            id = sanitise_map_id(&format!("{folder_name}_{dimension_name}")).to_lowercase();
        }
        let mut i = 1;
        let mut unique = id.clone();
        while ids.contains(&unique) {
            i += 1;
            unique = format!("{id}_{i}");
        }
        ids.insert(unique.clone());

        let mut name = format!("{folder_name} ({dimension_name})");
        if i > 1 {
            name = format!("{name} ({i})");
        }
        let (preset, base) = match dimension.formatted().as_str() {
            "minecraft:overworld" => (DimensionPreset::Overworld, 0),
            "minecraft:the_nether" => (DimensionPreset::Nether, 100),
            "minecraft:the_end" => (DimensionPreset::End, 200),
            _ => (DimensionPreset::Overworld, 300),
        };
        let content = map_conf(preset, &name, &folder, dimension, dimension_type, i - 1 + base, cwd);
        out.push((unique, content));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(folder: &str, dim: &str) -> ServerWorld {
        ServerWorld { world_folder: PathBuf::from(folder), dimension: Key::parse(dim), dimension_type: None }
    }

    #[test]
    fn auto_ids_like_bluemap() {
        let worlds = [
            world("world", "the_nether"),
            world("world", "overworld"),
            world("world", "the_end"),
            world("My World!", "overworld"),
            world("world", "mymod:mining"),
        ];
        let ids: Vec<String> = auto_map_configs(&worlds, Path::new("/srv")).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["world", "my_world_", "world_the_nether", "world_the_end", "world_mymod_mining"]);
    }

    #[test]
    fn duplicate_folder_and_dimension_gets_counter() {
        let worlds = [world("a/world", "overworld"), world("b/world", "overworld"), world("c/world", "overworld")];
        let maps = auto_map_configs(&worlds, Path::new("/srv"));
        let ids: Vec<&str> = maps.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["world", "world_overworld", "world_overworld_2"]);
        assert!(maps[2].1.contains("name: \"world (overworld) (2)\""));
        assert!(maps[2].1.contains("sorting: 1\n"));
    }
}
