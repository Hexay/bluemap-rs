//! Java addons (`AddonLoader.tryLoadAddons`): `packs/*.jar` holding a `bluemap.addon.json`. They hook core
//! registries through Java classloading, so a Rust core can only report them; their jars still load as resource
//! packs because `packs/` is a pack root.

use std::path::{Path, PathBuf};

use bm_resources::Pack;

const ADDON_INFO: &str = "bluemap.addon.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaAddon {
    /// `id` from `bluemap.addon.json`, or the file name when it has none.
    pub id: String,
    pub file: PathBuf,
}

impl JavaAddon {
    pub fn warning(&self) -> String {
        format!(
            "Java addon '{}' ({}) can not run on bluemap-rs and is skipped; resource packs it bundles still load.",
            self.id,
            self.file.display()
        )
    }
}

/// Every Java addon jar directly in `packs`, sorted by file name; a missing folder has none.
pub fn find_java_addons(packs: &Path) -> Vec<JavaAddon> {
    let Ok(entries) = std::fs::read_dir(packs) else { return Vec::new() };
    let mut jars: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jar"))
        .collect();
    jars.sort();
    jars.into_iter().filter_map(|file| addon_info(&file).map(|id| JavaAddon { id, file })).collect()
}

fn addon_info(jar: &Path) -> Option<String> {
    let json = Pack::open(jar).ok()?.read(ADDON_INFO)?;
    let id = serde_json::from_slice::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_owned));
    Some(id.unwrap_or_else(|| jar.file_name().unwrap_or_default().to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn jar(path: &Path, entries: &[(&str, &str)]) {
        let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in entries {
            zip.start_file(*name, stored).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn finds_only_jars_with_addon_info() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        jar(&p.join("b-linear.jar"), &[(ADDON_INFO, r#"{"id": "bluemap-linear", "entrypoint": "x.Y"}"#)]);
        jar(&p.join("a-noid.jar"), &[(ADDON_INFO, "{}")]);
        jar(&p.join("pack.jar"), &[("pack.mcmeta", "{}")]);
        std::fs::write(p.join("broken.jar"), b"not a zip").unwrap();
        std::fs::write(p.join("notes.json"), b"{}").unwrap();
        std::fs::create_dir(p.join("folder.jar")).unwrap();

        let found = find_java_addons(p);
        let ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["a-noid.jar", "bluemap-linear"]);
        assert!(found[1].warning().contains("'bluemap-linear'"));
    }

    #[test]
    fn missing_folder_has_none() {
        assert!(find_java_addons(Path::new("does/not/exist")).is_empty());
    }
}
