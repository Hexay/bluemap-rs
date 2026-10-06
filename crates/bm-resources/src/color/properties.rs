//! `assets/<ns>/blockProperties.json` (`BlockPropertiesConfig.java`), `BlockProperties` tri-states and how
//! `ResourcePack.loadBlockProperties` combines them with the block's models.

use bm_world::BlockState;
use serde_json::Value;

use super::{ConfigError, LoadFailure, StateMapping, StateMatcher, namespace_files};
use crate::json;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tristate {
    True,
    #[default]
    Undefined,
    False,
}

impl Tristate {
    pub fn get_or(self, default: bool) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Undefined => default,
        }
    }

    /// `getOr(Tristate)`: `self` unless undefined.
    pub fn or(self, other: Self) -> Self {
        if self == Self::Undefined { other } else { self }
    }
}

impl From<bool> for Tristate {
    fn from(b: bool) -> Self {
        if b { Self::True } else { Self::False }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockProperties {
    pub culling: Tristate,
    pub occluding: Tristate,
    pub always_waterlogged: Tristate,
    pub random_offset: Tristate,
    pub culling_identical: Tristate,
}

/// What a block model contributes (`Model.isCulling` / `isOccluding`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelProperties {
    pub culling: bool,
    pub occluding: bool,
}

impl BlockProperties {
    pub fn is_culling(&self) -> bool {
        self.culling.get_or(false)
    }

    pub fn is_occluding(&self) -> bool {
        self.occluding.get_or(false)
    }

    pub fn is_always_waterlogged(&self) -> bool {
        self.always_waterlogged.get_or(false)
    }

    pub fn is_random_offset(&self) -> bool {
        self.random_offset.get_or(false)
    }

    pub fn is_culling_identical(&self) -> bool {
        self.culling_identical.get_or(false)
    }

    /// `Builder.from`: every field `other` defines replaces ours.
    pub fn overridden_by(self, other: &Self) -> Self {
        Self {
            culling: other.culling.or(self.culling),
            occluding: other.occluding.or(self.occluding),
            always_waterlogged: other.always_waterlogged.or(self.always_waterlogged),
            random_offset: other.random_offset.or(self.random_offset),
            culling_identical: other.culling_identical.or(self.culling_identical),
        }
    }

    /// Fills undefined culling/occluding from the first model; once set, later models no longer apply.
    pub fn fill_from_models(&mut self, models: impl IntoIterator<Item = ModelProperties>) {
        for m in models {
            if self.occluding == Tristate::Undefined {
                self.occluding = m.occluding.into();
            }
            if self.culling == Tristate::Undefined {
                self.culling = m.culling.into();
            }
        }
    }
}

/// Every loaded entry in load order; the first one that fits a state wins.
#[derive(Clone, Debug, Default)]
pub struct BlockPropertiesConfig {
    mappings: StateMapping<BlockProperties>,
}

impl BlockPropertiesConfig {
    /// Loads one file. Entries before an error stay loaded, as upstream's streaming reader does. Unknown fields
    /// are skipped; upstream leaves their value unread, which aborts the rest of the file (not replicated).
    pub fn load_str(&mut self, src: &str) -> Result<(), ConfigError> {
        let (members, syntax_error) = json::parse_entries(src);
        for (key, value) in members {
            let matcher = StateMatcher::parse(&key)?;
            let Value::Object(fields) = value else { return Err(ConfigError::Expected { key, expected: "an object" }) };
            let mut props = BlockProperties::default();
            for (name, v) in fields {
                let field = match name.as_str() {
                    "culling" => &mut props.culling,
                    "occluding" => &mut props.occluding,
                    "alwaysWaterlogged" => &mut props.always_waterlogged,
                    "randomOffset" => &mut props.random_offset,
                    "cullingIdentical" => &mut props.culling_identical,
                    _ => continue,
                };
                let Value::Bool(b) = v else { return Err(ConfigError::Expected { key: name, expected: "a boolean" }) };
                *field = b.into();
            }
            self.mappings.push(matcher, props);
        }
        syntax_error.map_or(Ok(()), |e| Err(e.into()))
    }

    pub fn load_pack(&mut self, pack: &crate::Pack, failures: &mut Vec<LoadFailure>) {
        for (file, src) in namespace_files(pack, "blockProperties.json") {
            if let Err(error) = self.load_str(&src) {
                failures.push(LoadFailure { file: format!("{}: {file}", pack.origin), error });
            }
        }
    }

    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }

    /// The configured properties of `state`, all undefined when no entry fits.
    pub fn get(&self, state: &BlockState) -> BlockProperties {
        self.mappings.get(state).copied().unwrap_or_default()
    }

    /// The final properties: config over the (empty) extension defaults, then undefined culling/occluding from
    /// `models`, which the caller produces lazily from the state's resolved variants at (0, 0, 0) in visit order.
    pub fn resolve<I: IntoIterator<Item = ModelProperties>>(
        &self,
        state: &BlockState,
        models: impl FnOnce() -> I,
    ) -> BlockProperties {
        let mut props = BlockProperties::default().overridden_by(&self.get(state));
        if props.occluding == Tristate::Undefined || props.culling == Tristate::Undefined {
            props.fill_from_models(models());
        }
        props
    }
}

#[cfg(test)]
#[path = "properties_tests.rs"]
mod tests;
