//! Saving a map: serialise, write it to `capybara\saves\<profile>\<game
//! path>` (which appends to CHANGED.txt), record a snapshot in
//! `capybara\history\<profile>\`. Nothing is ever written outside `capybara\`.

use std::path::PathBuf;

use capy_history::{SavesDir, SnapshotId, SnapshotKind, SnapshotStore};
use d2_assets::GamePath;
use d2_formats::FormatError;
use d2_formats::ds1::Ds1;

use crate::paths::Paths;

/// What a save did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReport {
    /// The loose `.ds1` that was written.
    pub path: PathBuf,
    pub bytes: usize,
    /// `None` when the snapshot could not be recorded (see `warnings`).
    pub snapshot: Option<SnapshotId>,
    /// Entries in CHANGED.txt after the save.
    pub changed_total: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// The model holds something its DS1 version cannot store. Upgrading to
    /// `min_version` makes it writable.
    NotRepresentable { detail: String, min_version: i32 },
    /// Nothing was (completely) written.
    Failed(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NotRepresentable {
                detail,
                min_version,
            } => write!(
                f,
                "this DS1 version cannot store the map ({detail}); version {min_version} can"
            ),
            SaveError::Failed(text) => f.write_str(text),
        }
    }
}

/// Writes `ds1` as the profile's edited copy of `game_path`.
pub fn save_ds1(
    paths: &Paths,
    profile_id: &str,
    game_path: &GamePath,
    ds1: &Ds1,
) -> Result<SaveReport, SaveError> {
    let bytes = match ds1.to_bytes() {
        Ok(b) => b,
        Err(FormatError::NotRepresentable { detail, .. }) => {
            return Err(SaveError::NotRepresentable {
                detail,
                min_version: ds1.min_version_for_model(),
            });
        }
        Err(e) => return Err(SaveError::Failed(format!("cannot serialise the map: {e}"))),
    };
    let saves = SavesDir::new(&paths.saves, profile_id);
    let path = saves
        .save(game_path, &bytes)
        .map_err(|e| SaveError::Failed(format!("cannot write {game_path}: {e}")))?;
    let mut warnings = Vec::new();
    let snapshot = match SnapshotStore::open(&paths.history, profile_id)
        .and_then(|mut store| store.record(game_path, &bytes, SnapshotKind::Save))
    {
        Ok(s) => Some(s.id),
        Err(e) => {
            warnings.push(format!("saved, but the history snapshot failed: {e}"));
            None
        }
    };
    let changed_total = match saves.changed() {
        Ok(list) => list.len(),
        Err(e) => {
            warnings.push(format!("cannot read CHANGED.txt: {e}"));
            0
        }
    };
    Ok(SaveReport {
        path,
        bytes: bytes.len(),
        snapshot,
        changed_total,
        warnings,
    })
}

/// Number of files listed in the profile's CHANGED.txt (0 when unreadable).
pub fn changed_count(paths: &Paths, profile_id: &str) -> usize {
    SavesDir::new(&paths.saves, profile_id)
        .changed()
        .map_or(0, |l| l.len())
}

/// "Mark as packed": empties CHANGED.txt. Returns how many entries it held.
pub fn mark_packed(paths: &Paths, profile_id: &str) -> Result<usize, String> {
    let saves = SavesDir::new(&paths.saves, profile_id);
    let before = saves.changed().map_or(0, |l| l.len());
    saves.mark_packed().map_err(|e| e.to_string())?;
    Ok(before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::ds1::Cell;

    fn paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        (tmp, paths)
    }

    fn map() -> Ds1 {
        let mut ds1 = Ds1::new(4, 4, 0);
        ds1.floors[0].set(1, 1, Cell(1).with_indices(3, 2));
        ds1
    }

    #[test]
    fn save_writes_the_file_changed_txt_and_a_snapshot() {
        let (_tmp, paths) = paths();
        let game = GamePath::new("data\\global\\tiles\\act1\\town\\towne1.ds1");
        let ds1 = map();
        let report = save_ds1(&paths, "vanilla", &game, &ds1).expect("save");
        let expected = paths
            .saves
            .join("vanilla")
            .join("data")
            .join("global")
            .join("tiles")
            .join("act1")
            .join("town")
            .join("towne1.ds1");
        assert_eq!(report.path, expected);
        let on_disk = std::fs::read(&expected).expect("read back");
        assert_eq!(on_disk, ds1.to_bytes().expect("bytes"));
        assert_eq!(Ds1::parse(&on_disk).expect("parse"), ds1);
        assert_eq!(report.changed_total, 1);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(changed_count(&paths, "vanilla"), 1);
        let store = SnapshotStore::open(&paths.history, "vanilla").expect("store");
        let timeline = store.timeline(&game);
        assert_eq!(timeline.len(), 1);
        assert_eq!(Some(timeline[0].id), report.snapshot);
        assert_eq!(timeline[0].kind, SnapshotKind::Save);
        assert_eq!(store.read(timeline[0].id).expect("blob"), on_disk);
    }

    #[test]
    fn saving_twice_lists_the_path_once_and_dedupes_the_snapshot() {
        let (_tmp, paths) = paths();
        let game = GamePath::new("data\\global\\tiles\\a.ds1");
        let ds1 = map();
        let first = save_ds1(&paths, "p", &game, &ds1).expect("save");
        let second = save_ds1(&paths, "p", &game, &ds1).expect("save again");
        assert_eq!(second.changed_total, 1);
        assert_eq!(first.snapshot, second.snapshot, "identical bytes dedupe");
        let mut edited = ds1.clone();
        edited.floors[0].set(2, 2, Cell(1).with_indices(4, 0));
        let third = save_ds1(&paths, "p", &game, &edited).expect("save edited");
        assert_ne!(third.snapshot, first.snapshot);
        let store = SnapshotStore::open(&paths.history, "p").expect("store");
        assert_eq!(store.timeline(&game).len(), 2);
        // Other profiles are separate.
        assert_eq!(changed_count(&paths, "other"), 0);
    }

    #[test]
    fn mark_packed_clears_the_list() {
        let (_tmp, paths) = paths();
        for name in ["a", "b"] {
            let game = GamePath::new(&format!("data\\global\\tiles\\{name}.ds1"));
            save_ds1(&paths, "p", &game, &map()).expect("save");
        }
        assert_eq!(changed_count(&paths, "p"), 2);
        assert_eq!(mark_packed(&paths, "p"), Ok(2));
        assert_eq!(changed_count(&paths, "p"), 0);
        assert!(
            paths
                .saves
                .join("p")
                .join("data")
                .join("global")
                .join("tiles")
                .join("a.ds1")
                .is_file(),
            "the saved maps stay"
        );
    }

    #[test]
    fn an_unwritable_model_explains_itself_and_writes_nothing() {
        let (_tmp, paths) = paths();
        let mut ds1 = map();
        // A second wall layer cannot be stored in a v3 file.
        ds1.version = 3;
        ds1.add_wall_layer().ok();
        if ds1.walls.len() < 2 {
            // add_wall_layer refused (it checks the version): build it by hand.
            let extra = ds1.walls[0].clone();
            ds1.walls.push(extra);
        }
        let game = GamePath::new("data\\global\\tiles\\old.ds1");
        let err = save_ds1(&paths, "p", &game, &ds1).expect_err("not writable");
        match err {
            SaveError::NotRepresentable { min_version, .. } => assert!(min_version > 3),
            other => unreachable!("unexpected error {other:?}"),
        }
        assert!(
            !paths.saves.join("p").join("data").exists(),
            "nothing written"
        );
        assert_eq!(changed_count(&paths, "p"), 0);
        assert!(
            SnapshotStore::open(&paths.history, "p")
                .expect("store")
                .all()
                .is_empty()
        );
    }

    #[test]
    fn bad_game_paths_are_refused() {
        let (_tmp, paths) = paths();
        let game = GamePath::new("..\\..\\evil.ds1");
        let err = save_ds1(&paths, "p", &game, &map()).expect_err("refused");
        assert!(matches!(err, SaveError::Failed(_)), "{err:?}");
        assert!(
            !paths
                .root
                .parent()
                .expect("parent")
                .join("evil.ds1")
                .exists()
        );
    }
}
