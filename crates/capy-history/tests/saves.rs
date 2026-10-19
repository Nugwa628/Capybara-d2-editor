#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use capy_history::{HistoryError, SavesDir, atomic_write};
use d2_assets::GamePath;

fn gp(s: &str) -> GamePath {
    GamePath::new(s)
}

#[test]
fn path_for_rejects_escapes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = SavesDir::new(tmp.path(), "mxl");
    for bad in [
        "data\\..\\..\\evil.ds1",
        "C:\\windows\\x.ds1",
        "d:x",
        "..",
        "a\\..\\b",
    ] {
        assert!(
            matches!(dir.path_for(&gp(bad)), Err(HistoryError::BadGamePath(_))),
            "{bad}"
        );
        assert!(dir.save(&gp(bad), b"x").is_err());
    }
    // A leading separator is stripped by GamePath, so it stays inside the root.
    let p = dir.path_for(&gp("/data/x.ds1")).unwrap();
    assert!(p.starts_with(dir.root()));
    // Nothing was written outside the root.
    assert!(!tmp.path().join("evil.ds1").exists());
}

#[test]
fn changed_has_no_duplicates_across_saves_and_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let a = gp("data\\a.ds1");
    let b = gp("Data\\B.ds1");
    {
        let dir = SavesDir::new(tmp.path(), "p");
        assert!(dir.changed().unwrap().is_empty());
        let path = dir.save(&a, b"1").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"1");
        dir.save(&b, b"2").unwrap();
        dir.save(&a, b"3").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"3");
        assert_eq!(dir.changed().unwrap(), vec![a.clone(), b.clone()]);
    }
    let dir = SavesDir::new(tmp.path(), "p");
    dir.save(&a, b"4").unwrap();
    dir.save(&b, b"5").unwrap();
    assert_eq!(dir.changed().unwrap(), vec![a.clone(), b.clone()]);
    let raw = std::fs::read_to_string(dir.root().join("CHANGED.txt")).unwrap();
    assert_eq!(raw, "data\\a.ds1\r\ndata\\b.ds1\r\n");

    // Profiles are separate.
    let other = SavesDir::new(tmp.path(), "q");
    assert!(other.changed().unwrap().is_empty());

    dir.mark_packed().unwrap();
    assert!(dir.changed().unwrap().is_empty());
    assert_eq!(
        std::fs::metadata(dir.root().join("CHANGED.txt"))
            .unwrap()
            .len(),
        0
    );
    dir.save(&b, b"6").unwrap();
    assert_eq!(dir.changed().unwrap(), vec![b]);
}

#[test]
fn check_writable_ok_and_creates_root() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = SavesDir::new(&tmp.path().join("deep").join("saves"), "p");
    dir.check_writable().unwrap();
    assert!(dir.root().is_dir());
    // The probe file is gone again.
    assert_eq!(std::fs::read_dir(dir.root()).unwrap().count(), 0);
}

#[test]
fn check_writable_reports_not_writable_when_root_is_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("saves");
    std::fs::write(&blocker, b"i am a file").unwrap();
    let dir = SavesDir::new(&blocker, "p");
    assert!(matches!(
        dir.check_writable(),
        Err(HistoryError::NotWritable { .. })
    ));
}

#[test]
fn check_writable_reports_not_writable_on_read_only_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = SavesDir::new(tmp.path(), "p");
    std::fs::create_dir_all(dir.root()).unwrap();
    let mut perms = std::fs::metadata(dir.root()).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(dir.root(), perms.clone()).unwrap();
    let result = dir.check_writable();
    // Restore so the temp dir can be cleaned up.
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(dir.root(), perms).unwrap();
    match result {
        Err(HistoryError::NotWritable { .. }) => {}
        Ok(()) => {
            eprintln!("skipped: this platform/user ignores the read-only attribute on directories")
        }
        Err(other) => panic!("unexpected error {other}"),
    }
}

#[test]
fn check_writable_reports_not_writable_on_read_only_file() {
    // On Windows the read-only attribute on a directory does not stop file
    // creation, but it does stop overwriting a read-only file. Make the
    // probe's own name read-only to exercise the failure path there.
    let tmp = tempfile::tempdir().unwrap();
    let dir = SavesDir::new(tmp.path(), "p");
    std::fs::create_dir_all(dir.root()).unwrap();
    let probe = dir
        .root()
        .join(format!(".capy-write-probe-{}", std::process::id()));
    std::fs::write(&probe, b"x").unwrap();
    let mut perms = std::fs::metadata(&probe).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&probe, perms.clone()).unwrap();
    let result = dir.check_writable();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&probe, perms).unwrap();
    match result {
        Err(HistoryError::NotWritable { .. }) => {}
        Ok(()) => eprintln!("skipped: read-only attribute ignored on this platform/user"),
        Err(other) => panic!("unexpected error {other}"),
    }
}

#[test]
fn atomic_write_replaces_and_leaves_no_temps() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("x").join("f.bin");
    for i in 0..20u8 {
        let bytes = vec![i; 1000 + usize::from(i)];
        atomic_write(&p, &bytes).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
    }
    assert_eq!(std::fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
}
