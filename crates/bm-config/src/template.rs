//! `ConfigTemplate.java`: `${name<<...>>}` conditional blocks, then `${name}` variables (unknown ones become `?`).

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct ConfigTemplate {
    template: String,
    conditionals: HashSet<String>,
    variables: HashMap<String, String>,
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Matches `${name` at `s`; returns the name and the rest after it.
fn name_at(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("${")?;
    let end = rest.find(|c| !is_name_char(c)).unwrap_or(rest.len());
    (end > 0).then(|| (&rest[..end], &rest[end..]))
}

/// Java `Matcher.replaceAll` over `${name<<body>>}` (body: shortest match) or `${name}`.
fn replace_all(input: &str, conditional: bool, mut replace: impl FnMut(&str, &str) -> String) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let here = &input[i..];
        let matched = name_at(here).and_then(|(name, rest)| {
            if conditional {
                let body_start = rest.strip_prefix("<<")?;
                let body_len = body_start.find(">>}")?;
                Some((name, &body_start[..body_len], here.len() - body_start.len() + body_len + 3))
            } else {
                rest.starts_with('}').then(|| (name, "", here.len() - rest.len() + 1))
            }
        });
        match matched {
            Some((name, body, len)) => {
                out.push_str(&replace(name, body));
                i += len;
            }
            None => {
                let c = here.chars().next().unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

impl ConfigTemplate {
    pub fn new(template: impl Into<String>) -> Self {
        Self { template: template.into(), ..Self::default() }
    }

    pub fn conditional(mut self, name: &str, enabled: bool) -> Self {
        if enabled {
            self.conditionals.insert(name.to_owned());
        } else {
            self.conditionals.remove(name);
        }
        self
    }

    /// `None` unsets the variable, which then renders as `?` (Java `setVariable(name, null)`).
    pub fn variable(mut self, name: &str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.variables.insert(name.to_owned(), v.to_owned()),
            None => self.variables.remove(name),
        };
        self
    }

    pub fn var(self, name: &str, value: &str) -> Self {
        self.variable(name, Some(value))
    }

    pub fn build(&self) -> String {
        self.build_text(&self.template)
    }

    // Same two passes as Java: variables substituted inside a conditional body are scanned again by the outer pass.
    fn build_text(&self, text: &str) -> String {
        let resolved = replace_all(text, true, |name, body| {
            if self.conditionals.contains(name) { self.build_text(body) } else { String::new() }
        });
        replace_all(&resolved, false, |name, _| self.variables.get(name).cloned().unwrap_or_else(|| "?".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::ConfigTemplate;

    #[test]
    fn conditionals_and_variables() {
        let t = ConfigTemplate::new("a ${x} b${c<<\n# ${y} on>>} ${d<<off>>} ${missing} $notvar ${bad name}")
            .var("x", "X$\\1")
            .var("y", "Y")
            .conditional("c", true);
        assert_eq!(t.build(), "a X$\\1 b\n# Y on  ? $notvar ${bad name}");
    }
}
