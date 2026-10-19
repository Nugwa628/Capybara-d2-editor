//! Robustness: damaged files on disk (index.json, blobs, CHANGED.txt, the
//! milestone repository) must give errors, never a panic or an absurd
//! allocation.
//!
//! The mutation loops run `CAPY_FUZZ_ITERS` iterations each (default 200).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use capy_history::{HistoryError, Retention, SavesDir, SnapshotId, SnapshotKind, SnapshotStore};
use d2_assets::GamePath;
use serde_json::Value;

fn iters() -> u64 {
    let n = std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    println!("CAPY_FUZZ_ITERS = {n}");
    n
}

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Bit flips, byte overwrites, truncation, insertion, duplication.
fn mutate_bytes(rng: &mut Rng, data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    let rounds = 1 + rng.below(4);
    for _ in 0..rounds {
        match rng.below(6) {
            0 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] ^= 1 << rng.below(8);
            }
            1 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] = rng.next() as u8;
            }
            2 => {
                let n = rng.below(out.len() + 1);
                out.truncate(n);
            }
            3 => {
                let i = rng.below(out.len() + 1);
                let n = 1 + rng.below(16);
                let extra = rng.bytes(n);
                out.splice(i..i, extra);
            }
            4 if !out.is_empty() => {
                let a = rng.below(out.len());
                let b = a + rng.below(out.len() - a);
                let chunk = out[a..b].to_vec();
                out.extend_from_slice(&chunk);
            }
            _ => {
                let n = rng.below(64);
                out = rng.bytes(n);
            }
        }
    }
    out
}

/// Numbers that tend to break size and id arithmetic.
fn nasty_number(rng: &mut Rng) -> Value {
    match rng.below(10) {
        0 => Value::from(u64::MAX),
        1 => Value::from(u64::MAX - 1),
        2 => Value::from(i64::MIN),
        3 => Value::from(-1),
        4 => Value::from(0),
        5 => Value::from(1u64 << 62),
        6 => Value::from(1u64 << 40),
        7 => serde_json::json!(1.5e300),
        8 => Value::from(i64::MAX),
        _ => Value::from(rng.next()),
    }
}

/// Structural mutation of a JSON value: nasty numbers, dropped or retyped
/// fields, odd strings, duplicated array items.
fn mutate_json(rng: &mut Rng, v: &mut Value) {
    match v {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            if keys.is_empty() {
                return;
            }
            let key = keys[rng.below(keys.len())].clone();
            match rng.below(5) {
                0 => {
                    map.remove(&key);
                }
                1 => {
                    map.insert(key, nasty_number(rng));
                }
                2 => {
                    map.insert(key, Value::Null);
                }
                3 => {
                    let s = ["", "..\\..\\x", "é", "aé", "zz", "0", "\u{0}"][rng.below(7)];
                    map.insert(key, Value::from(s));
                }
                _ => {
                    if let Some(child) = map.get_mut(&key) {
                        mutate_json(rng, child);
                    }
                }
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                return;
            }
            let i = rng.below(items.len());
            match rng.below(4) {
                0 => {
                    let dup = items[i].clone();
                    items.push(dup);
                }
                1 => {
                    items.remove(i);
                }
                _ => mutate_json(rng, &mut items[i]),
            }
        }
        Value::Number(_) => *v = nasty_number(rng),
        Value::String(_) => {
            *v = Value::from(["", "é", "aé", "fff", "\\", "a\\b"][rng.below(6)]);
        }
        _ => *v = nasty_number(rng),
    }
}

fn gp(s: &str) -> GamePath {
    GamePath::new(s)
}

/// A store with a few snapshots of two maps; returns its profile folder.
fn seed_store(root: &Path) -> PathBuf {
    let mut st = SnapshotStore::open(root, "p").unwrap();
    let a = gp("data\\global\\tiles\\a.ds1");
    let b = gp("data\\global\\tiles\\b.ds1");
    st.record(&a, b"first version of a", SnapshotKind::Save)
        .unwrap();
    st.record(&b, &vec![7u8; 5000], SnapshotKind::Autosave)
        .unwrap();
    st.record(
        &a,
        b"second version of a",
        SnapshotKind::Checkpoint { name: "cp".into() },
    )
    .unwrap();
    st.record(&a, b"third", SnapshotKind::BeforeRestore)
        .unwrap();
    root.join("p")
}

/// Every read-side and write-side operation on whatever opened.
fn exercise(root: &Path) {
    let Ok(mut st) = SnapshotStore::open(root, "p") else {
        return;
    };
    let ids: Vec<SnapshotId> = st.all().iter().map(|s| s.id).collect();
    for s in st.all().to_vec() {
        let _ = st.timeline(&s.game_path);
    }
    for &id in &ids {
        match st.read(id) {
            Ok(_)
            | Err(HistoryError::Corrupt { .. })
            | Err(HistoryError::Io { .. })
            | Err(HistoryError::UnknownSnapshot(_)) => {}
            Err(e) => panic!("unexpected error kind from read: {e}"),
        }
    }
    let _ = st.read(SnapshotId(u64::MAX));
    let _ = st.record(&gp("data\\c.ds1"), b"new", SnapshotKind::Save);
    if let Some(&id) = ids.first() {
        let _ = st.set_label(id, Some("label".into()));
        let _ = st.make_checkpoint(id, "n".into());
        let _ = st.delete(id);
    }
    let _ = st.prune(&Retention {
        keep_last: 1,
        max_age_days: Some(u32::MAX),
    });
    let _ = st.prune(&Retention {
        keep_last: 0,
        max_age_days: Some(0),
    });
}

#[test]
fn corrupt_index_json_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed_store(tmp.path());
    let index = dir.join("index.json");
    let good = fs::read(&index).unwrap();
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..iters() {
        let bad = mutate_bytes(&mut rng, &good);
        fs::write(&index, &bad).unwrap();
        exercise(tmp.path());
    }
}

#[test]
fn corrupt_index_json_values() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed_store(tmp.path());
    let index = dir.join("index.json");
    let good: Value = serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();
    let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
    for _ in 0..iters() {
        let mut v = good.clone();
        for _ in 0..1 + rng.below(3) {
            mutate_json(&mut rng, &mut v);
        }
        fs::write(&index, serde_json::to_vec(&v).unwrap()).unwrap();
        exercise(tmp.path());
    }
}

#[test]
fn huge_sizes_and_ids_in_index_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed_store(tmp.path());
    let index = dir.join("index.json");
    let good: Value = serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();

    for size in [u64::MAX, u64::MAX - 1, 1 << 62, 1 << 40, 0] {
        let mut v = good.clone();
        for s in v["snapshots"].as_array_mut().unwrap() {
            s["size"] = Value::from(size);
        }
        fs::write(&index, serde_json::to_vec(&v).unwrap()).unwrap();
        let st = SnapshotStore::open(tmp.path(), "p").unwrap();
        for s in st.all() {
            assert!(
                matches!(st.read(s.id), Err(HistoryError::Corrupt { .. })),
                "size {size} must be a size mismatch"
            );
        }
    }

    // next_id at the very top: recording must fail cleanly, not overflow.
    let mut v = good.clone();
    v["next_id"] = Value::from(u64::MAX);
    v["snapshots"][0]["id"] = Value::from(u64::MAX);
    fs::write(&index, serde_json::to_vec(&v).unwrap()).unwrap();
    let mut st = SnapshotStore::open(tmp.path(), "p").unwrap();
    let before = st.all().len();
    let r = st.record(&gp("data\\z.ds1"), b"zzz", SnapshotKind::Save);
    assert!(matches!(r, Err(HistoryError::Index(_))), "{r:?}");
    assert_eq!(st.all().len(), before);

    let mut v = good;
    v["next_id"] = Value::from(u64::MAX - 1);
    fs::write(&index, serde_json::to_vec(&v).unwrap()).unwrap();
    let mut st = SnapshotStore::open(tmp.path(), "p").unwrap();
    // The id below the top is still usable; after it the ids are exhausted.
    let last = st
        .record(&gp("data\\y.ds1"), b"y1", SnapshotKind::Save)
        .unwrap();
    assert_eq!(last.id, SnapshotId(u64::MAX - 1));
    assert!(matches!(
        st.record(&gp("data\\y.ds1"), b"y2", SnapshotKind::Save),
        Err(HistoryError::Index(_))
    ));
    // Reopening the store keeps working.
    let st = SnapshotStore::open(tmp.path(), "p").unwrap();
    assert_eq!(st.read(last.id).unwrap(), b"y1");
}

fn blob_files(profile_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in fs::read_dir(profile_dir.join("objects")).unwrap().flatten() {
        for f in fs::read_dir(sub.path()).unwrap().flatten() {
            out.push(f.path());
        }
    }
    out.sort();
    out
}

#[test]
fn corrupt_blobs_are_reported_corrupt() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed_store(tmp.path());
    let blobs = blob_files(&dir);
    assert!(!blobs.is_empty());
    let originals: Vec<Vec<u8>> = blobs.iter().map(|p| fs::read(p).unwrap()).collect();
    let mut rng = Rng(0x0F0F_1234_5555_AAAA);
    for _ in 0..iters() {
        let k = rng.below(blobs.len());
        let bad = mutate_bytes(&mut rng, &originals[k]);
        let changed = bad != originals[k];
        fs::write(&blobs[k], &bad).unwrap();
        let st = SnapshotStore::open(tmp.path(), "p").unwrap();
        for s in st.all() {
            match st.read(s.id) {
                Ok(bytes) => {
                    // Only a blob that still decodes to the right content may pass.
                    assert_eq!(bytes.len() as u64, s.size);
                }
                Err(HistoryError::Corrupt { .. }) => {}
                Err(e) => panic!("unexpected error kind: {e}"),
            }
        }
        if !changed {
            continue;
        }
        fs::write(&blobs[k], &originals[k]).unwrap();
    }
    // A blob that decompresses to far more than its recorded size (a small
    // zlib "bomb") is cut off at the size and reported corrupt.
    let st = SnapshotStore::open(tmp.path(), "p").unwrap();
    let s = st.all()[0].clone();
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut enc, &vec![0u8; 8 << 20]).unwrap();
    let bomb = enc.finish().unwrap();
    let path = blobs
        .iter()
        .find(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            let prefix = p.parent().unwrap().file_name().unwrap().to_string_lossy();
            format!("{prefix}{}", name.trim_end_matches(".z")) == s.hash.0
        })
        .unwrap();
    fs::write(path, bomb).unwrap();
    assert!(matches!(st.read(s.id), Err(HistoryError::Corrupt { .. })));
    // Missing blob.
    fs::remove_file(path).unwrap();
    assert!(matches!(st.read(s.id), Err(HistoryError::Corrupt { .. })));
}

#[test]
fn corrupt_changed_txt() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = SavesDir::new(tmp.path(), "mxl");
    dir.save(&gp("data\\global\\a.ds1"), b"A").unwrap();
    dir.save(&gp("data\\global\\b.ds1"), b"B").unwrap();
    let changed = dir.root().join("CHANGED.txt");
    let good = fs::read(&changed).unwrap();
    let mut rng = Rng(0x5A5A_0000_1111_2222);
    for _ in 0..iters() {
        fs::write(&changed, mutate_bytes(&mut rng, &good)).unwrap();
        match dir.changed() {
            Ok(_) | Err(HistoryError::Io { .. }) => {}
            Err(e) => panic!("unexpected error kind: {e}"),
        }
        let _ = dir.save(&gp("data\\global\\c.ds1"), b"C");
        let _ = dir.mark_packed();
    }
}

#[cfg(feature = "milestones")]
#[test]
fn corrupt_milestone_repository() {
    use capy_history::Milestones;

    let tmp = tempfile::tempdir().unwrap();
    let saves = tmp.path().join("saves");
    let dir = SavesDir::new(&saves, "mxl");
    {
        let ms = Milestones::open_or_init(&saves).unwrap();
        dir.save(&gp("data\\global\\a.ds1"), b"A1").unwrap();
        ms.commit("one").unwrap();
        dir.save(&gp("data\\global\\a.ds1"), b"A2").unwrap();
        ms.commit("two").unwrap();
    }
    // Every small file under .git (HEAD, refs, config, loose objects).
    let mut files = Vec::new();
    let mut stack = vec![saves.join(".git")];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if fs::metadata(&p).map(|m| m.len()).unwrap_or(0) < 64 * 1024 {
                files.push(p);
            }
        }
    }
    files.sort();
    let originals: Vec<Vec<u8>> = files.iter().map(|p| fs::read(p).unwrap()).collect();
    let mut rng = Rng(0x7777_3333_9999_1111);
    // Milestone operations are slow-ish (a git repository each time).
    let n = iters().div_ceil(4);
    for i in 0..n {
        let k = rng.below(files.len());
        fs::write(&files[k], mutate_bytes(&mut rng, &originals[k])).unwrap();
        if let Ok(ms) = Milestones::open_or_init(&saves) {
            let _ = ms.log(10);
            if i % 4 == 0 {
                let _ = ms.commit("after damage");
            }
        }
        // Restore everything (a commit may have written new objects; those stay).
        for (p, b) in files.iter().zip(&originals) {
            fs::write(p, b).unwrap();
        }
    }
}

#[cfg(feature = "milestones")]
#[test]
fn deeply_nested_saves_folder_is_an_error() {
    use capy_history::Milestones;

    let tmp = tempfile::tempdir().unwrap();
    let saves = tmp.path().join("s");
    let ms = Milestones::open_or_init(&saves).unwrap();
    // 70 one-letter folders: deeper than the milestone walker follows, still
    // well inside Windows' long-path-free limit.
    let mut deep = saves.clone();
    for _ in 0..70 {
        deep.push("d");
    }
    if fs::create_dir_all(&deep).is_err() {
        println!("skipped: cannot create a deep folder here");
        return;
    }
    fs::write(deep.join("f"), b"x").unwrap();
    assert!(matches!(ms.commit("deep"), Err(HistoryError::Milestone(_))));
}
