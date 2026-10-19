#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use capy_history::{HistoryError, Retention, SnapshotId, SnapshotKind, SnapshotStore};
use d2_assets::GamePath;

fn gp(s: &str) -> GamePath {
    GamePath::new(s)
}

fn blob_files(profile_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let objects = profile_dir.join("objects");
    let Ok(rd) = std::fs::read_dir(&objects) else {
        return out;
    };
    for sub in rd.flatten() {
        for f in std::fs::read_dir(sub.path()).unwrap().flatten() {
            out.push(f.path());
        }
    }
    out
}

#[test]
fn dedupe_timeline_and_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let a = gp("data\\a.ds1");
    let b = gp("data\\b.ds1");
    let (s1, s2, s3, s4);
    {
        let mut st = SnapshotStore::open(tmp.path(), "mxl").unwrap();
        assert!(st.all().is_empty());
        s1 = st.record(&a, b"one", SnapshotKind::Save).unwrap();
        // Same bytes, Save or Autosave: deduped, same snapshot returned.
        let again = st.record(&a, b"one", SnapshotKind::Autosave).unwrap();
        assert_eq!(again, s1);
        let again = st.record(&a, b"one", SnapshotKind::Save).unwrap();
        assert_eq!(again, s1);
        assert_eq!(st.all().len(), 1);
        s2 = st.record(&b, b"one", SnapshotKind::Save).unwrap(); // other path: new snapshot
        assert_ne!(s2.id, s1.id);
        assert_eq!(s2.hash, s1.hash);
        s3 = st.record(&a, b"two", SnapshotKind::Autosave).unwrap();
        // BeforeRestore and Checkpoint are never deduped.
        s4 = st.record(&a, b"two", SnapshotKind::BeforeRestore).unwrap();
        assert_ne!(s4.id, s3.id);
        let cp = st
            .record(
                &a,
                b"two",
                SnapshotKind::Checkpoint {
                    name: "before boss".into(),
                },
            )
            .unwrap();
        assert!(matches!(cp.kind, SnapshotKind::Checkpoint { .. }));
        // Going back to older content after a change is a new snapshot.
        let back = st.record(&a, b"one", SnapshotKind::Save).unwrap();
        assert_ne!(back.id, s1.id);

        let tl: Vec<_> = st.timeline(&a).iter().map(|s| s.id).collect();
        assert_eq!(tl, vec![s1.id, s3.id, s4.id, cp.id, back.id]);
        assert_eq!(st.timeline(&b).len(), 1);
        // The blob for identical content is stored once.
        assert_eq!(blob_files(&tmp.path().join("mxl")).len(), 2);

        st.set_label(s1.id, Some("first".into())).unwrap();
        st.make_checkpoint(s3.id, "named".into()).unwrap();
    }
    let st = SnapshotStore::open(tmp.path(), "mxl").unwrap();
    assert_eq!(st.all().len(), 6);
    assert_eq!(st.get(s1.id).unwrap().label.as_deref(), Some("first"));
    assert_eq!(
        st.get(s3.id).unwrap().kind,
        SnapshotKind::Checkpoint {
            name: "named".into()
        }
    );
    assert_eq!(st.read(s1.id).unwrap(), b"one");
    assert_eq!(st.read(s3.id).unwrap(), b"two");
    assert_eq!(st.get(s2.id).unwrap().game_path, b);
    assert_eq!(st.get(s4.id).unwrap().size, 3);
    // New ids continue after reopen.
    let mut st = st;
    let next = st.record(&a, b"three", SnapshotKind::Save).unwrap();
    assert!(next.id > st.all()[4].id);
    // Another profile is independent.
    assert!(
        SnapshotStore::open(tmp.path(), "vanilla")
            .unwrap()
            .all()
            .is_empty()
    );
}

#[test]
fn read_verifies_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let a = gp("data\\a.ds1");
    let mut st = SnapshotStore::open(tmp.path(), "p").unwrap();
    let ok = st
        .record(&a, b"hello hello hello", SnapshotKind::Save)
        .unwrap();
    let bad = st
        .record(&a, b"other other other", SnapshotKind::Save)
        .unwrap();
    let missing = st.record(&a, b"third", SnapshotKind::Save).unwrap();
    let garbage = st.record(&a, b"fourth", SnapshotKind::Save).unwrap();

    let blob = |hash: &str| {
        tmp.path()
            .join("p")
            .join("objects")
            .join(&hash[..2])
            .join(format!("{}.z", &hash[2..]))
    };
    // Valid zlib stream, wrong content: replace with the blob of different bytes.
    let other = blob(&ok.hash.0);
    std::fs::copy(&other, blob(&bad.hash.0)).unwrap();
    std::fs::remove_file(blob(&missing.hash.0)).unwrap();
    std::fs::write(blob(&garbage.hash.0), b"not zlib at all").unwrap();

    assert_eq!(st.read(ok.id).unwrap(), b"hello hello hello");
    for id in [bad.id, missing.id, garbage.id] {
        match st.read(id) {
            Err(HistoryError::Corrupt { id: got, .. }) => assert_eq!(got, id.0),
            other => panic!("expected Corrupt for {id:?}, got {other:?}"),
        }
    }
    assert!(matches!(
        st.read(SnapshotId(9999)),
        Err(HistoryError::UnknownSnapshot(9999))
    ));
    assert!(matches!(
        st.set_label(SnapshotId(9999), None),
        Err(HistoryError::UnknownSnapshot(9999))
    ));
    assert!(matches!(
        st.make_checkpoint(SnapshotId(9999), "x".into()),
        Err(HistoryError::UnknownSnapshot(9999))
    ));
    assert!(matches!(
        st.delete(SnapshotId(9999)),
        Err(HistoryError::UnknownSnapshot(9999))
    ));
}

#[test]
fn corrupt_index_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.json"), b"{ not json").unwrap();
    assert!(matches!(
        SnapshotStore::open(tmp.path(), "p"),
        Err(HistoryError::Index(_))
    ));
    // A hash that could escape the objects folder is rejected too.
    std::fs::write(
        dir.join("index.json"),
        br#"{"version":1,"next_id":2,"snapshots":[{"id":1,"game_path":"data\\a.ds1","hash":"../../x","size":1,"created_unix_ms":0,"kind":{"kind":"save"},"label":null}]}"#,
    )
    .unwrap();
    assert!(matches!(
        SnapshotStore::open(tmp.path(), "p"),
        Err(HistoryError::Index(_))
    ));
}

#[test]
fn index_json_is_written_atomically_and_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let a = gp("data\\a.ds1");
    let mut st = SnapshotStore::open(tmp.path(), "p").unwrap();
    for i in 0..10 {
        st.record(&a, format!("v{i}").as_bytes(), SnapshotKind::Save)
            .unwrap();
        let dir = tmp.path().join("p");
        let index = std::fs::read(dir.join("index.json")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&index).unwrap();
        assert_eq!(v["snapshots"].as_array().unwrap().len(), i + 1);
        let temps = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("capytmp"))
            .count();
        assert_eq!(temps, 0);
    }
}

#[test]
fn prune_keep_last_and_checkpoints_survive() {
    let tmp = tempfile::tempdir().unwrap();
    let a = gp("data\\a.ds1");
    let b = gp("data\\b.ds1");
    let mut st = SnapshotStore::open(tmp.path(), "p").unwrap();
    let mut a_ids = Vec::new();
    for i in 0..6 {
        a_ids.push(
            st.record(&a, format!("a{i}").as_bytes(), SnapshotKind::Save)
                .unwrap()
                .id,
        );
    }
    let b_only = st.record(&b, b"b0", SnapshotKind::Autosave).unwrap();
    // Make the oldest A snapshot a checkpoint: it must survive.
    st.make_checkpoint(a_ids[0], "keep me".into()).unwrap();

    let rep = st
        .prune(&Retention {
            keep_last: 2,
            max_age_days: None,
        })
        .unwrap();
    // a1..a3 pruned (a4, a5 kept), checkpoint a0 kept, b kept.
    assert_eq!(rep.snapshots_removed, 3);
    assert_eq!(rep.blobs_removed, 3);
    assert!(rep.bytes_freed > 0);
    let left: Vec<_> = st.timeline(&a).iter().map(|s| s.id).collect();
    assert_eq!(left, vec![a_ids[0], a_ids[4], a_ids[5]]);
    assert!(st.get(b_only.id).is_some());
    assert_eq!(st.read(a_ids[0]).unwrap(), b"a0");
    assert_eq!(st.read(a_ids[5]).unwrap(), b"a5");
    assert_eq!(blob_files(&tmp.path().join("p")).len(), 4);

    // Pruning persisted.
    let st2 = SnapshotStore::open(tmp.path(), "p").unwrap();
    assert_eq!(st2.all().len(), 4);

    // keep_last = 0 removes every non-checkpoint snapshot.
    let rep = st
        .prune(&Retention {
            keep_last: 0,
            max_age_days: None,
        })
        .unwrap();
    assert_eq!(rep.snapshots_removed, 3);
    assert_eq!(st.all().len(), 1);
    assert_eq!(blob_files(&tmp.path().join("p")).len(), 1);
    // Nothing more to do.
    let rep = st
        .prune(&Retention {
            keep_last: 0,
            max_age_days: Some(1),
        })
        .unwrap();
    assert_eq!(rep, Default::default());
}

#[test]
fn delete_then_prune_collects_blob_but_shared_blobs_stay() {
    let tmp = tempfile::tempdir().unwrap();
    let a = gp("data\\a.ds1");
    let b = gp("data\\b.ds1");
    let mut st = SnapshotStore::open(tmp.path(), "p").unwrap();
    let s1 = st.record(&a, b"shared", SnapshotKind::Save).unwrap();
    let s2 = st.record(&b, b"shared", SnapshotKind::Save).unwrap();
    let s3 = st.record(&a, b"unique", SnapshotKind::Save).unwrap();
    st.delete(s1.id).unwrap();
    st.delete(s3.id).unwrap();
    assert_eq!(st.all().len(), 1);
    let rep = st
        .prune(&Retention {
            keep_last: 10,
            max_age_days: None,
        })
        .unwrap();
    assert_eq!(rep.snapshots_removed, 0);
    assert_eq!(rep.blobs_removed, 1);
    assert_eq!(st.read(s2.id).unwrap(), b"shared");
    assert_eq!(blob_files(&tmp.path().join("p")).len(), 1);
}
