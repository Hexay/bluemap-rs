/// BlueMap's namespaced key (`de.bluecolored.bluemap.core.util.Key`), e.g. `minecraft:the_nether`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    namespace: String,
    value: String,
}

impl Key {
    pub const MINECRAFT: &'static str = "minecraft";
    pub const BLUEMAP: &'static str = "bluemap";

    pub fn new(namespace: &str, value: &str) -> Self {
        Self { namespace: namespace.to_owned(), value: value.to_owned() }
    }

    /// `new Key(formatted)`: the namespace defaults to `minecraft`.
    pub fn parse(formatted: &str) -> Self {
        Self::parse_with_default(formatted, Self::MINECRAFT)
    }

    /// `Key.parse(formatted, defaultNamespace)`: splits at the first ':' unless it is the first character.
    pub fn parse_with_default(formatted: &str, default_namespace: &str) -> Self {
        match formatted.find(':') {
            Some(i) if i > 0 => Self::new(&formatted[..i], &formatted[i + 1..]),
            _ => Self::new(default_namespace, formatted),
        }
    }

    pub fn minecraft(value: &str) -> Self {
        Self::new(Self::MINECRAFT, value)
    }

    pub fn bluemap(value: &str) -> Self {
        Self::new(Self::BLUEMAP, value)
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn formatted(&self) -> String {
        format!("{}:{}", self.namespace, self.value)
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::Key;

    #[test]
    fn parsing() {
        assert_eq!(Key::parse("the_nether").formatted(), "minecraft:the_nether");
        assert_eq!(Key::parse("foo:bar:baz").formatted(), "foo:bar:baz");
        assert_eq!(Key::parse(":x").formatted(), "minecraft::x");
        assert_eq!(Key::parse_with_default("gzip", Key::BLUEMAP).formatted(), "bluemap:gzip");
    }
}
