//! Pack layering: which pack roots BlueMap loads and in what order (`common/.../BlueMapService.java` pack roots,
//! `RES/pack/Pack.java` per-root recursion, docs/02 §1). Every list here is highest priority first.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::pack_meta::{PackMeta, PackVersion};
use crate::vfs::Pack;

/// Guards overlay/nested-pack cycles, which overflow upstream's stack.
const MAX_DEPTH: u32 = 16;

/// Where [`pack_roots`] finds packs; mirrors the BlueMap config fields of the same names.
#[derive(Debug, Clone, Default)]
pub struct PackRootsConfig {
    /// Upstream creates this folder before listing it; that is left to the config layer.
    pub packs_folder: Option<PathBuf>,
    pub mods_folder: Option<PathBuf>,
    pub scan_for_mod_resources: bool,
    /// The core `data` folder; `defaultBlockstates.zip` is picked up from it whenever it exists.
    pub data_root: PathBuf,
    /// BlueMap's bundled pack (folder or zip).
    pub resource_extensions: PathBuf,
}

/// The ordered pack roots: packs folder (reverse-lexicographic), `extra_roots` (world datapacks, data packs
/// only), mod jars, `defaultBlockstates.zip`, resourceExtensions, then `client_jar` (the resource or data jar).
pub fn pack_roots(
    config: &PackRootsConfig,
    extra_roots: &[PathBuf],
    client_jar: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if let Some(folder) = config.packs_folder.as_deref().filter(|f| f.is_dir()) {
        let mut packs = list_dir(folder)?;
        packs.sort_by(|a, b| java_path_cmp(b, a));
        roots.extend(packs);
    }
    roots.extend_from_slice(extra_roots);
    if let Some(mods) = config.mods_folder.as_deref().filter(|f| config.scan_for_mod_resources && f.is_dir()) {
        // upstream keeps directory-listing order; sorted here so results don't depend on the filesystem
        let mut jars: Vec<PathBuf> = list_dir(mods)?
            .into_iter()
            .filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".jar")))
            .collect();
        jars.sort_by(|a, b| java_path_cmp(a, b));
        roots.extend(jars);
    }
    let default_blockstates = config.data_root.join("defaultBlockstates.zip");
    if default_blockstates.exists() {
        roots.push(default_blockstates);
    }
    roots.push(config.resource_extensions.clone());
    roots.push(client_jar.to_owned());
    Ok(roots)
}

fn list_dir(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect()
}

/// `java.nio.file.Path.compareTo`: case-insensitive on Windows, by bytes elsewhere.
fn java_path_cmp(a: &Path, b: &Path) -> Ordering {
    if cfg!(windows) {
        let key = |p: &Path| p.to_string_lossy().chars().flat_map(char::to_uppercase).collect::<String>();
        key(a).cmp(&key(b))
    } else {
        a.as_os_str().cmp(b.as_os_str())
    }
}

/// Pack roots opened so far, shared by several [`load_order_in`] calls: the client jar is both the resource and
/// the data root, and every open reads and indexes the whole file.
#[derive(Default)]
pub struct OpenedRoots(FxHashMap<PathBuf, Option<Pack>>);

impl OpenedRoots {
    /// Uses `pack` for the root at `path`, e.g. one opened from bytes already in memory.
    pub fn insert(&mut self, path: PathBuf, pack: Pack) {
        self.0.insert(path, Some(pack));
    }

    fn open(&mut self, path: &Path) -> Option<Pack> {
        self.0.entry(path.to_owned()).or_insert_with(|| Pack::open(path).ok()).clone()
    }
}

/// Every pack the roots expand to, in load order (see [`expand_root`]).
pub fn load_order(roots: &[PathBuf], version: PackVersion) -> Vec<Pack> {
    load_order_in(&mut OpenedRoots::default(), roots, version)
}

/// [`load_order`], reusing roots `opened` already holds.
pub fn load_order_in(opened: &mut OpenedRoots, roots: &[PathBuf], version: PackVersion) -> Vec<Pack> {
    let mut out = Vec::new();
    for pack in roots.iter().filter_map(|root| opened.open(root)) {
        expand(&pack, version, 0, &mut out);
    }
    out
}

/// One pack root (folder, zip or jar) expanded as `Pack.loadResourcePath` walks it: `fabric.mod.json` nested
/// jars, then nested datapacks (`data/*/datapacks/*`), then overlays in reverse entry order filtered by
/// `version`, then the root itself. Unreadable parts are skipped, as upstream only logs them at debug level.
pub fn expand_root(path: &Path, version: PackVersion) -> Vec<Pack> {
    load_order(std::slice::from_ref(&path.to_owned()), version)
}

fn expand(pack: &Pack, version: PackVersion, depth: u32, out: &mut Vec<Pack>) {
    if depth > MAX_DEPTH {
        return;
    }
    for jar in fabric_nested_jars(pack) {
        if pack.exists(&jar) {
            expand_entry(pack, &jar, version, depth, out);
        }
    }
    let meta = PackMeta::read(pack);
    for namespace in pack.list("data") {
        let datapacks = format!("data/{namespace}/datapacks");
        if pack.is_dir(&datapacks) {
            for nested in pack.list(&datapacks) {
                expand_entry(pack, &format!("{datapacks}/{nested}"), version, depth, out);
            }
        }
    }
    for overlay in meta.overlays.iter().rev() {
        let Some(dir) = overlay.directory.as_deref() else { continue };
        // an empty directory resolves to the root itself and recurses forever upstream
        if dir.trim_matches('/').is_empty() || !overlay.includes(version) || !pack.exists(dir) {
            continue;
        }
        expand_entry(pack, dir, version, depth, out);
    }
    out.push(pack.clone());
}

/// A path inside `parent` loaded as its own pack: a folder is re-rooted, a file is opened as a zip.
fn expand_entry(parent: &Pack, rel: &str, version: PackVersion, depth: u32, out: &mut Vec<Pack>) {
    if parent.is_dir(rel) {
        return expand(&parent.sub(rel), version, depth + 1, out);
    }
    let Some(bytes) = parent.read(rel) else { return };
    let origin: Arc<str> = format!("{}!/{}{rel}", parent.origin, parent.root()).into();
    if let Ok(nested) = Pack::zip(bytes.into(), origin) {
        expand(&nested, version, depth + 1, out);
    }
}

/// `jars[].file` of `fabric.mod.json`. Upstream stops at the first malformed entry, keeping the earlier ones.
fn fabric_nested_jars(pack: &Pack) -> Vec<String> {
    let Some(src) = pack.read_string("fabric.mod.json") else { return Vec::new() };
    let Ok(root) = crate::json::parse(&src) else { return Vec::new() };
    let Some(Value::Array(jars)) = root.get("jars") else { return Vec::new() };
    jars.iter().map_while(|j| j.get("file").and_then(Value::as_str).map(str::to_owned)).collect()
}

#[cfg(test)]
#[path = "packs_tests.rs"]
mod tests;
