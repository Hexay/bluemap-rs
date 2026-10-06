//! Mojang's version manifest and the client-jar download (`RES/VersionManifest.java`, `MinecraftVersion.download`).

use std::cmp::Ordering;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha1::{Digest, Sha1};

use crate::{Error, Result};

pub const DOMAIN: &str = "https://piston-meta.mojang.com/";
pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub time: String,
    pub release_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Download {
    pub url: String,
    #[serde(default)]
    pub size: u64,
    pub sha1: String,
}

#[derive(Deserialize)]
struct VersionDetail {
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    client: Download,
}

impl VersionManifest {
    pub fn fetch() -> Result<Self> {
        Self::parse(&get_string(MANIFEST_URL)?)
    }

    /// Parses and validates: every version url must be on [`DOMAIN`] and no id may escape the data folder.
    pub fn parse(src: &str) -> Result<Self> {
        let manifest: Self = crate::json::from_str(src)?;
        validate_id(&manifest.latest.release)?;
        validate_id(&manifest.latest.snapshot)?;
        for v in &manifest.versions {
            if !v.url.starts_with(DOMAIN) {
                return Err(Error::Manifest(format!("Invalid version manifest URL: {}", v.url)));
            }
            validate_id(&v.id)?;
        }
        Ok(manifest)
    }

    pub fn version(&self, id: &str) -> Result<&ManifestVersion> {
        // upstream builds a HashMap, so the last duplicate id wins
        self.versions
            .iter()
            .rev()
            .find(|v| v.id == id)
            .ok_or_else(|| Error::Manifest(format!("There is no version '{id}' in manifest.")))
    }
}

impl ManifestVersion {
    /// Orders by `releaseTime` as a local date-time: upstream parses it into `LocalDateTime`, dropping the offset.
    pub fn cmp_release(&self, other: &Self) -> Ordering {
        local_date_time(&self.release_time).cmp(local_date_time(&other.release_time))
    }

    pub fn fetch_client_download(&self) -> Result<Download> {
        let detail: VersionDetail = crate::json::from_str(&get_string(&self.url)?)?;
        Ok(detail.downloads.client)
    }
}

fn local_date_time(iso: &str) -> &str {
    let Some(t) = iso.find('T') else { return iso };
    let end = iso[t..].find(['+', '-', 'Z']).map_or(iso.len(), |i| t + i);
    &iso[..end]
}

pub fn validate_id(id: &str) -> Result<()> {
    if id.contains('/') || id.contains("..") || id.contains('\\') {
        return Err(Error::Manifest(format!("Invalid version manifest ID: {id}")));
    }
    Ok(())
}

/// Downloads `version`'s client jar to `file` via `<file>.unverified`, moved into place once its SHA-1 matches.
pub fn download_client(version: &ManifestVersion, file: &Path) -> Result<()> {
    let download = version.fetch_client_download()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let response = agent().get(&download.url).call().map_err(|e| http_err(&download.url, e))?;
    write_verified(response.into_body().into_reader(), file, &download.sha1)
}

/// Streams `reader` to `<file>.unverified`, checks its SHA-1 against `sha1_hex`, then renames it to `file`.
pub(crate) fn write_verified(mut reader: impl Read, file: &Path, sha1_hex: &str) -> Result<()> {
    let mut name = file.file_name().unwrap_or_default().to_owned();
    name.push(".unverified");
    let unverified = file.with_file_name(name);
    let result = (|| {
        let mut out = File::create(&unverified)?;
        let mut hasher = Sha1::new();
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
        }
        out.sync_all()?;
        drop(out);
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !actual.eq_ignore_ascii_case(sha1_hex) {
            return Err(Error::Checksum { expected: sha1_hex.to_owned(), actual });
        }
        std::fs::rename(&unverified, file)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&unverified);
    result
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build();
    // upstream's 10 s read timeout is per read; ureq only has whole-body timeouts, so the body stays unbounded
    ureq::config::Config::builder()
        .tls_config(tls)
        .timeout_connect(Some(TIMEOUT))
        .timeout_recv_response(Some(TIMEOUT))
        .build()
        .new_agent()
}

fn get_string(url: &str) -> Result<String> {
    let mut response = agent().get(url).call().map_err(|e| http_err(url, e))?;
    response.body_mut().read_to_string().map_err(|e| http_err(url, e))
}

fn http_err(url: &str, e: ureq::Error) -> Error {
    Error::Http(format!("{url}: {e}"))
}
