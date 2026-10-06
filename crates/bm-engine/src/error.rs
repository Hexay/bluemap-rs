use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The Minecraft client jar is missing and `accept-download` is off (BlueMap's `MissingResourcesException`).
    #[error("BlueMap is missing important resources: {0}")]
    MissingResources(bm_resources::Error),
    #[error("failed to load resources: {0}")]
    Resources(#[from] bm_resources::Error),
    #[error(transparent)]
    Config(#[from] bm_config::ConfigError),
    /// A configuration that loaded but can't be used (BlueMap's `ConfigurationException`).
    #[error("{0}")]
    Invalid(String),
    #[error("world: {0}")]
    World(#[from] bm_world::Error),
    #[error("storage: {0}")]
    Storage(#[from] bm_storage::Error),
    #[error("render state: {0}")]
    RenderState(#[from] bm_map::renderstate::Error),
    #[error("settings: {0}")]
    Settings(#[from] bm_map::settings::SettingsError),
    #[error("render mask: {0}")]
    Mask(#[from] bm_map::mask::MaskConfigError),
    #[error("lowres: {0}")]
    Lowres(String),
    #[error("render: {0}")]
    Render(#[from] bm_render::Error),
    #[error("{op} {}: {source}", path.display())]
    Io { op: &'static str, path: PathBuf, source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(op: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { op, path, source }
}
