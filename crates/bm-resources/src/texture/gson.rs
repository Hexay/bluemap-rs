//! Gson's token coercions for reading `serde_json::Value`s, and its writer's number and string output.

use serde_json::Value;

use crate::key::ResourcePath;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct GsonError(pub &'static str);

pub(crate) type GResult<T> = Result<T, GsonError>;

pub(crate) fn object(v: &Value) -> GResult<&serde_json::Map<String, Value>> {
    v.as_object().ok_or(GsonError("expected an object"))
}

/// `nextBoolean`: only a boolean token.
pub(crate) fn boolean(v: &Value) -> GResult<bool> {
    v.as_bool().ok_or(GsonError("expected a boolean"))
}

/// `nextDouble`: a number, or a string holding one.
pub(crate) fn double(v: &Value) -> GResult<f64> {
    match v {
        Value::Number(n) => n.as_f64().ok_or(GsonError("expected a number")),
        Value::String(s) => s.trim().parse().map_err(|_| GsonError("expected a number")),
        _ => Err(GsonError("expected a number")),
    }
}

/// `nextInt`: a number or numeric string that is an exact `int`.
pub(crate) fn int(v: &Value) -> GResult<i32> {
    if let Value::Number(n) = v
        && let Some(i) = n.as_i64()
    {
        return i32::try_from(i).map_err(|_| GsonError("expected an int"));
    }
    if let Value::String(s) = v
        && let Ok(i) = s.parse::<i32>()
    {
        return Ok(i);
    }
    let d = double(v)?;
    let i = d as i32;
    if i as f64 == d { Ok(i) } else { Err(GsonError("expected an int")) }
}

/// `nextString`: a string or number token.
fn next_string(v: &Value) -> GResult<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(GsonError("expected a string")),
    }
}

/// Gson's `String` adapter: null stays null, booleans are stringified.
pub(crate) fn string(v: &Value) -> GResult<Option<String>> {
    match v {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(b.to_string())),
        _ => next_string(v).map(Some),
    }
}

/// BlueMap's `KeyAdapter` (`Key.parse`, case kept). It is not null-safe, so a JSON null is an error.
pub(crate) fn key(v: &Value) -> GResult<ResourcePath> {
    next_string(v).map(|s| ResourcePath::key(&s))
}

/// Gson's `Boolean` adapter: a boolean, or a string where anything but `true` (any case) is false.
pub(crate) fn lenient_boolean(v: &Value) -> GResult<Option<bool>> {
    match v {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(*b)),
        Value::String(s) => Ok(Some(s.eq_ignore_ascii_case("true"))),
        _ => Err(GsonError("expected a boolean")),
    }
}

/// `Double.compare(a, b) == 0`.
pub(crate) fn java_double_eq(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// `JsonWriter.value(double)` = `Double.toString`: shortest round-trip digits, plain notation for
/// `1e-3 <= |v| < 1e7` (always with a fractional digit), `d.dddE±n` otherwise.
pub(crate) fn write_double(out: &mut String, v: f64) {
    if !v.is_finite() {
        out.push_str(if v.is_nan() {
            "NaN"
        } else if v > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        });
        return;
    }
    if v == 0.0 {
        out.push_str(if v.is_sign_negative() { "-0.0" } else { "0.0" });
        return;
    }
    let (digits, exp) = sci_digits(&format!("{:e}", v.abs()));
    let digits = tie_to_even(v.abs(), digits, exp);
    if v < 0.0 {
        out.push('-');
    }
    if (-3..7).contains(&exp) {
        if exp < 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
            out.push_str(&digits);
        } else {
            let int_len = exp as usize + 1;
            let padded = format!("{digits:0<int_len$}");
            let (int, frac) = padded.split_at(int_len);
            out.push_str(int);
            out.push('.');
            out.push_str(if frac.is_empty() { "0" } else { frac });
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push('E');
        out.push_str(&exp.to_string());
    }
}

/// `{:e}` output as (significant digits, decimal exponent).
fn sci_digits(sci: &str) -> (String, i32) {
    let (mantissa, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    (mantissa.chars().filter(|c| *c != '.').collect(), exp.parse().expect("{:e} exponent is an integer"))
}

/// Java breaks an exact tie between two shortest decimals towards the even last digit; Rust rounds up.
fn tie_to_even(v: f64, digits: String, exp: i32) -> String {
    let n = digits.len();
    // 800 fractional digits hold any f64 exactly
    let (exact, exact_exp) = sci_digits(&format!("{v:.800e}"));
    let rest = &exact.as_bytes()[n.min(exact.len())..];
    if exact_exp != exp || rest.first() != Some(&b'5') || rest[1..].iter().any(|&d| d != b'0') {
        return digits;
    }
    let mut even = exact.as_bytes()[..n].to_vec();
    if even[n - 1] % 2 == 1 {
        let Some(i) = even.iter().rposition(|&d| d != b'9') else { return digits };
        even[i] += 1;
        even[i + 1..].fill(b'0');
    }
    let even = String::from_utf8(even).expect("ASCII digits");
    let parsed: Option<f64> = format!("{}.{}e{exp}", &even[..1], &even[1..]).parse().ok();
    if parsed == Some(v) { even } else { digits }
}

/// `JsonWriter.string` with Gson's default HTML-safe escaping (`=` becomes `=`, so base64 padding does too).
pub(crate) fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(v: f64) -> String {
        let mut s = String::new();
        write_double(&mut s, v);
        s
    }

    #[test]
    fn doubles_print_like_java() {
        assert_eq!(d(1.0), "1.0");
        assert_eq!(d(0.0), "0.0");
        assert_eq!(d(0.5), "0.5");
        assert_eq!(d(0.40490248799324036), "0.40490248799324036");
        assert_eq!(d(0.001), "0.001");
        assert_eq!(d(0.000_45), "4.5E-4");
        assert_eq!(d(1e-10), "1.0E-10");
        assert_eq!(d(1234567.0), "1234567.0");
        assert_eq!(d(12345678.0), "1.2345678E7");
        assert_eq!(d(-2.5), "-2.5");
        assert_eq!(d(100.0), "100.0");
        assert_eq!(d(f64::from(0.1f32)), "0.10000000149011612");
        // exactly 0.70874786376953125: a tie between ...12 and ...13
        assert_eq!(d("0.70874786376953125".parse().unwrap()), "0.7087478637695312");
        assert_eq!(d(0.125), "0.125");
    }

    #[test]
    fn strings_escape_like_gson() {
        let mut s = String::new();
        write_string(&mut s, "a=b<'>&\"\\\n\u{1}é");
        // `~` stands for a backslash
        assert_eq!(s, r#""a~u003db~u003c~u0027~u003e~u0026~"~~~n~u0001é""#.replace('~', "\\"));
    }

    #[test]
    fn coercions() {
        use serde_json::json;
        assert_eq!(int(&json!(3)), Ok(3));
        assert_eq!(int(&json!(3.0)), Ok(3));
        assert_eq!(int(&json!("7")), Ok(7));
        assert!(int(&json!(2.5)).is_err());
        assert!(int(&json!(1i64 << 40)).is_err());
        assert_eq!(double(&json!("0.5")), Ok(0.5));
        assert_eq!(string(&json!(true)), Ok(Some("true".into())));
        assert!(key(&Value::Null).is_err());
        assert_eq!(key(&json!("Mod:X")).unwrap().as_str(), "Mod:X");
    }
}
