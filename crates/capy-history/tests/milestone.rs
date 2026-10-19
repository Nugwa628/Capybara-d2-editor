#![cfg(feature = "milestones")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use capy_history::{Milestones, SavesDir};
use d2_assets::GamePath;

fn tree_paths(repo_dir: &std::path::Path, commit: &str) -> Vec<String> {
    let repo = gix::open(repo_dir).unwrap();
    let id = gix::ObjectId::from_hex(commit.as_bytes()).unwrap();
    let tree = repo.find_commit(id).unwrap().tree().unwrap();
    let mut rec = gix::traverse::tree::Recorder::default();
    tree.traverse().breadthfirst(&mut rec).unwrap();
    let mut out: Vec<String> = rec
        .records
        .iter()
        .filter(|e| e.mode.is_blob())
        .map(|e| e.filepath.to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn init_commit_log_noop_and_deletions() {
    let tmp = tempfile::tempdir().unwrap();
    let saves = tmp.path().join("saves");
    let ms = Milestones::open_or_init(&saves).unwrap();
    assert!(ms.log(10).unwrap().is_empty());

    let dir = SavesDir::new(&saves, "mxl");
    let a = GamePath::new("data\\global\\a.ds1");
    let b = GamePath::new("data\\global\\b.ds1");
    dir.save(&a, b"A1").unwrap();
    dir.save(&b, b"B1").unwrap();
    let c1 = ms.commit("first").unwrap();

    dir.save(&a, b"A2").unwrap();
    let c2 = ms.commit("second").unwrap();
    assert_ne!(c1, c2);

    // No changes: same id back, no new commit.
    let c3 = ms.commit("nothing changed").unwrap();
    assert_eq!(c3, c2);
    let log = ms.log(10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, c2);
    assert_eq!(log[0].message, "second");
    assert_eq!(log[1].id, c1);
    assert_eq!(log[1].message, "first");
    assert!(log[0].time_unix >= log[1].time_unix && log[1].time_unix > 1_600_000_000);
    assert_eq!(ms.log(1).unwrap().len(), 1);

    // Reopen sees the same history.
    let again = Milestones::open_or_init(&saves).unwrap();
    assert_eq!(again.log(10).unwrap(), log);

    let before = tree_paths(&saves, &c2);
    assert!(
        before.contains(&"mxl/data/global/b.ds1".to_string()),
        "{before:?}"
    );
    assert!(before.contains(&"mxl/CHANGED.txt".to_string()));

    // Delete a file: it disappears from the next commit's tree.
    std::fs::remove_file(dir.path_for(&b).unwrap()).unwrap();
    let c4 = ms.commit("deleted b").unwrap();
    assert_ne!(c4, c2);
    let after = tree_paths(&saves, &c4);
    assert!(
        !after.contains(&"mxl/data/global/b.ds1".to_string()),
        "{after:?}"
    );
    assert!(after.contains(&"mxl/data/global/a.ds1".to_string()));
    // The old commit still has it.
    assert!(tree_paths(&saves, &c2).contains(&"mxl/data/global/b.ds1".to_string()));

    // Author identity.
    let repo = gix::open(&saves).unwrap();
    let head = repo.head_commit().unwrap();
    let author = head.author().unwrap();
    assert_eq!(author.name, "Capybara's D2 Editor");
}

#[test]
fn empty_saves_folder_can_be_committed_and_stale_temps_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let saves = tmp.path().join("saves");
    let ms = Milestones::open_or_init(&saves).unwrap();
    let first = ms.commit("empty").unwrap();
    assert_eq!(ms.commit("still empty").unwrap(), first);
    std::fs::create_dir_all(saves.join("p")).unwrap();
    std::fs::write(saves.join("p").join(".x.capytmp-1-1"), b"junk").unwrap();
    assert_eq!(ms.commit("temp only").unwrap(), first);
}
