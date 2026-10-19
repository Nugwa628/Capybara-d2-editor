//! Autosave: every `autosave_minutes`, each open map with unsaved changes is
//! recorded as a `SnapshotKind::Autosave` in `capybara\history\<profile>\`.
//! The serialising and writing run on a worker thread. Autosave never writes
//! `capybara\saves\` or CHANGED.txt.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use capy_history::{Snapshot, SnapshotKind};
use d2_assets::GamePath;
use d2_formats::ds1::Ds1;

use super::{HistoryEvent, with_store};
use crate::doc::MapDoc;

/// Counts frame time towards the next autosave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AutosaveTimer {
    elapsed: Duration,
}

impl AutosaveTimer {
    /// Adds `dt`. True when an autosave is due; the timer then restarts.
    /// `minutes == 0` means autosave is off: never due, and the timer rests.
    pub fn tick(&mut self, dt: Duration, minutes: u32) -> bool {
        if minutes == 0 {
            self.elapsed = Duration::ZERO;
            return false;
        }
        self.elapsed += dt;
        if self.elapsed >= Duration::from_secs(u64::from(minutes) * 60) {
            self.elapsed = Duration::ZERO;
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }

    #[cfg(test)]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
}

/// One map to record.
#[derive(Clone, Debug)]
pub struct AutosaveJob {
    pub doc: u64,
    /// The document revision the copy was taken at.
    pub revision: u64,
    pub profile: String,
    pub path: GamePath,
    pub ds1: Arc<Ds1>,
}

/// What recording one map did.
#[derive(Clone, Debug)]
pub struct AutosaveResult {
    pub doc: u64,
    pub revision: u64,
    pub path: GamePath,
    pub result: Result<Snapshot, String>,
}

/// The maps to autosave: dirty ones whose current revision has not been
/// autosaved yet. Clean maps are never recorded (their state is a save).
pub fn collect_jobs(docs: &[MapDoc]) -> Vec<AutosaveJob> {
    docs.iter()
        .filter(|d| d.is_dirty() && d.autosaved_revision != Some(d.edit.revision()))
        .map(|d| AutosaveJob {
            doc: d.id,
            revision: d.edit.revision(),
            profile: d.profile_id.clone(),
            path: d.path.clone(),
            // The edited model, not what the view shows (it may be a snapshot).
            ds1: Arc::new(d.edit.ds1().clone()),
        })
        .collect()
}

/// Serialises and records each job (the worker's body).
pub fn run_jobs(history_root: &Path, jobs: Vec<AutosaveJob>) -> Vec<AutosaveResult> {
    jobs.into_iter()
        .map(|job| {
            let result = job
                .ds1
                .to_bytes()
                .map_err(|e| format!("cannot serialise the map: {e}"))
                .and_then(|bytes| {
                    with_store(history_root, &job.profile, |store| {
                        store.record(&job.path, &bytes, SnapshotKind::Autosave)
                    })
                });
            AutosaveResult {
                doc: job.doc,
                revision: job.revision,
                path: job.path,
                result,
            }
        })
        .collect()
}

/// Runs `jobs` on a worker thread; the results arrive as
/// [`HistoryEvent::Autosaved`]. False when the thread could not start.
pub fn spawn(history_root: PathBuf, jobs: Vec<AutosaveJob>, tx: Sender<HistoryEvent>) -> bool {
    std::thread::Builder::new()
        .name("autosave".to_owned())
        .spawn(move || {
            let results = run_jobs(&history_root, jobs);
            let _ = tx.send(HistoryEvent::Autosaved(results));
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LayerVisibility;
    use crate::doc::{SourceCategory, SourceInfo};
    use capy_edit::{CellValue, LayerRef};
    use capy_history::SnapshotStore;
    use d2_formats::ds1::Cell;

    fn doc(id: u64, name: &str) -> MapDoc {
        let mut ds1 = Ds1::new(4, 4, 0);
        ds1.floors[0].set(0, 0, Cell(1).with_indices(1, 1));
        MapDoc::new(
            id,
            "vanilla".into(),
            GamePath::new(&format!("data\\global\\tiles\\{name}.ds1")),
            SourceInfo {
                category: SourceCategory::Base,
                layer_name: "d2data.mpq".into(),
            },
            ds1,
            &LayerVisibility::default(),
        )
    }

    fn paint(d: &mut MapDoc, x: u32) {
        let value = CellValue {
            cell: Cell(1).with_indices(3, 2).0,
            orientation: 0,
        };
        d.edit
            .paint(LayerRef::Floor(0), &[(x, 1, value)])
            .expect("paint");
        d.sync();
    }

    #[test]
    fn the_timer_fires_on_the_interval_and_is_off_at_zero() {
        let mut t = AutosaveTimer::default();
        let sec = Duration::from_secs(1);
        for _ in 0..59 {
            assert!(!t.tick(sec, 1));
        }
        assert!(t.tick(sec, 1), "one minute");
        assert_eq!(t.elapsed(), Duration::ZERO, "restarts");
        assert!(!t.tick(Duration::from_secs(30), 1));
        // Off: never fires and forgets what had accumulated.
        assert!(!t.tick(Duration::from_secs(3600), 0));
        assert_eq!(t.elapsed(), Duration::ZERO);
        assert!(!t.tick(Duration::from_secs(299), 5));
        assert!(t.tick(sec, 5));
        t.tick(Duration::from_secs(10), 5);
        t.reset();
        assert_eq!(t.elapsed(), Duration::ZERO);
    }

    #[test]
    fn only_dirty_maps_are_autosaved_once_per_revision() {
        let mut docs = vec![doc(1, "clean"), doc(2, "dirty")];
        assert!(collect_jobs(&docs).is_empty(), "nothing is dirty");
        paint(&mut docs[1], 2);
        let jobs = collect_jobs(&docs);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].doc, 2);
        assert_eq!(jobs[0].ds1.as_ref(), docs[1].edit.ds1());
        // Once recorded at this revision it is not picked again...
        docs[1].autosaved_revision = Some(jobs[0].revision);
        assert!(collect_jobs(&docs).is_empty());
        // ...until it changes again.
        paint(&mut docs[1], 3);
        assert_eq!(collect_jobs(&docs).len(), 1);
        // Undoing back to the saved state makes it clean: no autosave.
        docs[1].edit.undo();
        docs[1].edit.undo();
        assert!(!docs[1].is_dirty());
        assert!(collect_jobs(&docs).is_empty());
    }

    #[test]
    fn jobs_record_autosave_snapshots_only_in_history() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = crate::paths::Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        let mut d = doc(1, "a");
        paint(&mut d, 1);
        let results = run_jobs(&paths.history, collect_jobs(std::slice::from_ref(&d)));
        assert_eq!(results.len(), 1);
        let snap = results[0].result.clone().expect("recorded");
        assert_eq!(snap.kind, SnapshotKind::Autosave);
        let store = SnapshotStore::open(&paths.history, "vanilla").expect("store");
        assert_eq!(
            store.read(snap.id).expect("blob"),
            d.edit.ds1().to_bytes().expect("bytes")
        );
        // The same content again is deduplicated by the store.
        let again = run_jobs(&paths.history, collect_jobs(std::slice::from_ref(&d)));
        assert_eq!(again[0].result.as_ref().map(|s| s.id), Ok(snap.id));
        // Nothing in saves (no map, no CHANGED.txt).
        let saves = std::fs::read_dir(&paths.saves).expect("saves").count();
        assert_eq!(saves, 0, "autosave never writes saves");
    }
}
