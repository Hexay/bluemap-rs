use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

use super::*;

fn store() -> (tempfile::TempDir, BundleStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = BundleStore::new(dir.path().join("hires"), false);
    (dir, store)
}

fn bundle_files(store: &BundleStore) -> usize {
    fs::read_dir(&store.dir).map_or(0, |d| d.count())
}

#[test]
fn tiles_land_in_region_bundles() {
    let (_dir, s) = store();
    for t in [(0, 0), (15, 15), (16, 0), (-1, -1), (-16, 3)] {
        s.write(t, format!("{t:?}").as_bytes()).unwrap();
    }
    assert_eq!(bundle_files(&s), 4, "(0,0)+(15,15) | (16,0) | (-1,-1) | (-16,3)");
    for t in [(0, 0), (15, 15), (16, 0), (-1, -1), (-16, 3)] {
        assert_eq!(s.read(t).unwrap().unwrap(), format!("{t:?}").as_bytes());
        assert!(s.exists(t).unwrap());
    }
    assert_eq!(s.read((1, 1)).unwrap(), None);
    assert_eq!(s.read((100, 100)).unwrap(), None);
    let mut all = s.list().unwrap();
    all.sort();
    assert_eq!(all, vec![(-16, 3), (-1, -1), (0, 0), (15, 15), (16, 0)]);
}

#[test]
fn overwrite_delete_and_empty_bundles_vanish() {
    let (_dir, s) = store();
    s.write((1, 2), b"a").unwrap();
    s.write((1, 2), b"bb").unwrap();
    assert_eq!(s.read((1, 2)).unwrap().unwrap(), b"bb");
    s.delete((3, 3)).unwrap();
    s.delete((1, 2)).unwrap();
    assert_eq!(s.read((1, 2)).unwrap(), None);
    assert_eq!(bundle_files(&s), 0);
    s.delete((1, 2)).unwrap();
}

#[test]
fn another_reader_sees_appends_and_compactions() {
    let (dir, s) = store();
    let other = BundleStore::new(dir.path().join("hires"), true);
    let big = vec![7u8; 300 << 10];
    s.write((0, 0), &big).unwrap();
    assert_eq!(other.read((0, 0)).unwrap().unwrap(), big);
    for i in 0..8u8 {
        s.write((0, 0), &vec![i; 300 << 10]).unwrap();
        assert_eq!(other.read((0, 0)).unwrap().unwrap()[0], i);
    }
    let len = fs::metadata(s.path((0, 0))).unwrap().len();
    assert!(len < 3 * (300 << 10), "compacted: {len}");
    assert!(other.write((0, 0), b"x").is_err());
}

#[test]
fn versions_follow_each_tiles_record() {
    let (dir, s) = store();
    let other = BundleStore::new(dir.path().join("hires"), true);
    assert_eq!(s.version((1, 1)).unwrap(), None);
    s.write((1, 1), b"tile").unwrap();
    let v1 = s.version((1, 1)).unwrap().unwrap();
    assert_eq!(other.version((1, 1)).unwrap(), Some(v1));
    assert_eq!(other.read_versioned((1, 1)).unwrap(), Some((b"tile".to_vec(), Some(v1))));
    s.write((2, 2), b"neighbour").unwrap();
    assert_eq!(other.version((1, 1)).unwrap(), Some(v1), "a neighbour's write keeps this tile's version");
    s.write((1, 1), b"tile").unwrap();
    let v2 = other.version((1, 1)).unwrap().unwrap();
    assert_ne!(v1, v2, "a rewrite of equal bytes is a new version");
    for i in 0..4u8 {
        s.write((2, 2), &vec![i; 600 << 10]).unwrap();
    }
    assert_ne!(other.version((1, 1)).unwrap(), Some(v2), "compaction moved the record");
    assert_eq!(other.read((1, 1)).unwrap().unwrap(), b"tile");
    s.delete((1, 1)).unwrap();
    assert_eq!(other.version((1, 1)).unwrap(), None);
}

#[test]
fn torn_tail_is_ignored_then_truncated() {
    let (_dir, s) = store();
    s.write((2, 2), b"good").unwrap();
    let path = s.path((0, 0));
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(&[1, 3, 3, 0, 200, 0, 0, 0, 1, 2]).unwrap();
    drop(f);
    let fresh = BundleStore::new(s.dir.clone(), false);
    assert_eq!(fresh.read((2, 2)).unwrap().unwrap(), b"good");
    assert_eq!(fresh.read((3, 3)).unwrap(), None);
    fresh.write((3, 3), b"new").unwrap();
    let again = BundleStore::new(s.dir.clone(), true);
    assert_eq!(again.read((3, 3)).unwrap().unwrap(), b"new");
    assert_eq!(again.read((2, 2)).unwrap().unwrap(), b"good");
}

#[test]
fn torn_tail_with_a_whole_payload_is_never_served() {
    let (_dir, s) = store();
    s.write((2, 2), b"good").unwrap();
    s.release();
    let path = s.path((0, 0));
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    // a put of (2,2) whose payload landed but whose checksum (last 4 head bytes) is wrong
    f.write_all(&[1, 2, 2, 0, 3, 0, 0, 0, 9, 9, 9, 9, b'b', b'a', b'd']).unwrap();
    drop(f);
    let reader = BundleStore::new(s.dir.clone(), true);
    assert!(reader.exists((2, 2)).unwrap());
    assert_eq!(reader.read((2, 2)).unwrap().unwrap(), b"good");
    let writer = BundleStore::new(s.dir.clone(), false);
    assert_eq!(writer.read((2, 2)).unwrap().unwrap(), b"good");
    writer.write((3, 3), b"new").unwrap();
    let again = BundleStore::new(s.dir.clone(), true);
    assert_eq!(
        (again.read((2, 2)).unwrap().unwrap(), again.read((3, 3)).unwrap().unwrap()),
        (b"good".to_vec(), b"new".to_vec())
    );
}

#[test]
fn a_full_cache_drops_least_recently_used_bundles() {
    let (_dir, s) = store();
    let n = cache::CACHE_CAP as i32 + 8;
    for b in 0..n {
        s.write((b << 4, 0), &b.to_le_bytes()).unwrap();
    }
    s.release();
    for b in 0..n {
        assert_eq!(s.read((b << 4, 0)).unwrap().unwrap(), b.to_le_bytes());
        // bundle 0 stays in use, so eviction never picks it
        s.read((0, 0)).unwrap();
    }
    let cached = lock(&s.cache).get((0, 0));
    assert!(cached.is_some_and(|b| lock(&b).ix.generation != 0), "the hot bundle survived the evictions");
}

fn sidecar_index(s: &BundleStore) -> bool {
    let state = s.bundle((0, 0));
    lock(&state).ix.sidecar
}

#[test]
fn flush_leaves_a_sidecar_that_readers_start_from() {
    let (dir, s) = store();
    s.write((1, 2), b"one").unwrap();
    s.write((1, 2), b"two").unwrap();
    s.write((3, 4), b"x").unwrap();
    assert!(!s.sidecar_path((0, 0)).exists(), "none while a writer appends");
    s.flush();
    assert!(s.sidecar_path((0, 0)).exists());
    let reader = BundleStore::new(dir.path().join("hires"), true);
    assert_eq!(reader.read((1, 2)).unwrap().unwrap(), b"two");
    assert!(sidecar_index(&reader));
    let mut all = reader.list().unwrap();
    all.sort();
    assert_eq!(all, vec![(1, 2), (3, 4)]);

    s.write((5, 5), b"later").unwrap();
    assert!(!s.sidecar_path((0, 0)).exists(), "the next append deletes it");
    assert_eq!(reader.read((5, 5)).unwrap().unwrap(), b"later");
}

#[test]
fn records_appended_after_a_sidecar_are_scanned() {
    let (dir, s) = store();
    s.write((1, 2), b"two").unwrap();
    s.flush();
    // an older version appends without knowing about sidecars
    let mut tail = Vec::new();
    log::record(&mut tail, true, (6, 6), b"old version");
    log::record(&mut tail, false, (1, 2), b"");
    OpenOptions::new().append(true).open(s.path((0, 0))).unwrap().write_all(&tail).unwrap();
    let reader = BundleStore::new(dir.path().join("hires"), true);
    assert_eq!(reader.read((6, 6)).unwrap().unwrap(), b"old version");
    assert_eq!(reader.read((1, 2)).unwrap(), None);
    assert!(sidecar_index(&reader));
}

#[test]
fn a_sidecar_that_disagrees_with_the_log_is_dropped() {
    let (dir, s) = store();
    s.write((1, 2), b"two").unwrap();
    s.write((3, 4), b"four").unwrap();
    s.flush();
    let (generation, end, mut entries) = {
        let state = s.bundle((0, 0));
        let b = lock(&state);
        (b.ix.generation, b.ix.end, b.ix.entries.clone())
    };
    let (a, b) = (entries[&(1, 2)], entries[&(3, 4)]);
    entries.insert((1, 2), b);
    entries.insert((3, 4), a);
    let wrong = Index { entries, end, ..Index::fresh(generation) };
    fs::write(s.sidecar_path((0, 0)), sidecar::encode(&wrong).unwrap()).unwrap();
    let reader = BundleStore::new(dir.path().join("hires"), true);
    assert_eq!(reader.read((1, 2)).unwrap().unwrap(), b"two");
    assert_eq!(reader.read((3, 4)).unwrap().unwrap(), b"four");
    assert!(!sidecar_index(&reader), "rescanned");
    assert_eq!(lock(&reader.bundle((0, 0))).distrust, generation);
}

#[test]
fn compaction_leaves_a_sidecar_of_the_new_file() {
    let (dir, s) = store();
    // the third write leaves 1.2 MB dead against 0.6 MB live: compaction, and no writer open after it
    for i in 0..3u8 {
        s.write((2, 2), &vec![i; 600 << 10]).unwrap();
    }
    let generation = lock(&s.bundle((0, 0))).ix.generation;
    let len = fs::metadata(s.path((0, 0))).unwrap().len();
    assert!(len < 2 * (600 << 10), "compacted: {len}");
    assert!(sidecar::load(&s.sidecar_path((0, 0)), generation, len).is_some());
    let reader = BundleStore::new(dir.path().join("hires"), true);
    assert_eq!(reader.read((2, 2)).unwrap().unwrap()[0], 2);
    assert!(sidecar_index(&reader));
}

#[test]
#[ignore = "timing probe"]
fn probe_write_costs() {
    let (dir, s) = store();
    let blob = vec![5u8; 10_000];
    let t = std::time::Instant::now();
    for i in 0..256 {
        s.write((i % 16, i / 16), &blob).unwrap();
    }
    eprintln!("bundle appends: {:?}/tile", t.elapsed() / 256);
    let t = std::time::Instant::now();
    for i in 0..256 {
        fsops::write_atomic(&dir.path().join(format!("f/x{i}.prbm.gz")), &blob).unwrap();
    }
    eprintln!("atomic files: {:?}/tile", t.elapsed() / 256);
    let t = std::time::Instant::now();
    for i in 0..256 {
        s.read((i % 16, i / 16)).unwrap().unwrap();
    }
    eprintln!("bundle reads: {:?}/tile", t.elapsed() / 256);
    // what the held write handle avoids (see the module docs)
    let raw = dir.path().join("raw.bin");
    let t = std::time::Instant::now();
    for _ in 0..256 {
        let mut f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&raw).unwrap();
        let len = f.metadata().unwrap().len();
        f.seek(SeekFrom::Start(len)).unwrap();
        f.write_all(&blob).unwrap();
    }
    eprintln!("open-append-close: {:?}/tile", t.elapsed() / 256);
}

#[test]
fn concurrent_writers_and_readers() {
    let (_dir, s) = store();
    let s = Arc::new(s);
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let s = s.clone();
            std::thread::spawn(move || {
                for i in 0..64 {
                    let tile = ((i % 32) as i32, t);
                    s.write(tile, &vec![t as u8; 1000 + i]).unwrap();
                    let back = s.read(tile).unwrap().unwrap();
                    assert!(back.iter().all(|&b| b == t as u8));
                }
            })
        })
        .collect();
    handles.into_iter().for_each(|h| h.join().unwrap());
    assert_eq!(s.list().unwrap().len(), 8 * 32);
}
