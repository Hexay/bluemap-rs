//! `Double.toString` / `Float.toString` (JDK 19+ shortest-decimal algorithm and rendering rules).

pub fn double_to_string(v: f64) -> String {
    if let Some(s) = special(v.is_nan(), v.is_infinite(), v == 0.0, v.is_sign_negative()) {
        return s.to_owned();
    }
    let a = v.abs();
    let (digits, exp) = shortest(&format!("{a:e}"), |p| format!("{a:.p$e}"), |s| s.parse::<f64>() == Ok(a));
    render(v.is_sign_negative(), &digits, exp)
}

pub fn float_to_string(v: f32) -> String {
    if let Some(s) = special(v.is_nan(), v.is_infinite(), v == 0.0, v.is_sign_negative()) {
        return s.to_owned();
    }
    let a = v.abs();
    let (digits, exp) = shortest(&format!("{a:e}"), |p| format!("{a:.p$e}"), |s| s.parse::<f32>() == Ok(a));
    render(v.is_sign_negative(), &digits, exp)
}

fn special(nan: bool, inf: bool, zero: bool, neg: bool) -> Option<&'static str> {
    match (nan, inf, zero, neg) {
        (true, ..) => Some("NaN"),
        (_, true, _, false) => Some("Infinity"),
        (_, true, _, true) => Some("-Infinity"),
        (_, _, true, false) => Some("0.0"),
        (_, _, true, true) => Some("-0.0"),
        _ => None,
    }
}

/// Significant digits and the exponent of the first one. Rust's `{:e}` gives the shortest length but not always
/// the closest decimal of that length (exact ties: Java takes the even one); Java also widens 1 digit to 2 (`1.4E-45`).
/// `rounded(p)` formats the exact value correctly rounded to `p` fraction digits.
fn shortest(sci: &str, rounded: impl Fn(usize) -> String, round_trips: impl Fn(&str) -> bool) -> (String, i32) {
    let split = |s: &str| -> (String, i32) {
        let (mantissa, exp) = s.split_once('e').unwrap_or((s, "0"));
        let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
        let digits = digits.trim_end_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        (digits.to_owned(), exp.parse().unwrap_or(0))
    };
    let (digits, exp) = split(sci);
    // the nearest decimal of that length can fall outside the rounding interval only at binade edges
    let nearest = rounded(digits.len().max(2) - 1);
    if round_trips(&nearest) {
        return split(&nearest);
    }
    (digits, exp)
}

fn render(negative: bool, digits: &str, exp: i32) -> String {
    let mut s = String::with_capacity(digits.len() + 8);
    if negative {
        s.push('-');
    }
    let n = digits.len() as i32;
    if (-3..7).contains(&exp) {
        let point = exp + 1;
        if point <= 0 {
            s.push_str("0.");
            s.extend(std::iter::repeat_n('0', (-point) as usize));
            s.push_str(digits);
        } else if point < n {
            s.push_str(&digits[..point as usize]);
            s.push('.');
            s.push_str(&digits[point as usize..]);
        } else {
            s.push_str(digits);
            s.extend(std::iter::repeat_n('0', (point - n) as usize));
            s.push_str(".0");
        }
    } else {
        s.push_str(&digits[..1]);
        s.push('.');
        s.push_str(if n > 1 { &digits[1..] } else { "0" });
        s.push('E');
        s.push_str(&exp.to_string());
    }
    s
}
