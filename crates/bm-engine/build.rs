//! Packs `assets/resourceExtensions` into `OUT_DIR/resourceExtensions.zip`, embedded so the binary is self-contained.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

fn main() {
    // runtime var, not env!(): a build-script binary shared through one target dir would bake in another checkout's path
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let src = Path::new(&manifest).join("../../assets/resourceExtensions");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut files = Vec::new();
    walk(&src, &src, &mut files);
    files.sort();

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("resourceExtensions.zip");
    let mut zip = ZipWriter::new(File::create(&out).expect("create resourceExtensions.zip"));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default());
    for (name, path) in files {
        println!("cargo:rerun-if-changed={}", path.display());
        zip.start_file(name, options).expect("zip entry");
        zip.write_all(&std::fs::read(&path).expect("read resource extension")).expect("zip write");
    }
    zip.finish().expect("finish resourceExtensions.zip");
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("read resourceExtensions") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).expect("under root").to_string_lossy().replace('\\', "/");
            out.push((rel, path));
        }
    }
}
