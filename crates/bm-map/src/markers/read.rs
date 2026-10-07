//! Gson 2.8.9's scalar coercions as MarkerGson meets them. Marker sets are read by the streaming `JsonReader`;
//! markers go through `JsonDeserializer` and so are read from a `JsonElement` tree, where ints truncate and wrap
//! (`LazilyParsedNumber.intValue`) instead of failing.
//! Deliberate deviation: non-finite doubles are rejected (Java writes `NaN` the webapp cannot parse, or fails).

use super::json::Json;

type R<T> = Result<T, String>;

fn expected<T>(what: &str, v: &Json) -> R<T> {
    Err(format!("expected {what}, got {}", v.kind()))
}

/// `TypeAdapters.STRING`: booleans and numbers (literal text) are accepted too.
pub(super) fn string(v: &Json) -> R<String> {
    match v {
        Json::Str(s) | Json::Num(s) => Ok(s.clone()),
        Json::Bool(b) => Ok(b.to_string()),
        _ => expected("a string", v),
    }
}

/// `TypeAdapters.BOOLEAN`: strings via `Boolean.parseBoolean` (anything but "true" is false).
pub(super) fn boolean(v: &Json) -> R<bool> {
    match v {
        Json::Bool(b) => Ok(*b),
        Json::Str(s) => Ok(s.eq_ignore_ascii_case("true")),
        _ => expected("a boolean", v),
    }
}

/// `JsonTreeReader.nextInt`: numbers through `LazilyParsedNumber.intValue`, strings through `Integer.parseInt`.
pub(super) fn tree_int(v: &Json) -> R<i32> {
    match v {
        Json::Num(t) => big_decimal_int_value(t).ok_or_else(|| format!("{t} is not a number")),
        Json::Str(s) => s.parse().map_err(|_| format!("\"{s}\" is not an int")),
        _ => expected("an int", v),
    }
}

/// `JsonReader.nextInt`: the value has to be a whole number in int range.
pub(super) fn stream_int(v: &Json) -> R<i32> {
    let (Json::Num(t) | Json::Str(t)) = v else { return expected("an int", v) };
    if let Ok(i) = t.parse() {
        return Ok(i);
    }
    let d = java_parse_double(t).ok_or_else(|| format!("\"{t}\" is not an int"))?;
    let i = d as i32;
    if i as f64 == d { Ok(i) } else { Err(format!("{t} is not an int")) }
}

/// `nextDouble` on either reader: `Double.parseDouble` of the number text or string.
pub(super) fn double(v: &Json) -> R<f64> {
    let (Json::Num(t) | Json::Str(t)) = v else { return expected("a number", v) };
    let d = java_parse_double(t).ok_or_else(|| format!("\"{t}\" is not a number"))?;
    if d.is_finite() { Ok(d) } else { Err(format!("{t} is not a finite number")) }
}

/// `(float) nextDouble()`.
pub(super) fn float(v: &Json) -> R<f32> {
    let f = double(v)? as f32;
    if f.is_finite() { Ok(f) } else { Err(format!("{v:?} overflows a float")) }
}

/// `Double.parseDouble`, minus hex literals; `NaN`/`Infinity` come back as such.
pub(super) fn java_parse_double(s: &str) -> Option<f64> {
    let s = s.trim_matches(|c: char| c <= ' ');
    let s = s.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(s);
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    match unsigned {
        "NaN" => return Some(f64::NAN),
        "Infinity" => return Some(if s.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY }),
        _ => {}
    }
    let numeric = unsigned.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'));
    if numeric { s.parse().ok() } else { None }
}

/// `new BigDecimal(text).intValue()`: the integer part, truncated, modulo 2^32 (exact, not via a double).
pub(super) fn big_decimal_int_value(text: &str) -> Option<i32> {
    let (negative, rest) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    let (mantissa, exp) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], rest[i + 1..].parse::<i64>().ok()?),
        None => (rest, 0),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = || int_part.bytes().chain(frac_part.bytes());
    if digits().next().is_none() || !digits().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let point = int_part.len() as i64 + exp;
    let mut value = 0u32;
    for (i, d) in digits().enumerate() {
        if i as i64 >= point {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(u32::from(d - b'0'));
    }
    // 10^32 ≡ 0 (mod 2^32), so trailing zeros past 32 change nothing
    let zeros = (point - (int_part.len() + frac_part.len()) as i64).clamp(0, 32);
    for _ in 0..zeros {
        value = value.wrapping_mul(10);
    }
    Some(if negative { value.wrapping_neg() } else { value } as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(t: &str) -> Json {
        Json::Num(t.into())
    }

    #[test]
    fn tree_ints_wrap_like_lazily_parsed_number() {
        // values printed by Java 5.28 for these config numbers (see tests/data/markers)
        for (t, want) in [("1.0E10", 1410065408), ("1.0E30", 1073741824), ("3000000000", -1294967296), ("-2.9", -2)] {
            assert_eq!(tree_int(&num(t)), Ok(want), "{t}");
        }
        assert_eq!(big_decimal_int_value("12.5e1"), Some(125));
        assert_eq!(big_decimal_int_value("-0.5"), Some(0));
        assert_eq!(big_decimal_int_value("1e-3"), Some(0));
        assert_eq!(tree_int(&Json::Str("5".into())), Ok(5));
        assert!(tree_int(&Json::Str("5.0".into())).is_err());
    }

    #[test]
    fn stream_ints_must_be_exact() {
        assert_eq!(stream_int(&num("1.0")), Ok(1));
        assert_eq!(stream_int(&Json::Str("7.0".into())), Ok(7));
        assert!(stream_int(&num("1.5")).is_err());
        assert!(stream_int(&num("3000000000")).is_err());
    }

    #[test]
    fn scalar_coercions() {
        assert_eq!(string(&num("1.5")), Ok("1.5".into()));
        assert_eq!(string(&Json::Bool(true)), Ok("true".into()));
        assert_eq!(boolean(&Json::Str("TRUE".into())), Ok(true));
        assert_eq!(boolean(&Json::Str("yes".into())), Ok(false));
        assert!(boolean(&num("1")).is_err());
        assert_eq!(double(&Json::Str(" 50d ".into())), Ok(50.0));
        assert!(double(&Json::Str("NaN".into())).is_err());
        assert!(double(&Json::Str("inf".into())).is_err());
        assert!(float(&num("1e300")).is_err());
    }
}
