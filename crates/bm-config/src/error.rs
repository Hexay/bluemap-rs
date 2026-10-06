use std::path::PathBuf;

/// HOCON syntax or substitution error, positioned in its source (`origin` is the file path or a label).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{origin}:{line}:{col}: {message}")]
pub struct ParseError {
    pub origin: String,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read {}: {source}", path.display())]
    Read { path: PathBuf, source: std::io::Error },
    #[error("failed to write {}: {source}", path.display())]
    Write { path: PathBuf, source: std::io::Error },
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// A value has the wrong type or is out of range (Configurate `SerializationException`).
    #[error("{}: invalid value at '{key}': {message}", file.display())]
    Value { file: PathBuf, key: String, message: String },
    /// The file parsed and mapped, but its content is not a valid configuration.
    #[error("{}: {message}", file.display())]
    Invalid { file: PathBuf, message: String },
}

impl ConfigError {
    pub(crate) fn invalid(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        ConfigError::Invalid { file: file.into(), message: message.into() }
    }
}
