/// Builds a key path from `(text, quoted)` pieces: unquoted text splits on '.', quoted text is taken literally,
/// adjacent pieces join (`a."b.c"` → `["a", "b.c"]`, `foo bar` → `["foo bar"]`).
pub(crate) fn build_path(pieces: &[(String, bool)]) -> Result<Vec<String>, String> {
    let is_blank = |(text, quoted): &&(String, bool)| !*quoted && text.trim().is_empty();
    let start = pieces.iter().position(|p| !is_blank(&p)).unwrap_or(pieces.len());
    let end = pieces.iter().rposition(|p| !is_blank(&p)).map_or(start, |i| i + 1);
    let pieces = &pieces[start..end];
    if pieces.is_empty() {
        return Err("expected a key, found nothing".into());
    }

    let mut path = vec![String::new()];
    let mut touched = vec![false];
    for (text, quoted) in pieces {
        if *quoted {
            path.last_mut().unwrap().push_str(text);
            *touched.last_mut().unwrap() = true;
            continue;
        }
        let trimmed = text.trim();
        for (i, part) in text.split('.').enumerate() {
            if i > 0 {
                path.push(String::new());
                touched.push(false);
            }
            path.last_mut().unwrap().push_str(part);
            if !part.is_empty() && !trimmed.is_empty() {
                *touched.last_mut().unwrap() = true;
            }
        }
    }
    if touched.iter().any(|t| !t) {
        let shown: Vec<&str> = pieces.iter().map(|(t, _)| t.as_str()).collect();
        return Err(format!(
            "'{}' has an empty path element (leading, trailing or doubled '.'); quote keys that contain dots",
            shown.concat()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::build_path;

    fn p(pieces: &[(&str, bool)]) -> Result<Vec<String>, String> {
        build_path(&pieces.iter().map(|(t, q)| (t.to_string(), *q)).collect::<Vec<_>>())
    }

    #[test]
    fn paths() {
        assert_eq!(p(&[("a.b.c", false)]).unwrap(), ["a", "b", "c"]);
        assert_eq!(p(&[("k.", false), ("x.y", true)]).unwrap(), ["k", "x.y"]);
        assert_eq!(p(&[(" ", false), ("a", false), (" ", false), ("b", false), (" ", false)]).unwrap(), ["a b"]);
        assert_eq!(p(&[("", true)]).unwrap(), [""]);
        assert!(p(&[("a..b", false)]).is_err());
        assert!(p(&[(".a", false)]).is_err());
    }
}
