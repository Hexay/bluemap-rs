//! HOCON memory sizes (typesafe-config `getBytes`): `512M`, `1.5 GiB`, `2GB`, `1048576`.

use serde::de::{Deserialize, Deserializer, Error};

use super::coerce;
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MemorySizeError {
    #[error("'{0}' is not a memory size (expected e.g. 512M, 2G, 1.5GiB or a byte count)")]
    Malformed(String),
    #[error("unknown memory size unit '{0}' (expected B, K/Ki/KiB, kB, M/Mi/MiB, MB, G/Gi/GiB, GB, T/Ti/TiB, TB)")]
    Unit(String),
    #[error("memory size '{0}' is out of range")]
    Range(String),
}

const PREFIXES: [(char, &str, &str); 6] = [
    ('k', "kilo", "kibi"),
    ('m', "mega", "mebi"),
    ('g', "giga", "gibi"),
    ('t', "tera", "tebi"),
    ('p', "peta", "pebi"),
    ('e', "exa", "exbi"),
];

/// Bytes per `unit`: single letters and `Ki`/`KiB` are powers of 1024, `kB`/`MB`/… powers of 1000.
fn unit_factor(unit: &str) -> Option<f64> {
    if matches!(unit, "" | "B" | "b" | "byte" | "bytes") {
        return Some(1.0);
    }
    let lower = unit.to_ascii_lowercase();
    let first = lower.chars().next()?;
    let rest = &unit[first.len_utf8()..];
    let word = lower.strip_suffix('s').unwrap_or(&lower).strip_suffix("byte");
    for (i, (letter, decimal, binary)) in PREFIXES.iter().enumerate() {
        let exp = i as i32 + 1;
        if word == Some(binary) || first == *letter && matches!(rest, "" | "i" | "iB") {
            return Some(1024f64.powi(exp));
        }
        if word == Some(decimal) || first == *letter && rest == "B" {
            return Some(1000f64.powi(exp));
        }
    }
    None
}

pub fn parse_memory_size(text: &str) -> Result<u64, MemorySizeError> {
    let trimmed = text.trim();
    let split = trimmed.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split);
    let number: f64 = number.parse().map_err(|_| MemorySizeError::Malformed(text.to_owned()))?;
    let factor = unit_factor(unit.trim()).ok_or_else(|| MemorySizeError::Unit(unit.trim().to_owned()))?;
    let bytes = number * factor;
    if !bytes.is_finite() || bytes >= u64::MAX as f64 {
        return Err(MemorySizeError::Range(text.to_owned()));
    }
    Ok(bytes as u64)
}

/// `None` when unset or 0 (no limit).
pub(crate) fn opt_memory_size<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let bytes = match Value::deserialize(d)? {
        Value::Null => return Ok(None),
        Value::Int(i) => u64::try_from(i).map_err(|_| MemorySizeError::Malformed(i.to_string())),
        v => parse_memory_size(&coerce::to_string(&v).map_err(D::Error::custom)?),
    };
    bytes.map(|b| (b > 0).then_some(b)).map_err(D::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_like_typesafe_config() {
        let p = |s: &str| parse_memory_size(s).unwrap();
        assert_eq!(p("1048576"), 1 << 20);
        assert_eq!(p("512M"), 512 << 20);
        assert_eq!(p("512m"), 512 << 20);
        assert_eq!(p("512MiB"), 512 << 20);
        assert_eq!(p("512 Mi"), 512 << 20);
        assert_eq!(p("1G"), 1 << 30);
        assert_eq!(p(" 1.5 GiB "), 3 << 29);
        assert_eq!(p("2GB"), 2_000_000_000);
        assert_eq!(p("4kB"), 4000);
        assert_eq!(p("4K"), 4096);
        assert_eq!(p("3 gigabytes"), 3_000_000_000);
        assert_eq!(p("1 tebibyte"), 1 << 40);
        assert_eq!(p("100 bytes"), 100);
        assert_eq!(p("0"), 0);
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(matches!(parse_memory_size("lots"), Err(MemorySizeError::Malformed(_))));
        assert!(matches!(parse_memory_size("-1G"), Err(MemorySizeError::Malformed(_))));
        assert!(matches!(parse_memory_size("12 parsecs"), Err(MemorySizeError::Unit(u)) if u == "parsecs"));
        assert!(matches!(parse_memory_size("99999999E"), Err(MemorySizeError::Range(_))));
        assert!(parse_memory_size("").is_err());
    }
}
