//! Scalar coercions of Configurate 4.2's `NumericSerializers`, `BooleanSerializer` and `StringSerializer`
//! (verified against the real jar with `tools/probe.sh typed`).

use crate::value::{Value, java_double_to_string};

pub(crate) fn not_scalar(v: &Value) -> String {
    format!("expected a single value, got {}", v.type_name())
}

pub(crate) fn to_bool(v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Int(i) => Ok(*i != 0),
        // Configurate checks `!number.equals(Integer 0)`, which a Double never equals: 0.0 means true.
        Value::Float(_) => Ok(true),
        Value::String(s) => match s.to_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "1" => Ok(true),
            "false" | "f" | "no" | "n" | "0" => Ok(false),
            _ => Err(format!("\"{s}\" is not a boolean (use true or false)")),
        },
        other => Err(not_scalar(other)),
    }
}

/// Integer in `[min, max]` (`what` = "int"/"long" for messages).
pub(crate) fn to_int(v: &Value, min: i64, max: i64, what: &str) -> Result<i64, String> {
    let range_err = |n: &dyn std::fmt::Display| format!("{n} is out of range for {what} ([{min},{max}])");
    match v {
        Value::Int(i) if (min..=max).contains(i) => Ok(*i),
        Value::Int(i) => Err(range_err(i)),
        Value::Float(f) => {
            let whole = (f - f.floor()).abs() < f32::MIN_POSITIVE as f64;
            if whole && *f >= min as f64 && *f <= max as f64 {
                Ok(*f as i64)
            } else {
                Err(format!("{} is not a whole number in range for {what}", java_double_to_string(*f)))
            }
        }
        Value::String(s) => {
            let n = parse_java_int(s).ok_or_else(|| format!("For input string: \"{s}\" (expected a whole number)"))?;
            if (min as i128..=max as i128).contains(&n) { Ok(n as i64) } else { Err(range_err(&n)) }
        }
        Value::Bool(b) => Err(format!("{b} is not a number")),
        other => Err(not_scalar(other)),
    }
}

/// Configurate `parseNumber`: optional `-`/`+`, `0x`/`#`/`0b` radix prefixes, `u` (unsigned) suffix.
fn parse_java_int(s: &str) -> Option<i128> {
    let mut body = s;
    let unsigned = body.ends_with('u');
    if unsigned {
        body = &body[..body.len() - 1];
    }
    let negative = body.starts_with('-');
    if negative {
        if unsigned {
            return None;
        }
        body = &body[1..];
    } else if let Some(rest) = body.strip_prefix('+') {
        body = rest;
    }
    let (radix, digits) = if let Some(d) = body.strip_prefix("0x") {
        (16, d)
    } else if let Some(d) = body.strip_prefix('#') {
        (16, d)
    } else if let Some(d) = body.strip_prefix("0b") {
        (2, d)
    } else {
        (10, body)
    };
    let digits = if negative { format!("-{digits}") } else { digits.to_owned() };
    let n = i128::from_str_radix(&digits, radix).ok()?;
    // unsigned parses wrap into the signed range like Integer.parseUnsignedInt
    Some(if unsigned && n > i32::MAX as i128 && n <= u32::MAX as i128 { n - (1i128 << 32) } else { n })
}

pub(crate) fn to_f64(v: &Value) -> Result<f64, String> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        Value::String(s) => parse_java_float(s).ok_or_else(|| format!("For input string: \"{s}\" (expected a number)")),
        Value::Bool(b) => Err(format!("{b} is not a number")),
        other => Err(not_scalar(other)),
    }
}

pub(crate) fn to_f32(v: &Value) -> Result<f32, String> {
    match v {
        Value::String(s) => {
            parse_java_float::<f32>(s).ok_or_else(|| format!("For input string: \"{s}\" (expected a number)"))
        }
        other => to_f64(other).map(|f| f as f32),
    }
}

/// Java `Double.parseDouble`/`Float.parseFloat`: surrounding whitespace, a `d`/`f` suffix, `NaN`, `Infinity`.
fn parse_java_float<F: std::str::FromStr>(s: &str) -> Option<F> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    let (sign, rest) = match t.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", t.strip_prefix('+').unwrap_or(t)),
    };
    let normalized = match rest {
        "NaN" => "NaN".to_owned(),
        "Infinity" => format!("{sign}inf"),
        r if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) => {
            format!("{sign}{r}")
        }
        _ => return None,
    };
    normalized.parse().ok()
}

pub(crate) fn to_string(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Int(i) => Ok(i.to_string()),
        Value::Float(f) => Ok(java_double_to_string(*f)),
        other => Err(not_scalar(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_like_configurate() {
        let s = |x: &str| Value::String(x.into());
        assert_eq!(to_int(&s("+7"), i32::MIN as i64, i32::MAX as i64, "int"), Ok(7));
        assert_eq!(to_int(&s("0x10"), i32::MIN as i64, i32::MAX as i64, "int"), Ok(16));
        assert_eq!(to_int(&s("-3"), i32::MIN as i64, i32::MAX as i64, "int"), Ok(-3));
        assert_eq!(to_int(&s("#ff"), i32::MIN as i64, i32::MAX as i64, "int"), Ok(255));
        assert_eq!(to_int(&s("0b101"), i32::MIN as i64, i32::MAX as i64, "int"), Ok(5));
        assert!(to_int(&s("  3"), i32::MIN as i64, i32::MAX as i64, "int").is_err());
        assert!(to_int(&s("7.0"), i32::MIN as i64, i32::MAX as i64, "int").is_err());
        assert_eq!(to_int(&Value::Float(2.0), i32::MIN as i64, i32::MAX as i64, "int"), Ok(2));
        assert!(to_int(&Value::Float(1.5), i32::MIN as i64, i32::MAX as i64, "int").is_err());
        assert!(to_int(&Value::Int(2147483648), i32::MIN as i64, i32::MAX as i64, "int").is_err());
    }

    #[test]
    fn bools_like_configurate() {
        let s = |x: &str| to_bool(&Value::String(x.into()));
        assert_eq!(s("yes"), Ok(true));
        assert_eq!(s("TRUE"), Ok(true));
        assert_eq!(s("n"), Ok(false));
        assert_eq!(s("1"), Ok(true));
        assert!(s("maybe").is_err());
        assert_eq!(to_bool(&Value::Int(0)), Ok(false));
        assert_eq!(to_bool(&Value::Int(2)), Ok(true));
        assert_eq!(to_bool(&Value::Float(0.0)), Ok(true));
    }

    #[test]
    fn floats_and_strings() {
        assert_eq!(to_f32(&Value::String("0.5".into())), Ok(0.5));
        assert_eq!(to_f64(&Value::String(" 2.5d ".into())), Ok(2.5));
        assert_eq!(to_f64(&Value::String("-Infinity".into())), Ok(f64::NEG_INFINITY));
        assert!(to_f64(&Value::String("inf".into())).is_err());
        assert!(to_f64(&Value::String("abc".into())).is_err());
        assert_eq!(to_string(&Value::Float(1.0)), Ok("1.0".into()));
        assert_eq!(to_string(&Value::Bool(true)), Ok("true".into()));
        assert!(to_string(&Value::List(vec![])).is_err());
    }
}
