//! `BlueMapService.updateDefaultBlockstatesPack`: the server's default block states (from `Hello`) as
//! `<data>/defaultBlockstates.zip`, one `data/<namespace>/defaultBlockstates.json` per namespace.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use zip::write::SimpleFileOptions;

/// `dump`: JSON object `{"minecraft:stone": "minecraft:stone", …}`; empty = the server sent none (file kept).
pub fn write_pack(data: &Path, dump: &[u8]) -> Result<()> {
    if dump.is_empty() {
        return Ok(());
    }
    let states: BTreeMap<String, String> = serde_json::from_slice(dump).context("default blockstates dump")?;
    let mut by_namespace: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    for (key, state) in &states {
        let namespace = key.split_once(':').map_or("minecraft", |(ns, _)| ns);
        by_namespace.entry(namespace).or_default().insert(key, state);
    }
    let file = data.join("defaultBlockstates.zip");
    std::fs::create_dir_all(data).with_context(|| format!("create {}", data.display()))?;
    let write = || -> Result<()> {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file)?);
        for (namespace, states) in by_namespace {
            zip.start_file(format!("data/{namespace}/defaultBlockstates.json"), SimpleFileOptions::default())?;
            zip.write_all(serde_json::to_string_pretty(&states)?.as_bytes())?;
        }
        zip.finish()?;
        Ok(())
    };
    write().with_context(|| {
        format!("Failed to create {}! Does BlueMap have sufficient write permissions?", file.display())
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn one_json_per_namespace() {
        let dir = std::env::temp_dir().join(format!("bm-blockstates-{}", std::process::id()));
        let dump = br#"{"minecraft:stone":"minecraft:stone","minecraft:oak_door":"minecraft:oak_door[facing=north]","mod:x":"mod:x[a=1]"}"#;
        write_pack(&dir, dump).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(dir.join("defaultBlockstates.zip")).unwrap()).unwrap();
        let mut json = String::new();
        zip.by_name("data/minecraft/defaultBlockstates.json").unwrap().read_to_string(&mut json).unwrap();
        assert_eq!(
            json,
            "{\n  \"minecraft:oak_door\": \"minecraft:oak_door[facing=north]\",\n  \"minecraft:stone\": \"minecraft:stone\"\n}"
        );
        assert!(zip.by_name("data/mod/defaultBlockstates.json").is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }
}
