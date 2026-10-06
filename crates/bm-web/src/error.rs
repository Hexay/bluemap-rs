use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error(
        "failed to resolve the webserver ip {ip:?}: {source}\nCheck if `ip` in webserver.conf is correctly configured."
    )]
    Resolve { ip: String, source: io::Error },
    #[error(
        "failed to bind the webserver to {addr}: {source}\nThis usually happens when the configured port is already in \
         use by some other program (don't use your Minecraft server's port), or the ip is not an address of this machine."
    )]
    Bind { addr: std::net::SocketAddr, source: io::Error },
    #[error("invalid webserver port {0}: must be 0..=65535")]
    Port(i32),
    #[error(transparent)]
    Format(#[from] crate::javafmt::FormatError),
    #[error("webserver log format uses argument {0}, but only 1..=7 exist")]
    LogFormatArgs(usize),
    #[error("webserver log file {path}: {source}")]
    LogFile { path: PathBuf, source: io::Error },
    #[error("invalid additional header {name:?}: {reason}")]
    Header { name: String, reason: &'static str },
    #[error("invalid map id {0:?}")]
    MapId(String),
    #[error("{op} {path}: {source}")]
    Io { op: &'static str, path: PathBuf, source: io::Error },
    #[error(transparent)]
    Settings(#[from] bm_map::settings::SettingsError),
    #[error("webserver failed: {0}")]
    Serve(io::Error),
}
