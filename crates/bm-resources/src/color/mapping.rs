//! Config keys like `minecraft:melon_stem[age=7]` and the per-block-id lookup of `BlockStateMapping`.

use bm_world::BlockState;
use rustc_hash::FxHashMap;

use super::ConfigError;
use crate::ResourcePath;

/// `BlockState.fromString` as a pattern: equal by id and property set, like Java's `BlockState.equals`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StateMatcher {
    pub id: ResourcePath,
    /// Sorted by key; a key listed twice keeps its last value (Java `HashMap.put`).
    pub properties: Vec<(String, String)>,
}

impl StateMatcher {
    /// Parses `id[k=v,…]`. Mirrors Java: only the id and the whole property list are trimmed, `split(",")` drops
    /// trailing empty items, and an item without `=` fails.
    pub fn parse(s: &str) -> Result<Self, ConfigError> {
        let (id, props) = match s.find('[') {
            Some(i) if s.ends_with(']') => (&s[..i], Some(&s[i + 1..s.len() - 1])),
            _ => (s, None),
        };
        let mut properties: Vec<(String, String)> = Vec::new();
        if let Some(props) = props.filter(|p| !p.is_empty()) {
            let mut items: Vec<&str> = props.trim().split(',').collect();
            while items.len() > 1 && items.last() == Some(&"") {
                items.pop();
            }
            for item in items {
                let (k, v) = item.split_once('=').ok_or_else(|| ConfigError::BadState(s.to_owned()))?;
                match properties.iter_mut().find(|(pk, _)| pk == k) {
                    Some(p) => p.1 = v.to_owned(),
                    None => properties.push((k.to_owned(), v.to_owned())),
                }
            }
        }
        properties.sort_unstable();
        Ok(Self { id: ResourcePath::key(id.trim()), properties })
    }

    /// `BlockStateMapping.fitsTo`: same id and every listed property equal; unlisted ones are ignored.
    pub fn fits(&self, state: &BlockState) -> bool {
        *state.name == *self.id.as_str() && self.properties.iter().all(|(k, v)| state.property(k) == Some(v.as_str()))
    }
}

/// Mappings grouped by block id; the first one that fits wins.
///
/// Upstream builds the blockColors lists by iterating a `HashMap`, so overlapping keys resolve in hash order. Here
/// every list is in load order (higher-priority pack first, then file order), which also matches what upstream
/// does for blockProperties.
#[derive(Clone, Debug)]
pub struct StateMapping<T> {
    by_id: FxHashMap<Box<str>, Vec<(StateMatcher, T)>>,
}

impl<T> Default for StateMapping<T> {
    fn default() -> Self {
        Self { by_id: FxHashMap::default() }
    }
}

impl<T> StateMapping<T> {
    pub fn push(&mut self, matcher: StateMatcher, value: T) {
        self.by_id.entry(matcher.id.as_str().into()).or_default().push((matcher, value));
    }

    pub fn get(&self, state: &BlockState) -> Option<&T> {
        let list = self.by_id.get(&*state.name)?;
        list.iter().find(|(m, _)| m.fits(state)).map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        self.by_id.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use bm_world::BlockStates;

    use super::*;

    fn m(s: &str) -> StateMatcher {
        StateMatcher::parse(s).unwrap()
    }

    #[test]
    fn parses_like_block_state_from_string() {
        let a = m(" stone [b=2,a=1]");
        assert_eq!(a.id.as_str(), "minecraft:stone");
        assert_eq!(a, m("minecraft:stone [a=1,b=2]"));
        assert_eq!(m("x[a=1,a=2]").properties, [("a".into(), "2".into())]);
        assert_eq!(m("x[ a=1 , b= ]").properties, [(" b".into(), "".into()), ("a".into(), "1 ".into())]);
        assert_eq!(m("x[a=1,,]").properties.len(), 1);
        assert_eq!(m("x[]").properties, []);
        assert_eq!(m("x[a=1").id.as_str(), "minecraft:x[a=1");
        for bad in ["x[a]", "x[,a=1]", "x[a=1,,b=2]", "x[ ]"] {
            assert!(StateMatcher::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn first_fitting_mapping_wins() {
        let states = BlockStates::default();
        let mut map = StateMapping::default();
        map.push(m("minecraft:stem[age=7]"), 7);
        map.push(m("stem"), 0);
        map.push(m("stem[age=7,facing=up]"), 99);
        let s = |p: &str| states.get(states.intern_str(p).unwrap());
        assert_eq!(map.get(&s("stem[age=7,facing=up]")), Some(&7));
        assert_eq!(map.get(&s("stem[age=3]")), Some(&0));
        assert_eq!(map.get(&s("other")), None);
        assert_eq!(map.len(), 3);
    }
}
