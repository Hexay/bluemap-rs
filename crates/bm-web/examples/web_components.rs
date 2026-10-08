//! Per-operation cost of what bm-web does per request, to attribute `serve_bench` CPU without a sampling profiler.
//!
//! `web_components <webroot> <map-id>`; prints µs/op (wall, single thread unless noted).

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bm_compress::Compression;
use bm_storage::{FileStorage, GridKey, ItemKey, MapStorage, Storage};

fn bench(name: &str, iters: u32, mut f: impl FnMut()) {
    f();
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    println!("{name:<48} {:>10.2} µs/op", t.elapsed().as_secs_f64() * 1e6 / f64::from(iters));
}

fn first_file(dir: &Path, ext: &str) -> PathBuf {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().ends_with(ext) {
                return p;
            }
        }
    }
    panic!("no {ext} under {}", dir.display())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let webroot = PathBuf::from(args.next().expect("webroot"));
    let map_id = args.next().expect("map id");
    let map_dir = webroot.join("maps").join(&map_id);
    let storage = FileStorage::new(webroot.join("maps"), Compression::Gzip).read_only(true);
    let map: std::sync::Arc<dyn MapStorage> = storage.map(&map_id).unwrap();

    let tile = first_file(&map_dir.join("tiles/0"), ".prbm.gz");
    let tile_gz = std::fs::read(&tile).unwrap();
    let missing = map_dir.join("tiles/0/x9/9/9/z9/9/9.prbm.gz");
    let settings = std::fs::read(map_dir.join("settings.json")).unwrap();
    let textures_gz = std::fs::read(map_dir.join("textures.json.gz")).unwrap();
    let js = first_file(&webroot.join("assets"), ".js");
    println!(
        "tile {} B gz, settings {} B, textures {} B gz, js {} B",
        tile_gz.len(),
        settings.len(),
        textures_gz.len(),
        std::fs::metadata(&js).unwrap().len()
    );

    bench("std::fs::read hires tile", 5000, || drop(black_box(std::fs::read(&tile).unwrap())));
    bench("std::fs::read missing (NotFound)", 5000, || {
        black_box(std::fs::read(&missing).is_err());
    });
    bench("std::fs::metadata existing", 5000, || {
        black_box(std::fs::metadata(&tile).unwrap());
    });
    bench("std::fs::metadata missing", 5000, || {
        black_box(std::fs::metadata(&missing).is_err());
    });
    bench("std::fs::File::open existing", 5000, || drop(black_box(std::fs::File::open(&tile).unwrap())));
    bench("MapStorage::read_grid hires (path+read)", 5000, || {
        drop(black_box(map.read_grid(GridKey::Hires, (0, 0)).unwrap()))
    });
    bench("MapStorage::read_item settings.json", 5000, || drop(black_box(map.read_item(&ItemKey::Settings).unwrap())));
    bench("std::fs::read 1.2 MB js", 500, || drop(black_box(std::fs::read(&js).unwrap())));

    bench("gzip compress settings.json (354 B)", 2000, || drop(black_box(Compression::Gzip.compress(&settings))));
    let tile_raw = Compression::Gzip.decompress(&tile_gz, bm_storage::MAX_DECODED).unwrap();
    let tex_raw = Compression::Gzip.decompress(&textures_gz, bm_storage::MAX_DECODED).unwrap();
    bench(&format!("gunzip hires tile ({} B raw)", tile_raw.len()), 500, || {
        drop(black_box(Compression::Gzip.decompress(&tile_gz, bm_storage::MAX_DECODED).unwrap()))
    });
    bench(&format!("gunzip textures.json ({} B raw)", tex_raw.len()), 50, || {
        drop(black_box(Compression::Gzip.decompress(&textures_gz, bm_storage::MAX_DECODED).unwrap()))
    });
    bench("gzip compress hires tile raw", 50, || drop(black_box(Compression::Gzip.compress(&tile_raw))));

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let t = Instant::now();
        for _ in 0..20000 {
            tokio::task::spawn_blocking(|| ()).await.unwrap();
        }
        println!(
            "{:<48} {:>10.2} µs/op",
            "tokio spawn_blocking no-op (sequential)",
            t.elapsed().as_secs_f64() * 1e6 / 20000.0
        );
        let t = Instant::now();
        let n = 20000u32;
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..32 {
            set.spawn(async move {
                for _ in 0..n / 32 {
                    tokio::task::spawn_blocking(|| ()).await.unwrap();
                }
            });
        }
        while set.join_next().await.is_some() {}
        println!(
            "{:<48} {:>10.2} µs/op (wall, 32 concurrent)",
            "tokio spawn_blocking no-op",
            t.elapsed().as_secs_f64() * 1e6 / f64::from(n)
        );
        let t = Instant::now();
        for _ in 0..5000 {
            black_box(tokio::fs::metadata(&tile).await.unwrap());
        }
        println!("{:<48} {:>10.2} µs/op", "tokio::fs::metadata existing", t.elapsed().as_secs_f64() * 1e6 / 5000.0);
    });
}
