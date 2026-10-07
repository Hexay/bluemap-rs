//! Which vanilla client jars BlueMap uses and their pack formats (`RES/MinecraftVersion.java`, docs/02 §1).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::manifest::{self, ManifestVersion, VersionManifest};
use crate::pack_meta::{PackVersion, gson_int};
use crate::vfs::Pack;
use crate::{Error, Result};

pub const EARLIEST_RESOURCEPACK_VERSION: &str = "1.13";
pub const EARLIEST_DATAPACK_VERSION: &str = "1.19.4";

/// The `pack_version` of a client jar's `version.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackVersions {
    pub resource: PackVersion,
    pub data: PackVersion,
}

impl Default for PackVersions {
    /// What upstream assumes without a `version.json` (1.13–1.14.4).
    fn default() -> Self {
        Self { resource: PackVersion::new(4, 0), data: PackVersion::new(4, 0) }
    }
}

impl PackVersions {
    /// `pack_version` is an int (resource major only; data stays 4) or
    /// `{resource_major|resource, resource_minor, data_major|data, data_minor}` with defaults 4/0/4/0.
    pub fn parse_version_json(src: &str) -> Result<Self> {
        let root = crate::json::parse(src)?;
        let mut out = Self::default();
        match root.get("pack_version").unwrap_or(&Value::Null) {
            Value::Null => {}
            v @ Value::Number(_) => out.resource.major = gson_int(v)?,
            Value::Object(o) => {
                let int = |names: &[&str]| {
                    names.iter().find_map(|n| o.get(*n).filter(|v| !v.is_null())).map(gson_int).transpose()
                };
                out.resource.major = int(&["resource_major", "resource"])?.unwrap_or(4);
                out.resource.minor = int(&["resource_minor"])?.unwrap_or(0);
                out.data.major = int(&["data_major", "data"])?.unwrap_or(4);
                out.data.minor = int(&["data_minor"])?.unwrap_or(0);
            }
            v => return Err(Error::Pack(format!("invalid pack_version: {v}"))),
        }
        Ok(out)
    }

    /// Reads `version.json` from a client jar; a jar without one gets [`PackVersions::default`].
    pub fn read(jar: &Path) -> Result<Self> {
        let json = if jar.is_dir() { Pack::open(jar)?.read_string("version.json") } else { zip_entry_string(jar, "version.json")? };
        json.map_or(Ok(Self::default()), |s| Self::parse_version_json(&s))
    }
}

/// One entry of a zip on disk, reading only its directory and that entry (a client jar is ~40 MB).
fn zip_entry_string(zip: &Path, name: &str) -> Result<Option<String>> {
    let file = std::io::BufReader::new(std::fs::File::open(zip)?);
    let mut archive = zip::ZipArchive::new(file).map_err(|e| Error::Pack(format!("{}: {e}", zip.display())))?;
    let Ok(mut entry) = archive.by_name(name) else { return Ok(None) };
    let mut s = String::new();
    Ok(std::io::Read::read_to_string(&mut entry, &mut s).ok().map(|_| s))
}

/// `<data>/minecraft-client-<id>.jar`, rejecting ids that could leave the data folder.
pub fn client_jar_path(data_root: &Path, id: &str) -> Result<PathBuf> {
    manifest::validate_id(id)?;
    Ok(data_root.join(format!("minecraft-client-{id}.jar")))
}

/// The configured version and the manifest entries of the jars its resources and datapacks come from.
#[derive(Debug, Clone)]
pub struct JarSelection {
    pub id: String,
    /// `max(version, 1.13)` by release time.
    pub resource: ManifestVersion,
    /// `max(version, 1.19.4)` by release time.
    pub data: ManifestVersion,
}

/// `id` `None` means `latest.release`.
pub fn select_jars(manifest: &VersionManifest, id: Option<&str>) -> Result<JarSelection> {
    let id = id.map_or_else(|| manifest.latest.release.clone(), str::to_owned);
    let version = manifest.version(&id)?;
    let at_least = |floor: &str| -> Result<ManifestVersion> {
        let floor = manifest.version(floor)?;
        Ok(if version.cmp_release(floor) == Ordering::Greater { version } else { floor }.clone())
    };
    Ok(JarSelection {
        resource: at_least(EARLIEST_RESOURCEPACK_VERSION)?,
        data: at_least(EARLIEST_DATAPACK_VERSION)?,
        id,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftVersion {
    pub id: String,
    pub resource_pack: PathBuf,
    pub resource_pack_version: PackVersion,
    pub data_pack: PathBuf,
    pub data_pack_version: PackVersion,
}

impl MinecraftVersion {
    /// Fetches the manifest, then see [`MinecraftVersion::load_with`].
    pub fn load(id: Option<&str>, data_root: &Path, accept_download: bool) -> Result<Self> {
        Self::load_with(VersionManifest::fetch(), id, data_root, accept_download)
    }

    /// Resolves and (if `accept_download`) downloads the client jars into `data_root`. Without a manifest a
    /// configured `id` falls back to an existing local jar for both packs. A jar that fails to open is deleted
    /// (only when downloads are accepted) so the next start downloads it again.
    pub fn load_with(
        manifest: Result<VersionManifest>,
        id: Option<&str>,
        data_root: &Path,
        accept_download: bool,
    ) -> Result<Self> {
        let resolved = manifest.and_then(|m| {
            let sel = select_jars(&m, id)?;
            Ok((client_jar_path(data_root, &sel.resource.id)?, client_jar_path(data_root, &sel.data.id)?, sel))
        });
        let (id, resource_pack, data_pack) = match resolved {
            Ok((resource_pack, data_pack, sel)) => {
                if accept_download {
                    for (version, file) in [(&sel.resource, &resource_pack), (&sel.data, &data_pack)] {
                        if !file.exists() {
                            manifest::download_client(version, file)?;
                        }
                    }
                }
                (sel.id, resource_pack, data_pack)
            }
            Err(e) => {
                let Some(id) = id else { return Err(e) };
                let jar = client_jar_path(data_root, id)?;
                (id.to_owned(), jar.clone(), jar)
            }
        };
        for jar in [&resource_pack, &data_pack] {
            if !jar.exists() {
                return Err(if accept_download {
                    Error::ResourceFileMissing(jar.clone())
                } else {
                    Error::DownloadNotAccepted(jar.clone())
                });
            }
        }
        let versions = PackVersions::read(&resource_pack).and_then(|r| {
            let d = if resource_pack == data_pack { r } else { PackVersions::read(&data_pack)? };
            Ok((r.resource, d.data))
        });
        match versions {
            Ok((resource_pack_version, data_pack_version)) => {
                Ok(Self { id, resource_pack, resource_pack_version, data_pack, data_pack_version })
            }
            Err(e) => {
                // upstream deletes only on IOException (corrupt zip), not on a malformed version.json
                if accept_download && matches!(e, Error::Io(_) | Error::Pack(_)) {
                    let _ = std::fs::remove_file(&resource_pack);
                    let _ = std::fs::remove_file(&data_pack);
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
#[path = "client_jar_tests.rs"]
mod tests;
