//! The bundled BlueMap webapp (see `NOTICE`) and the webroot setup BlueMap does on start
//! (`BlueMapService.createOrUpdateWebApp`, `WebFilesManager`).

use std::borrow::Cow;
use std::io::ErrorKind;
use std::path::Path;

use bm_map::settings::{WebappConfig, WebappSettings};
use bytes::Bytes;
use rust_embed::RustEmbed;

use crate::WebError;
use crate::paths::join;

/// BlueMap release the embedded webapp (and our `Server` header) comes from.
pub const WEBAPP_VERSION: &str = "5.28";

#[derive(RustEmbed)]
#[folder = "webapp/"]
struct Webapp;

pub(crate) struct EmbeddedFile {
    pub data: Bytes,
    pub last_modified_ms: i64,
}

/// A bundled file by `/`-separated path relative to the webroot.
pub(crate) fn embedded_file(rel: &str) -> Option<EmbeddedFile> {
    let file = Webapp::get(rel)?;
    let data = match file.data {
        Cow::Borrowed(b) => Bytes::from_static(b),
        Cow::Owned(v) => Bytes::from(v),
    };
    let secs = file.metadata.last_modified().unwrap_or(0);
    Some(EmbeddedFile { data, last_modified_ms: i64::try_from(secs).unwrap_or(0) * 1000 })
}

pub(crate) fn embedded_is_dir(rel: &str) -> bool {
    rel.is_empty() || Webapp::iter().any(|f| f.strip_prefix(rel).is_some_and(|r| r.starts_with('/')))
}

/// Paths of all bundled files.
pub fn webapp_files() -> impl Iterator<Item = Cow<'static, str>> {
    Webapp::iter()
}

/// `filesNeedUpdate` + `updateFiles`: when `force` or `index.html` is missing, writes every bundled file into
/// `webroot`, replacing same-named files. Everything else (root `settings.json`, custom scripts/styles, `maps/`)
/// is left alone. Returns whether files were written.
pub fn install_webapp(webroot: &Path, force: bool) -> Result<bool, WebError> {
    if !force && webroot.join("index.html").is_file() {
        return Ok(false);
    }
    for rel in Webapp::iter() {
        let file = Webapp::get(&rel).expect("listed embedded file");
        let path = join(webroot, &rel);
        let io = |op, source| WebError::Io { op, path: path.clone(), source };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io("create dir for", e))?;
        }
        let mut part = path.clone().into_os_string();
        part.push(".filepart");
        std::fs::write(&part, &file.data).map_err(|e| io("write", e))?;
        std::fs::rename(&part, &path).map_err(|e| io("replace", e))?;
    }
    Ok(true)
}

/// Rewrites the webroot `settings.json` from `config` and the maps (id, sorting), or with
/// `update-settings-file: false` merges into the existing file. `version` is BlueMap's (e.g. [`WEBAPP_VERSION`]).
pub fn write_settings(
    webroot: &Path,
    version: &str,
    config: &WebappConfig,
    maps: &[(&str, i32)],
) -> Result<(), WebError> {
    let path = webroot.join("settings.json");
    let io = |op, source| WebError::Io { op, path: path.clone(), source };
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(io("read", e)),
    };
    let json = WebappSettings::create_or_update(version, config, maps, existing.as_deref())?.to_json()?;
    std::fs::create_dir_all(webroot).map_err(|e| io("create dir for", e))?;
    std::fs::write(&path, json).map_err(|e| io("write", e))
}
