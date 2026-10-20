//! The asset index in the background (P1): open `capybara\cache\index.db`,
//! check whether it matches the profile's archives and rebuild on a worker
//! thread when it does not, reporting progress to the status bar.
//!
//! The thread owns the `Index` (SQLite connections are not `Send`) and talks
//! to the UI through [`IndexEvent`]s. [`IndexState::apply`] is the whole state
//! machine and is pure, so it is tested without a thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use capy_index::{BuildPhase, BuildProgress, Index, IndexError, IndexStatus};
use d2_assets::{Overlay, Profile};

/// What the thread tells the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexEvent {
    /// `status()` answered.
    Checked(IndexStatus),
    Progress(BuildProgress),
    Done(Result<IndexSummary, String>),
}

/// How a run ended well.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexSummary {
    /// Rows of every kind in the profile's index.
    pub assets: u64,
    /// True when this run built the index; false when it was already current.
    pub rebuilt: bool,
    pub warnings: usize,
    pub elapsed: Duration,
}

/// The error text `capy-index` gives when the cancel flag stops a build.
const CANCELLED: &str = "build cancelled";

#[derive(Clone, Debug, PartialEq)]
pub enum IndexState {
    /// Nothing started (no profile yet, or disabled).
    Idle,
    /// Opening the database and comparing fingerprints.
    Checking,
    Building(BuildProgress),
    /// Index current: `rebuilt` says whether this launch had to build it.
    Ready(IndexSummary),
    Cancelled,
    Failed(String),
}

impl IndexState {
    /// The transition for one event. Events that do not fit the state (late
    /// progress after the end, say) leave it alone.
    pub fn apply(&mut self, event: IndexEvent) {
        match (&*self, event) {
            (IndexState::Checking, IndexEvent::Checked(IndexStatus::UpToDate)) => {
                // Stay in Checking until the count comes back with `Done`.
            }
            (IndexState::Checking, IndexEvent::Checked(_)) => {
                *self = IndexState::Building(BuildProgress {
                    phase: BuildPhase::Listing,
                    done: 0,
                    total: 0,
                    current: String::new(),
                });
            }
            (IndexState::Building(_) | IndexState::Checking, IndexEvent::Progress(p)) => {
                *self = IndexState::Building(p);
            }
            (IndexState::Checking | IndexState::Building(_), IndexEvent::Done(result)) => {
                *self = match result {
                    Ok(summary) => IndexState::Ready(summary),
                    Err(e) if e == CANCELLED => IndexState::Cancelled,
                    Err(e) => IndexState::Failed(e),
                };
            }
            _ => {}
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, IndexState::Checking | IndexState::Building(_))
    }

    /// Fraction done of the current build phase, if it has a total.
    pub fn fraction(&self) -> Option<f32> {
        match self {
            IndexState::Building(p) if p.total > 0 => {
                Some((p.done as f32 / p.total as f32).min(1.0))
            }
            _ => None,
        }
    }
}

pub fn phase_label(phase: BuildPhase) -> &'static str {
    match phase {
        BuildPhase::Listing => "Listing files",
        BuildPhase::Tables => "Reading tables",
        BuildPhase::Maps => "Indexing maps",
        BuildPhase::Tiles => "Indexing tiles",
        BuildPhase::Sprites => "Indexing sprites",
        BuildPhase::Thumbnails => "Making thumbnails",
        BuildPhase::Writing => "Writing index",
    }
}

/// The handle the UI keeps for the running (or last) task.
pub struct IndexTask {
    pub state: IndexState,
    cancel: Arc<AtomicBool>,
    /// Which request the state belongs to; late events of older ones are dropped.
    pub generation: u64,
    cancel_requested: bool,
}

impl Default for IndexTask {
    fn default() -> Self {
        IndexTask::new()
    }
}

impl IndexTask {
    pub fn new() -> IndexTask {
        IndexTask {
            state: IndexState::Idle,
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
            cancel_requested: false,
        }
    }

    /// Stop any running build and prepare for a new request. Returns the new
    /// generation and the cancel flag for the thread to watch.
    pub fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel.store(true, Ordering::SeqCst);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        self.state = IndexState::Checking;
        self.cancel_requested = false;
        (self.generation, self.cancel.clone())
    }

    /// Ask the running build to stop (the Cancel button, a profile switch).
    pub fn cancel(&mut self) {
        if self.state.is_running() {
            self.cancel.store(true, Ordering::SeqCst);
            self.cancel_requested = true;
        }
    }

    pub fn cancelling(&self) -> bool {
        self.cancel_requested && self.state.is_running()
    }

    pub fn apply(&mut self, generation: u64, event: IndexEvent) {
        if generation == self.generation {
            self.state.apply(event);
        }
    }
}

/// Throttle for progress events: the UI repaints once a frame at most.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Open the index in `cache_dir`, build it when it is missing or stale, and
/// report through `emit`. Runs on the calling thread.
pub fn run(
    cache_dir: &Path,
    profile: &Profile,
    overlay: &Overlay,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(IndexEvent),
) -> Result<IndexSummary, String> {
    let started = Instant::now();
    let describe = |e: IndexError| e.to_string();
    let mut index = Index::open(cache_dir).map_err(describe)?;
    let status = index.status(&profile.id, overlay).map_err(describe)?;
    emit(IndexEvent::Checked(status.clone()));
    let mut warnings = 0usize;
    let rebuilt = status != IndexStatus::UpToDate;
    if rebuilt {
        let mut last = Instant::now() - PROGRESS_INTERVAL;
        let mut progress = |p: &BuildProgress| {
            let now = Instant::now();
            if now.duration_since(last) >= PROGRESS_INTERVAL {
                last = now;
                emit(IndexEvent::Progress(p.clone()));
            }
        };
        let report = index
            .build(profile, overlay, &mut progress, cancel)
            .map_err(describe)?;
        warnings = report.warnings.len();
    }
    let assets = index.count(&profile.id, None).map_err(describe)?;
    Ok(IndexSummary {
        assets,
        rebuilt,
        warnings,
        elapsed: started.elapsed(),
    })
}

/// Start [`run`] on a worker thread. `send` delivers each event; the last one
/// is always `Done`.
pub fn spawn(
    cache_dir: PathBuf,
    profile: Profile,
    overlay: Arc<Overlay>,
    cancel: Arc<AtomicBool>,
    send: impl Fn(IndexEvent) + Send + Sync + 'static,
) {
    let send = Arc::new(send);
    let thread_send = send.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("index-{}", profile.id))
        .spawn(move || {
            let outcome = crate::crash::catch(|| {
                run(&cache_dir, &profile, &overlay, &cancel, &mut |e| {
                    thread_send(e)
                })
            });
            let result = match outcome {
                Ok(r) => r,
                Err(_) => Err("index task panicked".to_owned()),
            };
            thread_send(IndexEvent::Done(result));
        });
    if let Err(e) = spawned {
        send(IndexEvent::Done(Err(format!(
            "cannot start the index thread: {e}"
        ))));
    }
}

/// After a save: `Index::refresh_saves` on a worker thread. `send` gets a
/// one-line result or the error text.
pub fn spawn_refresh(
    cache_dir: PathBuf,
    profile: Profile,
    overlay: Arc<Overlay>,
    send: impl FnOnce(Result<String, String>) + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name(format!("index-saves-{}", profile.id))
        .spawn(move || {
            let outcome = crate::crash::catch(|| {
                let mut index = Index::open(&cache_dir).map_err(|e| e.to_string())?;
                let report = index
                    .refresh_saves(&profile, &overlay)
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "Asset index: saved maps and tilesets refreshed ({} files, {} ms)",
                    report.files, report.millis
                ))
            });
            send(outcome.unwrap_or_else(|_| Err("the refresh panicked".to_owned())));
        });
    // A thread that cannot start only means a slightly stale index.
    drop(spawned);
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_assets::{AssetSource, FolderSource, GamePath, GameVersion};

    fn progress(phase: BuildPhase, done: u64, total: u64) -> BuildProgress {
        BuildProgress {
            phase,
            done,
            total,
            current: "x".into(),
        }
    }

    fn summary(rebuilt: bool) -> IndexSummary {
        IndexSummary {
            assets: 10,
            rebuilt,
            warnings: 0,
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn missing_goes_through_building_to_ready() {
        let mut t = IndexTask::new();
        assert_eq!(t.state, IndexState::Idle);
        let (g, _) = t.begin();
        assert_eq!(t.state, IndexState::Checking);
        t.apply(g, IndexEvent::Checked(IndexStatus::Missing));
        assert!(matches!(t.state, IndexState::Building(_)));
        t.apply(g, IndexEvent::Progress(progress(BuildPhase::Tiles, 5, 20)));
        assert_eq!(t.state.fraction(), Some(0.25));
        t.apply(g, IndexEvent::Done(Ok(summary(true))));
        assert_eq!(t.state, IndexState::Ready(summary(true)));
        assert!(!t.state.is_running());
        // Late progress after the end changes nothing.
        t.apply(g, IndexEvent::Progress(progress(BuildPhase::Tiles, 6, 20)));
        assert_eq!(t.state, IndexState::Ready(summary(true)));
    }

    #[test]
    fn stale_also_rebuilds_and_up_to_date_does_not() {
        let mut t = IndexTask::new();
        let (g, _) = t.begin();
        t.apply(
            g,
            IndexEvent::Checked(IndexStatus::Stale {
                reason: "size".into(),
            }),
        );
        assert!(matches!(t.state, IndexState::Building(_)));
        let (g, _) = t.begin();
        t.apply(g, IndexEvent::Checked(IndexStatus::UpToDate));
        assert_eq!(t.state, IndexState::Checking, "no building phase");
        t.apply(g, IndexEvent::Done(Ok(summary(false))));
        assert_eq!(t.state, IndexState::Ready(summary(false)));
    }

    #[test]
    fn cancel_flag_and_cancelled_state() {
        let mut t = IndexTask::new();
        let (g, flag) = t.begin();
        t.apply(g, IndexEvent::Checked(IndexStatus::Missing));
        assert!(!flag.load(Ordering::SeqCst));
        t.cancel();
        assert!(flag.load(Ordering::SeqCst));
        assert!(t.cancelling());
        t.apply(g, IndexEvent::Done(Err(CANCELLED.to_owned())));
        assert_eq!(t.state, IndexState::Cancelled);
        assert!(!t.cancelling());
        // Cancelling an idle task is a no-op.
        t.cancel();
        assert_eq!(t.state, IndexState::Cancelled);
    }

    #[test]
    fn a_new_request_cancels_the_old_one_and_ignores_its_events() {
        let mut t = IndexTask::new();
        let (g1, flag1) = t.begin();
        t.apply(g1, IndexEvent::Checked(IndexStatus::Missing));
        let (g2, flag2) = t.begin(); // profile switch
        assert!(flag1.load(Ordering::SeqCst), "old build told to stop");
        assert!(!flag2.load(Ordering::SeqCst));
        t.apply(g1, IndexEvent::Done(Ok(summary(true))));
        assert_eq!(t.state, IndexState::Checking, "stale event dropped");
        t.apply(g2, IndexEvent::Done(Err("boom".into())));
        assert_eq!(t.state, IndexState::Failed("boom".into()));
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = GamePath::new(rel).to_native(root);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    /// The real runner over a tiny loose-folder profile: first launch builds,
    /// second launch finds the index current and does not rebuild.
    #[test]
    fn run_builds_once_then_reports_up_to_date() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().join("base");
        write(
            &base,
            "data\\global\\excel\\lvltypes.txt",
            b"Name\tId\tAct\tFile 1\nT\t1\t1\tact1\\town\\floor.dt1\n",
        );
        write(
            &base,
            "data\\global\\excel\\lvlprest.txt",
            b"Def\tLevelId\tDt1Mask\tFile1\n1\t1\t1\tact1\\town\\a.ds1\n",
        );
        write(
            &base,
            "data\\global\\excel\\levels.txt",
            b"Id\tLevelName\tAct\tLevelType\tDrlgType\n1\tTown\t0\t1\t2\n",
        );
        let overlay = Overlay::new(vec![
            Box::new(FolderSource::new(&base, "d2data.mpq")) as Box<dyn AssetSource>
        ]);
        let profile = Profile {
            id: "synthetic".into(),
            name: "Synthetic".into(),
            game_version: GameVersion::V114d,
            layers: Vec::new(),
        };
        let cache = tmp.path().join("cache");
        let cancel = AtomicBool::new(false);

        let mut events = Vec::new();
        let first = run(&cache, &profile, &overlay, &cancel, &mut |e| events.push(e))
            .expect("first run builds");
        assert!(first.rebuilt);
        assert!(matches!(
            events.first(),
            Some(IndexEvent::Checked(IndexStatus::Missing))
        ));

        let mut events = Vec::new();
        let second = run(&cache, &profile, &overlay, &cancel, &mut |e| events.push(e))
            .expect("second run checks");
        assert!(!second.rebuilt, "second launch must not rebuild");
        assert_eq!(second.assets, first.assets);
        assert_eq!(
            events,
            vec![IndexEvent::Checked(IndexStatus::UpToDate)],
            "no progress events without a build"
        );
    }

    #[test]
    fn run_honours_a_preset_cancel_flag() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().join("base");
        write(&base, "data\\global\\excel\\levels.txt", b"Id\n1\n");
        let overlay = Overlay::new(vec![
            Box::new(FolderSource::new(&base, "d2data.mpq")) as Box<dyn AssetSource>
        ]);
        let profile = Profile {
            id: "c".into(),
            name: "c".into(),
            game_version: GameVersion::V114d,
            layers: Vec::new(),
        };
        let cancel = AtomicBool::new(true);
        let err = run(
            &tmp.path().join("cache"),
            &profile,
            &overlay,
            &cancel,
            &mut |_| {},
        )
        .expect_err("cancelled");
        assert_eq!(err, CANCELLED);
        let mut state = IndexState::Checking;
        state.apply(IndexEvent::Done(Err(err)));
        assert_eq!(state, IndexState::Cancelled);
    }
}
