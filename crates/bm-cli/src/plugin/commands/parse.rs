//! Matching `/bluemap …` input against the usages of `commands.json` (upstream's BlueCommands tree): literals
//! match exactly, `<map>`/`<storage>` must name a known id, coordinates and radii are ints, anything else a word.

use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Spec {
    pub root: String,
    pub commands: Vec<Usage>,
}

#[derive(Deserialize)]
pub struct Usage {
    pub usage: String,
    pub permission: String,
}

pub static SPEC: LazyLock<Spec> =
    LazyLock::new(|| serde_json::from_str(include_str!("../commands.json")).expect("valid commands.json"));

pub struct Matched {
    pub usage: &'static str,
    pub permission: &'static str,
    args: Vec<(&'static str, String)>,
}

impl Matched {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.args.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    pub fn int(&self, name: &str) -> Option<i32> {
        self.get(name).and_then(|v| v.parse().ok())
    }
}

/// Known ids; empty slices accept any word (nothing loaded yet).
pub struct Ids<'a> {
    pub maps: &'a [String],
    pub storages: &'a [String],
}

/// The first usage `input` (without `/`) matches; `None` if it isn't a valid `/bluemap` command.
pub fn parse(input: &str, ids: &Ids) -> Option<Matched> {
    let mut words = input.split_whitespace();
    if words.next()? != SPEC.root {
        return None;
    }
    let words: Vec<&str> = words.collect();
    SPEC.commands.iter().find_map(|u| match_usage(u, &words, ids))
}

fn match_usage(u: &'static Usage, words: &[&str], ids: &Ids) -> Option<Matched> {
    let tokens: Vec<&'static str> = u.usage.split_whitespace().collect();
    if tokens.len() != words.len() {
        return None;
    }
    let mut args = Vec::new();
    for (token, word) in tokens.iter().zip(words) {
        match token.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
            None if token == word => {}
            None => return None,
            Some(name) => {
                let known = |list: &[String]| list.is_empty() || list.iter().any(|id| id == word);
                let ok = match name {
                    // `storages <s> delete <map>` names maps that are not loaded
                    "map" => u.usage.starts_with("storages") || known(ids.maps),
                    "storage" => known(ids.storages),
                    "x" | "y" | "z" | "radius" => word.parse::<i32>().is_ok(),
                    _ => true,
                };
                if !ok {
                    return None;
                }
                args.push((name, (*word).to_owned()));
            }
        }
    }
    Some(Matched { usage: &u.usage, permission: &u.permission, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Vec<String>, Vec<String>) {
        (vec!["world".into(), "nether".into()], vec!["file".into()])
    }

    #[test]
    fn matches_usages_and_permissions() {
        let (maps, storages) = ids();
        let ids = Ids { maps: &maps, storages: &storages };
        let m = parse("bluemap", &ids).unwrap();
        assert_eq!((m.usage, m.permission), ("", "bluemap.status"));
        let m = parse("bluemap freeze world", &ids).unwrap();
        assert_eq!((m.usage, m.get("map")), ("freeze <map>", Some("world")));
        assert!(parse("bluemap freeze nope", &ids).is_none());
        let m = parse("bluemap update 100", &ids).unwrap();
        assert_eq!((m.usage, m.int("radius")), ("update <radius>", Some(100)));
        let m = parse("bluemap force-update nether 1 -2 30", &ids).unwrap();
        assert_eq!(m.usage, "force-update <map> <x> <z> <radius>");
        assert_eq!((m.int("x"), m.int("z")), (Some(1), Some(-2)));
        let m = parse("bluemap storages file delete world", &ids).unwrap();
        assert_eq!(m.permission, "bluemap.storages.delete");
        assert_eq!(parse("bluemap storages file delete old", &ids).unwrap().get("map"), Some("old"));
        assert_eq!(parse("bluemap reload light", &ids).unwrap().permission, "bluemap.reload.light");
        assert!(parse("other", &ids).is_none());
        assert!(parse("bluemap maps extra", &ids).is_none());
    }

    #[test]
    fn every_usage_parses() {
        let (maps, storages) = ids();
        let ids = Ids { maps: &maps, storages: &storages };
        for u in &SPEC.commands {
            let input: Vec<&str> = std::iter::once("bluemap")
                .chain(u.usage.split_whitespace().map(|t| match t {
                    "<map>" => "world",
                    "<storage>" => "file",
                    "<x>" | "<y>" | "<z>" | "<radius>" => "5",
                    "<task-ref>" => "ab12",
                    lit => lit,
                }))
                .collect();
            let m = parse(&input.join(" "), &ids).unwrap_or_else(|| panic!("{}", u.usage));
            assert_eq!(m.permission, u.permission, "{}", u.usage);
        }
    }
}
