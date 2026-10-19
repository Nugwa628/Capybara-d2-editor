//! History: autosave, the snapshot timeline of each map, named checkpoints,
//! the visual diff against a snapshot, restore, retention and gix
//! milestones. Logic and state only; the panel is in `crate::ui::history`.
//!
//! Every access to a profile's `SnapshotStore` goes through [`with_store`],
//! which opens the store fresh under one process-wide lock. The UI thread,
//! the autosave worker and saves therefore never overwrite each other's
//! `index.json` with a stale copy.

pub mod autosave;
pub mod overlay;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use capy_edit::{Document, LayerRef, Patch};
use capy_history::{
    MilestoneInfo, Milestones, PruneReport, Retention, Snapshot, SnapshotId, SnapshotKind,
    SnapshotStore,
};
use d2_assets::GamePath;
use d2_formats::ds1::Ds1;

use crate::iso;
use crate::state::AppState;
use autosave::{AutosaveResult, AutosaveTimer};
use overlay::DiffOverlay;

/// How many milestones the panel lists.
pub const MILESTONE_LOG_LIMIT: usize = 50;

static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Opens `history_root\<profile>` and runs `f` on it, serialised with every
/// other store access of this process.
pub fn with_store<T>(
    history_root: &Path,
    profile: &str,
    f: impl FnOnce(&mut SnapshotStore) -> capy_history::Result<T>,
) -> Result<T, String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mut store = SnapshotStore::open(history_root, profile).map_err(|e| e.to_string())?;
    f(&mut store).map_err(|e| e.to_string())
}

/// Reads one snapshot's bytes.
pub fn read_snapshot(
    history_root: &Path,
    profile: &str,
    id: SnapshotId,
) -> Result<Vec<u8>, String> {
    with_store(history_root, profile, |s| s.read(id))
}

// ───────────────────────── formatting ─────────────────────────

fn system_tz() -> &'static jiff::tz::TimeZone {
    static TZ: OnceLock<jiff::tz::TimeZone> = OnceLock::new();
    TZ.get_or_init(jiff::tz::TimeZone::system)
}

/// `unix_ms` in local time, formatted with `strftime` codes.
pub fn local_time(unix_ms: i64, format: &str) -> String {
    match jiff::Timestamp::from_millisecond(unix_ms) {
        Ok(t) => t.to_zoned(system_tz().clone()).strftime(format).to_string(),
        Err(_) => "?".to_owned(),
    }
}

/// Now, in milliseconds since 1970.
pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// `1.2 KB`, `340 B`.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn kind_label(kind: &SnapshotKind) -> &'static str {
    match kind {
        SnapshotKind::Save => "Save",
        SnapshotKind::Autosave => "Autosave",
        SnapshotKind::Checkpoint { .. } => "Checkpoint",
        SnapshotKind::BeforeRestore => "Before restore",
        #[allow(unreachable_patterns)]
        _ => "Snapshot",
    }
}

/// The kind's script name: `save`, `autosave`, `checkpoint`, `before-restore`.
pub fn kind_slug(kind: &SnapshotKind) -> &'static str {
    match kind {
        SnapshotKind::Save => "save",
        SnapshotKind::Autosave => "autosave",
        SnapshotKind::Checkpoint { .. } => "checkpoint",
        SnapshotKind::BeforeRestore => "before-restore",
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

/// The name a snapshot is shown and found by: the checkpoint name, else the
/// label.
pub fn display_name(s: &Snapshot) -> Option<&str> {
    match &s.kind {
        SnapshotKind::Checkpoint { name } => Some(name.as_str()),
        _ => s.label.as_deref(),
    }
}

/// Finds a snapshot in a newest-first timeline by its position (`0` is the
/// newest) or by checkpoint name / label (newest match).
pub fn find_in_timeline(timeline: &[Snapshot], key: &str) -> Option<SnapshotId> {
    if let Ok(i) = key.parse::<usize>()
        && let Some(s) = timeline.get(i)
    {
        return Some(s.id);
    }
    timeline
        .iter()
        .find(|s| display_name(s) == Some(key) || s.label.as_deref() == Some(key))
        .map(|s| s.id)
}

// ───────────────────────── restore ─────────────────────────

/// Restores `snapshot_bytes` into `doc`: first records the current map as a
/// `BeforeRestore` snapshot, then applies `Patch::replace(current, snapshot)`
/// as one undoable step (the map becomes dirty). If the current map cannot
/// be recorded, nothing is changed. Returns the BeforeRestore snapshot.
pub fn restore_into(
    history_root: &Path,
    profile: &str,
    game_path: &GamePath,
    doc: &mut Document,
    snapshot_bytes: &[u8],
    label: &str,
) -> Result<Snapshot, String> {
    let target =
        Ds1::parse(snapshot_bytes).map_err(|e| format!("the snapshot does not parse: {e}"))?;
    let current = doc.ds1().clone();
    let current_bytes = current
        .to_bytes()
        .map_err(|e| format!("cannot record the current map before restoring: {e}"))?;
    let before = with_store(history_root, profile, |s| {
        s.record(game_path, &current_bytes, SnapshotKind::BeforeRestore)
    })
    .map_err(|e| format!("cannot record the current map before restoring: {e}"))?;
    doc.apply_patch(Patch::replace(label, current, target))
        .map_err(|e| format!("cannot apply the snapshot: {e}"))?;
    Ok(before)
}

// ───────────────────────── milestones ─────────────────────────

/// Commits `capybara\saves\` as a milestone. `Ok(None)` when nothing
/// changed since the last milestone.
pub fn commit_milestone(saves_root: &Path, message: &str) -> Result<Option<String>, String> {
    let repo = Milestones::open_or_init(saves_root).map_err(|e| e.to_string())?;
    let head = repo
        .log(1)
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .map(|m| m.id);
    let id = repo.commit(message).map_err(|e| e.to_string())?;
    Ok((head.as_deref() != Some(id.as_str())).then_some(id))
}

/// The milestone log, newest first. A saves folder without a repository has
/// no milestones (nothing is created by looking).
pub fn milestone_log(saves_root: &Path, limit: usize) -> Result<Vec<MilestoneInfo>, String> {
    if !saves_root.join(".git").exists() {
        return Ok(Vec::new());
    }
    Milestones::open_or_init(saves_root)
        .and_then(|m| m.log(limit))
        .map_err(|e| e.to_string())
}

// ───────────────────────── state ─────────────────────────

/// Finished background history work.
pub enum HistoryEvent {
    Autosaved(Vec<AutosaveResult>),
    Pruned {
        profile: String,
        result: Result<PruneReport, String>,
    },
}

/// A question or text prompt of the History panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryPrompt {
    /// "Checkpoint...": name for a new checkpoint of the active map.
    Checkpoint {
        name: String,
    },
    /// Rename / label a snapshot.
    Label {
        id: SnapshotId,
        text: String,
    },
    /// Turn a snapshot into a checkpoint (or rename one).
    MakeCheckpoint {
        id: SnapshotId,
        name: String,
    },
    Restore {
        id: SnapshotId,
    },
    Delete {
        id: SnapshotId,
    },
    /// "Milestone...": the commit message.
    Milestone {
        message: String,
    },
}

/// The snapshot picked in the timeline, its parsed map and the diff overlay.
struct Compare {
    doc: u64,
    id: SnapshotId,
    snapshot: Result<Arc<Ds1>, String>,
    /// The document revision the overlay was computed at.
    overlay: Option<(u64, DiffOverlay)>,
}

pub struct HistoryState {
    pub timer: AutosaveTimer,
    pub autosave_in_flight: bool,
    /// Finished autosave runs (scripts wait for the next one).
    pub autosave_runs: u64,
    /// The failures of the last autosave run.
    pub autosave_errors: Vec<String>,
    /// Local time of the last autosave ("14:32"), for the status bar.
    pub last_autosave: Option<String>,
    /// Every snapshot of `list_profile`, oldest first (as the store keeps them).
    list: Vec<Snapshot>,
    list_profile: Option<String>,
    list_stale: bool,
    pub list_error: Option<String>,
    compare: Option<Compare>,
    /// Draw the diff overlay on the map.
    pub overlay_visible: bool,
    /// Layers left out of the overlay (the summary's checkboxes).
    pub overlay_hidden: BTreeSet<LayerRef>,
    /// Where "jump to" is in each layer's list of changes.
    pub jump_cursor: usize,
    pub prompt: Option<HistoryPrompt>,
    milestones: Option<Result<Vec<MilestoneInfo>, String>>,
    /// Bring the History panel to the front (scripts, the View menu).
    pub focus: bool,
    tx: Sender<HistoryEvent>,
    rx: Mutex<Receiver<HistoryEvent>>,
}

impl Default for HistoryState {
    fn default() -> Self {
        let (tx, rx) = channel();
        HistoryState {
            timer: AutosaveTimer::default(),
            autosave_in_flight: false,
            autosave_runs: 0,
            autosave_errors: Vec::new(),
            last_autosave: None,
            list: Vec::new(),
            list_profile: None,
            list_stale: true,
            list_error: None,
            compare: None,
            overlay_visible: true,
            overlay_hidden: BTreeSet::new(),
            jump_cursor: 0,
            prompt: None,
            milestones: None,
            focus: false,
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl HistoryState {
    /// Re-read the timeline on next use.
    pub fn invalidate(&mut self) {
        self.list_stale = true;
    }

    /// The snapshot selected for document `doc`, if any.
    pub fn selected_for(&self, doc: u64) -> Option<SnapshotId> {
        self.compare.as_ref().filter(|c| c.doc == doc).map(|c| c.id)
    }

    /// The overlay for document `doc` once computed.
    pub fn overlay_for(&self, doc: u64) -> Option<&DiffOverlay> {
        self.compare
            .as_ref()
            .filter(|c| c.doc == doc)
            .and_then(|c| c.overlay.as_ref())
            .map(|(_, o)| o)
    }

    /// Why the selected snapshot of `doc` could not be read, if it could not.
    pub fn compare_error(&self, doc: u64) -> Option<&str> {
        self.compare
            .as_ref()
            .filter(|c| c.doc == doc)
            .and_then(|c| c.snapshot.as_ref().err())
            .map(String::as_str)
    }

    pub fn modal_open(&self) -> bool {
        self.prompt.is_some()
    }
}

impl AppState {
    /// Drain finished autosaves and prunes. Called once per frame.
    pub fn poll_history(&mut self) {
        let events: Vec<HistoryEvent> = match self.history.rx.lock() {
            Ok(rx) => rx.try_iter().collect(),
            Err(_) => return,
        };
        for event in events {
            match event {
                HistoryEvent::Autosaved(results) => self.finish_autosave(results),
                HistoryEvent::Pruned { profile, result } => {
                    self.history.invalidate();
                    match result {
                        Ok(r) if r.snapshots_removed > 0 => self.info(format!(
                            "History of {profile}: retention removed {} snapshot(s), {} freed",
                            r.snapshots_removed,
                            human_size(r.bytes_freed)
                        )),
                        Ok(_) => {}
                        Err(e) => self.warn(format!("History of {profile}: cannot prune: {e}")),
                    }
                }
            }
        }
    }

    fn finish_autosave(&mut self, results: Vec<AutosaveResult>) {
        self.history.autosave_in_flight = false;
        self.history.autosave_runs += 1;
        self.history.autosave_errors.clear();
        self.history.invalidate();
        let mut saved = 0;
        for r in results {
            match r.result {
                Ok(_) => {
                    saved += 1;
                    if let Some(doc) = self.docs.iter_mut().find(|d| d.id == r.doc) {
                        doc.autosaved_revision = Some(r.revision);
                    }
                }
                Err(e) => {
                    let text = format!("Autosave of {} failed: {e}", r.path);
                    self.history.autosave_errors.push(text.clone());
                    self.warn(text);
                }
            }
        }
        if saved > 0 {
            let at = local_time(now_ms(), "%H:%M");
            self.info(format!("Autosaved {saved} map(s) to history at {at}"));
            self.history.last_autosave = Some(at);
        }
    }

    /// Advance the autosave timer by one frame.
    pub fn autosave_tick(&mut self, dt: Duration) {
        let minutes = self.config.autosave_minutes;
        if self.history.autosave_in_flight {
            return;
        }
        if self.history.timer.tick(dt, minutes) {
            self.autosave_now();
        }
    }

    /// Record every dirty map now (off the UI thread). Returns how many maps
    /// were handed to the worker.
    pub fn autosave_now(&mut self) -> usize {
        if self.history.autosave_in_flight {
            return 0;
        }
        let jobs = autosave::collect_jobs(&self.docs);
        let n = jobs.len();
        if n == 0 {
            return 0;
        }
        if autosave::spawn(self.paths.history.clone(), jobs, self.history.tx.clone()) {
            self.history.autosave_in_flight = true;
            n
        } else {
            self.warn("Autosave: cannot start a worker thread");
            0
        }
    }

    /// After a profile opened: apply the retention policy in the background.
    pub fn start_auto_prune(&mut self, profile: &str) {
        let (root, policy, tx) = (
            self.paths.history.clone(),
            self.config.retention(),
            self.history.tx.clone(),
        );
        let profile = profile.to_owned();
        let _ = std::thread::Builder::new()
            .name("history-prune".to_owned())
            .spawn(move || {
                let result = with_store(&root, &profile, |s| s.prune(&policy));
                let _ = tx.send(HistoryEvent::Pruned { profile, result });
            });
    }

    /// "Prune now" (and the `prune` script command): synchronous, for the
    /// open profile.
    pub fn prune_now(&mut self, policy: &Retention) -> Result<PruneReport, String> {
        let profile = self
            .active_profile_id()
            .map(str::to_owned)
            .ok_or("no profile is open")?;
        let report = with_store(&self.paths.history, &profile, |s| s.prune(policy))?;
        self.history.invalidate();
        Ok(report)
    }

    /// The profile whose history the panel shows: the active map's.
    fn history_profile(&self) -> Option<String> {
        self.active_doc()
            .map(|d| d.profile_id.clone())
            .or_else(|| self.active_profile_id().map(str::to_owned))
    }

    fn refresh_history_list(&mut self) {
        let profile = self.history_profile();
        if !self.history.list_stale && self.history.list_profile == profile {
            return;
        }
        self.history.list_stale = false;
        self.history.list_profile = profile.clone();
        let Some(profile) = profile else {
            self.history.list.clear();
            return;
        };
        match with_store(&self.paths.history, &profile, |s| Ok(s.all().to_vec())) {
            Ok(list) => {
                self.history.list = list;
                self.history.list_error = None;
            }
            Err(e) => {
                self.history.list.clear();
                self.history.list_error = Some(e);
            }
        }
    }

    /// A snapshot of the loaded timeline.
    pub fn history_list_entry(&self, id: SnapshotId) -> Option<Snapshot> {
        self.history.list.iter().find(|s| s.id == id).cloned()
    }

    /// The snapshots of document `index`, newest first.
    pub fn timeline_of(&mut self, index: usize) -> Vec<Snapshot> {
        self.refresh_history_list();
        let Some(doc) = self.docs.get(index) else {
            return Vec::new();
        };
        if self.history.list_profile.as_deref() != Some(doc.profile_id.as_str()) {
            return Vec::new();
        }
        let mut t: Vec<Snapshot> = self
            .history
            .list
            .iter()
            .filter(|s| s.game_path == doc.path)
            .cloned()
            .collect();
        t.reverse();
        t
    }

    /// "Checkpoint...": record the current document as a named checkpoint,
    /// without saving to `saves\`.
    pub fn checkpoint(&mut self, index: usize, name: &str) -> Result<Snapshot, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a checkpoint needs a name".to_owned());
        }
        let doc = self.docs.get(index).ok_or("no such map")?;
        let bytes = doc
            .edit
            .ds1()
            .to_bytes()
            .map_err(|e| format!("cannot serialise the map: {e}"))?;
        let (profile, path) = (doc.profile_id.clone(), doc.path.clone());
        let snap = with_store(&self.paths.history, &profile, |s| {
            s.record(
                &path,
                &bytes,
                SnapshotKind::Checkpoint {
                    name: name.to_owned(),
                },
            )
        })?;
        self.history.invalidate();
        Ok(snap)
    }

    fn doc_store_key(&self) -> Result<(usize, String, GamePath), String> {
        let index = self.active.ok_or("no map is open")?;
        let doc = &self.docs[index];
        Ok((index, doc.profile_id.clone(), doc.path.clone()))
    }

    pub fn label_snapshot(&mut self, id: SnapshotId, label: Option<String>) -> Result<(), String> {
        let (_, profile, _) = self.doc_store_key()?;
        let label = label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty());
        with_store(&self.paths.history, &profile, |s| s.set_label(id, label))?;
        self.history.invalidate();
        Ok(())
    }

    pub fn make_checkpoint(&mut self, id: SnapshotId, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a checkpoint needs a name".to_owned());
        }
        let (_, profile, _) = self.doc_store_key()?;
        with_store(&self.paths.history, &profile, |s| {
            s.make_checkpoint(id, name.to_owned())
        })?;
        self.history.invalidate();
        Ok(())
    }

    pub fn delete_snapshot(&mut self, id: SnapshotId) -> Result<(), String> {
        let (_, profile, _) = self.doc_store_key()?;
        with_store(&self.paths.history, &profile, |s| s.delete(id))?;
        if self.history.compare.as_ref().is_some_and(|c| c.id == id) {
            self.clear_snapshot_selection();
        }
        self.history.invalidate();
        Ok(())
    }

    /// Pick snapshot `id` of the active map for the diff (reads and parses it).
    pub fn select_snapshot(&mut self, id: SnapshotId) -> Result<(), String> {
        let (index, profile, _) = self.doc_store_key()?;
        let doc_id = self.docs[index].id;
        if self.history.selected_for(doc_id) == Some(id) {
            return Ok(());
        }
        self.clear_snapshot_selection();
        let snapshot = read_snapshot(&self.paths.history, &profile, id).and_then(|bytes| {
            Ds1::parse(&bytes)
                .map(Arc::new)
                .map_err(|e| format!("the snapshot does not parse: {e}"))
        });
        let error = snapshot.as_ref().err().cloned();
        self.history.compare = Some(Compare {
            doc: doc_id,
            id,
            snapshot,
            overlay: None,
        });
        self.history.jump_cursor = 0;
        self.history_frame();
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn clear_snapshot_selection(&mut self) {
        if let Some(c) = self.history.compare.take()
            && let Some(doc) = self.docs.iter_mut().find(|d| d.id == c.doc)
        {
            doc.show_current();
        }
    }

    /// Once per frame (and before script checks): drop a selection whose map
    /// was closed, and bring the diff overlay up to date with the map.
    pub fn history_frame(&mut self) {
        let Some(c) = self.history.compare.as_mut() else {
            return;
        };
        let Some(doc) = self.docs.iter_mut().find(|d| d.id == c.doc) else {
            self.history.compare = None;
            return;
        };
        let Ok(snapshot) = &c.snapshot else {
            return;
        };
        let revision = doc.edit.revision();
        if c.overlay.as_ref().is_none_or(|(r, _)| *r != revision) {
            let overlay = DiffOverlay::between(snapshot, doc.edit.ds1());
            c.overlay = Some((revision, overlay));
        }
    }

    /// "Show snapshot": draw the selected snapshot in place of the map
    /// (read-only), or go back to the current map.
    pub fn set_show_snapshot(&mut self, on: bool) -> Result<(), String> {
        let index = self.active.ok_or("no map is open")?;
        let doc_id = self.docs[index].id;
        if !on {
            self.docs[index].show_current();
            return Ok(());
        }
        let c = self
            .history
            .compare
            .as_ref()
            .filter(|c| c.doc == doc_id)
            .ok_or("select a snapshot first")?;
        let snapshot = c.snapshot.clone()?;
        let id = c.id;
        self.docs[index].show_snapshot(id, snapshot);
        Ok(())
    }

    /// Restore snapshot `id` into the active map (BeforeRestore first, then
    /// one undoable replace).
    pub fn restore_snapshot(&mut self, id: SnapshotId) -> Result<Snapshot, String> {
        let (index, profile, path) = self.doc_store_key()?;
        let bytes = read_snapshot(&self.paths.history, &profile, id)?;
        let label = self
            .history
            .list
            .iter()
            .find(|s| s.id == id)
            .map(|s| match display_name(s) {
                Some(n) => format!("Restore \"{n}\""),
                None => format!("Restore {}", kind_label(&s.kind).to_ascii_lowercase()),
            })
            .unwrap_or_else(|| "Restore snapshot".to_owned());
        self.docs[index].show_current();
        let doc = &mut self.docs[index];
        let before = restore_into(
            &self.paths.history,
            &profile,
            &path,
            &mut doc.edit,
            &bytes,
            &label,
        )?;
        doc.sync();
        self.history.invalidate();
        Ok(before)
    }

    /// "Milestone...": commit `capybara\saves\`. `Ok(None)` = nothing changed.
    pub fn milestone(&mut self, message: &str) -> Result<Option<String>, String> {
        let message = message.trim();
        if message.is_empty() {
            return Err("a milestone needs a message".to_owned());
        }
        let result = commit_milestone(&self.paths.saves, message);
        self.history.milestones = None;
        result
    }

    /// The milestone log (cached until the next milestone or refresh).
    pub fn milestones(&mut self) -> &Result<Vec<MilestoneInfo>, String> {
        let saves = self.paths.saves.clone();
        self.history
            .milestones
            .get_or_insert_with(|| milestone_log(&saves, MILESTONE_LOG_LIMIT))
    }

    pub fn refresh_milestones(&mut self) {
        self.history.milestones = None;
    }

    /// Centre the active map's view on cell `(x, y)`.
    pub fn jump_to_cell(&mut self, x: u32, y: u32) {
        if let Some(doc) = self.active_doc_mut() {
            doc.view.center = iso::tile_to_iso(f64::from(x) + 0.5, f64::from(y) + 0.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capy_edit::CellValue;
    use d2_formats::ds1::Cell;

    fn paths() -> (tempfile::TempDir, crate::paths::Paths) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = crate::paths::Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        (tmp, paths)
    }

    fn map() -> Ds1 {
        let mut ds1 = Ds1::new(5, 5, 0);
        ds1.floors[0].set(1, 1, Cell(1).with_indices(3, 2));
        ds1
    }

    fn paint(doc: &mut Document, x: u32) {
        let v = CellValue {
            cell: Cell(1).with_indices(7, 1).0,
            orientation: 0,
        };
        doc.paint(LayerRef::Floor(0), &[(x, 2, v)]).expect("paint");
    }

    #[test]
    fn restore_records_before_restore_first_then_replaces_in_one_step() {
        let (_tmp, paths) = paths();
        let game = GamePath::new("data\\global\\tiles\\a.ds1");
        let checkpoint = map().to_bytes().expect("bytes");
        let mut doc = Document::new(map());
        paint(&mut doc, 0);
        paint(&mut doc, 3);
        doc.mark_saved();
        let edited = doc.ds1().clone();
        let before = restore_into(&paths.history, "p", &game, &mut doc, &checkpoint, "Restore")
            .expect("restore");
        // 1. The current map was recorded as BeforeRestore...
        assert_eq!(before.kind, SnapshotKind::BeforeRestore);
        let store = SnapshotStore::open(&paths.history, "p").expect("store");
        assert_eq!(
            store.read(before.id).expect("blob"),
            edited.to_bytes().expect("bytes")
        );
        // 2. ...then the snapshot replaced it as one undoable, dirty step.
        assert_eq!(doc.ds1().to_bytes().expect("bytes"), checkpoint);
        assert!(doc.is_dirty());
        assert!(doc.undo());
        assert_eq!(doc.ds1(), &edited);
        assert!(!doc.is_dirty());
        // Nothing went to saves.
        assert_eq!(std::fs::read_dir(&paths.saves).expect("saves").count(), 0);
    }

    #[test]
    fn restore_changes_nothing_when_before_restore_cannot_be_recorded() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // A file where the history folder should be: recording must fail.
        let blocked = tmp.path().join("history");
        std::fs::write(&blocked, b"not a folder").expect("write");
        let game = GamePath::new("data\\global\\tiles\\a.ds1");
        let checkpoint = map().to_bytes().expect("bytes");
        let mut doc = Document::new(map());
        paint(&mut doc, 4);
        doc.mark_saved();
        let edited = doc.ds1().clone();
        let err = restore_into(&blocked, "p", &game, &mut doc, &checkpoint, "Restore")
            .expect_err("cannot record");
        assert!(err.contains("before restoring"), "{err}");
        assert_eq!(doc.ds1(), &edited, "the map is untouched");
        assert!(!doc.is_dirty());
        assert!(!doc.can_redo());
        // A snapshot that does not parse changes nothing either.
        assert!(restore_into(&tmp.path().join("h2"), "p", &game, &mut doc, b"junk", "R").is_err());
        assert_eq!(doc.ds1(), &edited);
    }

    #[test]
    fn timeline_lookup_by_position_and_name() {
        let snap = |id: u64, kind: SnapshotKind, label: Option<&str>| Snapshot {
            id: SnapshotId(id),
            game_path: GamePath::new("data\\a.ds1"),
            hash: capy_history::ContentHash("0".repeat(64)),
            size: 1,
            created_unix_ms: 0,
            kind,
            label: label.map(str::to_owned),
        };
        let newest_first = vec![
            snap(3, SnapshotKind::Save, Some("good one")),
            snap(
                2,
                SnapshotKind::Checkpoint {
                    name: "before-wall".into(),
                },
                None,
            ),
            snap(1, SnapshotKind::Autosave, None),
        ];
        assert_eq!(find_in_timeline(&newest_first, "0"), Some(SnapshotId(3)));
        assert_eq!(find_in_timeline(&newest_first, "2"), Some(SnapshotId(1)));
        assert_eq!(
            find_in_timeline(&newest_first, "before-wall"),
            Some(SnapshotId(2))
        );
        assert_eq!(
            find_in_timeline(&newest_first, "good one"),
            Some(SnapshotId(3))
        );
        assert_eq!(find_in_timeline(&newest_first, "9"), None);
        assert_eq!(find_in_timeline(&newest_first, "nope"), None);
        assert_eq!(kind_slug(&newest_first[1].kind), "checkpoint");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
    }

    #[test]
    fn milestones_report_nothing_changed() {
        let (_tmp, paths) = paths();
        assert!(milestone_log(&paths.saves, 10).expect("log").is_empty());
        assert!(
            !paths.saves.join(".git").exists(),
            "looking creates nothing"
        );
        let file = paths.saves.join("p").join("a.ds1");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, b"one").expect("write");
        let first = commit_milestone(&paths.saves, "first").expect("commit");
        assert!(first.is_some());
        assert_eq!(
            commit_milestone(&paths.saves, "again").expect("commit"),
            None
        );
        std::fs::write(&file, b"two").expect("write");
        assert!(
            commit_milestone(&paths.saves, "second")
                .expect("commit")
                .is_some()
        );
        let log = milestone_log(&paths.saves, 10).expect("log");
        let messages: Vec<&str> = log.iter().map(|m| m.message.trim()).collect();
        assert_eq!(messages, vec!["second", "first"], "newest first");
    }

    #[test]
    fn local_times_format() {
        let t = local_time(now_ms(), "%H:%M");
        assert_eq!(t.len(), 5, "{t}");
        assert_eq!(t.as_bytes()[2], b':');
        assert_eq!(local_time(i64::MAX, "%H:%M"), "?");
    }
}
