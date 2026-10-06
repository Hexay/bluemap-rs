//! `pack.mcmeta` as BlueMap reads it (`RES/pack/PackMeta.java`, `PackVersion.java`): the parts that decide which
//! overlays load. Any malformed field makes upstream drop the whole meta, so parsing is all-or-nothing.

use serde_json::{Map, Value};

use crate::vfs::Pack;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackVersion {
    pub major: i32,
    pub minor: i32,
}

impl PackVersion {
    pub const fn new(major: i32, minor: i32) -> Self {
        Self { major, minor }
    }

    /// Upstream `isGreaterOrEqual`, operands swapped as in BlueMap: true when `other >= self`.
    pub fn is_greater_or_equal(self, other: Self) -> bool {
        if other.major == self.major { other.minor >= self.minor } else { other.major > self.major }
    }

    /// Upstream `isSmallerOrEqual`, operands swapped as in BlueMap: true when `other <= self`.
    pub fn is_smaller_or_equal(self, other: Self) -> bool {
        if other.major == self.major { other.minor <= self.minor } else { other.major < self.major }
    }

    /// A `min_format`: a missing minor is 0.
    pub fn parse_min(v: &Value) -> Result<Self> {
        parse_version(v, 0)
    }

    /// A `max_format`: a missing minor is `i32::MAX`.
    pub fn parse_max(v: &Value) -> Result<Self> {
        parse_version(v, i32::MAX)
    }
}

fn parse_version(v: &Value, default_minor: i32) -> Result<PackVersion> {
    match v {
        Value::String(s) => parse_version_str(s, default_minor),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            if f == f.floor() {
                return Ok(PackVersion::new(f as i32, default_minor));
            }
            // upstream re-parses `"%.9f"`, so 69.1 becomes minor 100000000
            parse_version_str(&format!("{f:.9}"), default_minor)
        }
        Value::Array(items) => match items.as_slice() {
            [major] => Ok(PackVersion::new(gson_int(major)?, default_minor)),
            [major, minor] => Ok(PackVersion::new(gson_int(major)?, gson_int(minor)?)),
            _ => Err(meta_err(format!("invalid version array: {v}"))),
        },
        _ => Err(meta_err(format!("invalid version format: {v}"))),
    }
}

fn parse_version_str(s: &str, default_minor: i32) -> Result<PackVersion> {
    let (major, minor) = s.split_once('.').map_or((s, None), |(a, b)| (a, Some(b)));
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit());
    let int = |p: &str| p.parse::<i32>().map_err(|_| meta_err(format!("invalid version string: '{s}'")));
    if !digits(major) || !minor.is_none_or(digits) {
        return Err(meta_err(format!("invalid version string: '{s}'")));
    }
    Ok(PackVersion::new(int(major)?, minor.map(int).transpose()?.unwrap_or(default_minor)))
}

/// Gson `nextInt`: integral numbers and numeric strings that fit an `i32`.
pub(crate) fn gson_int(v: &Value) -> Result<i32> {
    let as_int = |f: f64| (f == f.trunc() && f >= i32::MIN as f64 && f <= i32::MAX as f64).then_some(f as i32);
    let i = match v {
        Value::Number(n) => n.as_i64().and_then(|i| i32::try_from(i).ok()).or_else(|| n.as_f64().and_then(as_int)),
        Value::String(s) => s.parse::<i32>().ok().or_else(|| s.parse::<f64>().ok().and_then(as_int)),
        _ => None,
    };
    i.ok_or_else(|| meta_err(format!("expected an int, got {v}")))
}

/// An inclusive range of pack-format majors (`PackMeta.VersionRange`); the default includes everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    pub min_inclusive: i32,
    pub max_inclusive: i32,
}

impl Default for VersionRange {
    fn default() -> Self {
        Self { min_inclusive: i32::MIN, max_inclusive: i32::MAX }
    }
}

impl VersionRange {
    pub fn includes(&self, version: i32) -> bool {
        (self.min_inclusive..=self.max_inclusive).contains(&version)
    }

    /// An int, `[min, max, ...]` (extra items ignored) or `{min_inclusive, max_inclusive}`.
    pub fn parse(v: &Value) -> Result<Self> {
        match v {
            Value::Number(_) => gson_int(v).map(|i| Self { min_inclusive: i, max_inclusive: i }),
            Value::Array(items) if items.len() >= 2 => {
                Ok(Self { min_inclusive: gson_int(&items[0])?, max_inclusive: gson_int(&items[1])? })
            }
            Value::Object(o) => {
                let mut range = Self::default();
                if let Some(v) = field(o, "min_inclusive") {
                    range.min_inclusive = gson_int(v)?;
                }
                if let Some(v) = field(o, "max_inclusive") {
                    range.max_inclusive = gson_int(v)?;
                }
                Ok(range)
            }
            _ => Err(meta_err(format!("invalid version range: {v}"))),
        }
    }
}

/// The `pack` section. Upstream never consults it while loading, but a malformed one still voids the meta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackSection {
    pub min_format: Option<PackVersion>,
    pub max_format: Option<PackVersion>,
    pub pack_format: VersionRange,
    pub supported_formats: Option<VersionRange>,
}

impl PackSection {
    pub fn includes(&self, version: PackVersion) -> bool {
        match (self.min_format, self.max_format) {
            (Some(min), Some(max)) => version.is_greater_or_equal(min) && version.is_smaller_or_equal(max),
            _ => {
                self.supported_formats.is_some_and(|r| r.includes(version.major))
                    || self.pack_format.includes(version.major)
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    pub min_format: Option<PackVersion>,
    pub max_format: Option<PackVersion>,
    pub directory: Option<String>,
    pub formats: VersionRange,
}

impl Overlay {
    /// Both `min_format` and `max_format` set → the (swapped, see [`PackVersion`]) bounds check; else `formats`.
    pub fn includes(&self, version: PackVersion) -> bool {
        match (self.min_format, self.max_format) {
            (Some(min), Some(max)) => version.is_greater_or_equal(min) && version.is_smaller_or_equal(max),
            _ => self.formats.includes(version.major),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackMeta {
    pub pack: PackSection,
    /// In file order; the loader walks them in reverse.
    pub overlays: Vec<Overlay>,
}

impl PackMeta {
    pub fn parse(src: &str) -> Result<Self> {
        let root = crate::json::parse(src)?;
        let root = object(&root)?.ok_or_else(|| meta_err("pack.mcmeta is not an object".into()))?;
        let pack = match field(root, "pack").map(object).transpose()?.flatten() {
            Some(o) => PackSection {
                min_format: field(o, "min_format").map(PackVersion::parse_min).transpose()?,
                max_format: field(o, "max_format").map(PackVersion::parse_max).transpose()?,
                pack_format: field(o, "pack_format").map(VersionRange::parse).transpose()?.unwrap_or_default(),
                supported_formats: field(o, "supported_formats").map(VersionRange::parse).transpose()?,
            },
            None => PackSection::default(),
        };
        let entries = field(root, "overlays").map(object).transpose()?.flatten().and_then(|o| field(o, "entries"));
        let overlays = match entries {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|e| object(e).transpose())
                .map(|e| e.and_then(overlay))
                .collect::<Result<_>>()?,
            Some(v) => return Err(meta_err(format!("overlays.entries is not an array: {v}"))),
        };
        Ok(Self { pack, overlays })
    }

    /// The pack's `pack.mcmeta`; missing or malformed yields the default (no overlays), as upstream.
    pub fn read(pack: &Pack) -> Self {
        pack.read_string("pack.mcmeta").and_then(|s| Self::parse(&s).ok()).unwrap_or_default()
    }
}

fn overlay(o: &Map<String, Value>) -> Result<Overlay> {
    let directory = match field(o, "directory") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(v @ (Value::Number(_) | Value::Bool(_))) => Some(v.to_string()),
        Some(v) => return Err(meta_err(format!("invalid overlay directory: {v}"))),
    };
    Ok(Overlay {
        min_format: field(o, "min_format").map(PackVersion::parse_min).transpose()?,
        max_format: field(o, "max_format").map(PackVersion::parse_max).transpose()?,
        directory,
        formats: field(o, "formats").map(VersionRange::parse).transpose()?.unwrap_or_default(),
    })
}

/// A present, non-null field (Gson leaves defaults in place for JSON nulls on these fields).
fn field<'a>(o: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    o.get(name).filter(|v| !v.is_null())
}

fn object(v: &Value) -> Result<Option<&Map<String, Value>>> {
    match v {
        Value::Object(o) => Ok(Some(o)),
        Value::Null => Ok(None),
        _ => Err(meta_err(format!("expected an object, got {v}"))),
    }
}

fn meta_err(msg: String) -> Error {
    Error::PackMeta(msg)
}

#[cfg(test)]
#[path = "pack_meta_tests.rs"]
mod tests;
