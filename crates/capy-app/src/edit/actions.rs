//! What the UI and the scripts ask the application to do to the open maps:
//! run an edit (and offer an upgrade when the DS1 version is too old), undo,
//! copy and paste, save, and close with the Save / Discard / Cancel question.

use capy_edit::{Clipboard, LayerRef, PasteMode, Selection};

use super::clip;
use super::ops::{self, EditOp, OpOutcome, OpResult};
use super::paste::PasteSession;
use super::prompt::{
    CloseChoice, ClosePlan, ClosePrompt, CloseResolution, CloseTarget, DocFacts, plan_close,
    plan_quit, resolve,
};
use super::save::{self, SaveError};
use super::state::{CachedPreview, PendingUpgrade, SaveProblem};
use crate::doc::MapDoc;
use crate::iso;
use crate::state::{AppState, LogLevel, Toast};

/// How a save ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Saved,
    /// The map needs a newer DS1 version; the user was asked.
    NeedsUpgrade,
    Failed,
}

fn facts(docs: &[MapDoc]) -> Vec<DocFacts> {
    docs.iter()
        .map(|d| DocFacts {
            id: d.id,
            name: d.path.file_name().to_owned(),
            dirty: d.is_dirty(),
        })
        .collect()
}

impl AppState {
    /// An informational toast (and a log line).
    pub fn notify(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.log(LogLevel::Info, text.clone());
        self.toasts.push(Toast {
            level: LogLevel::Info,
            text,
            shown_at: None,
        });
    }

    /// Copy every document's edits to its render copy.
    pub fn sync_docs(&mut self) {
        for doc in &mut self.docs {
            doc.sync();
        }
    }

    pub fn doc_index(&self, id: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.id == id)
    }

    // ───────── running edits ─────────

    /// Runs `op` on document `index`. A map that is too old for it opens the
    /// upgrade question; other failures are reported. True when it was done.
    pub fn exec(&mut self, index: usize, op: EditOp) -> bool {
        let Some(doc) = self.docs.get_mut(index) else {
            return false;
        };
        doc.sync_clip_context();
        let result = ops::run(&mut doc.edit, &doc.selection, &op);
        self.finish_op(index, op, result)
    }

    fn finish_op(&mut self, index: usize, op: EditOp, result: OpResult) -> bool {
        let Some(doc) = self.docs.get_mut(index) else {
            return false;
        };
        doc.sync();
        let id = doc.id;
        match result {
            OpResult::Done(outcome) => {
                self.after_outcome(index, &op, outcome);
                true
            }
            OpResult::NeedsVersion { current, required } => {
                self.edit.upgrade = Some(PendingUpgrade {
                    doc: id,
                    op,
                    current,
                    required,
                });
                false
            }
            OpResult::Failed(e) => {
                self.error(format!("{}: {e}", op.name()));
                false
            }
        }
    }

    fn after_outcome(&mut self, index: usize, op: &EditOp, outcome: OpOutcome) {
        match outcome {
            OpOutcome::Moved(report) => {
                if let EditOp::MoveToLayer { from, to, .. } = op {
                    self.follow_move(index, *from, *to);
                    self.notify(ops::move_summary(*from, *to, &report));
                }
            }
            OpOutcome::Pasted(preview) => self.notify(ops::paste_summary(&preview)),
            OpOutcome::Cut(clip) => {
                let cells = clip.cells.len();
                self.set_clipboard(clip);
                self.notify(format!("Cut {cells} cell(s)"));
            }
            OpOutcome::Done => {
                if let Some(doc) = self.docs.get_mut(index)
                    && matches!(op, EditOp::DeleteObjects(_))
                {
                    doc.object_sel.clear();
                }
            }
            OpOutcome::Count { changed, skipped } => {
                let mut text = format!("{}: {changed} cell(s) changed", op.name());
                if skipped > 0 {
                    text.push_str(&format!(", {skipped} left alone (no matching choice)"));
                }
                self.notify(text);
            }
            OpOutcome::Added(object) => {
                if let Some(doc) = self.docs.get_mut(index) {
                    doc.object_sel = vec![object];
                }
                self.objects.path.follow_object(Some(object));
            }
        }
    }

    /// After a layer move the selection follows the cells to their new layer.
    fn follow_move(&mut self, index: usize, from: LayerRef, to: LayerRef) {
        let Some(doc) = self.docs.get_mut(index) else {
            return;
        };
        let mut next = Selection::new();
        for (layer, x, y) in doc.selection.iter() {
            let moved = layer == from
                && from
                    .get(doc.edit.ds1(), x, y)
                    .is_none_or(|v| from.is_empty_value(v));
            if moved {
                next.insert(to, x, y);
            } else {
                next.insert(layer, x, y);
            }
        }
        doc.selection = next;
    }

    /// The user agreed to upgrade: do it and try the operation again. The
    /// question comes back if the retry asks for yet another version.
    pub fn accept_upgrade(&mut self) {
        let Some(pending) = self.edit.upgrade.take() else {
            return;
        };
        let Some(index) = self.doc_index(pending.doc) else {
            return;
        };
        let doc = &mut self.docs[index];
        let result =
            ops::upgrade_and_retry(&mut doc.edit, &doc.selection, &pending.op, pending.required);
        let name = pending.op.name();
        if self.finish_op(index, pending.op, result) {
            self.info(format!("{name}: map upgraded to v{}", pending.required));
        }
    }

    pub fn cancel_upgrade(&mut self) {
        self.edit.upgrade = None;
    }

    /// Runs `op`, upgrading the map by itself when it asks for a newer
    /// version (scripts, where nobody can be asked). Logs each upgrade.
    pub fn exec_upgrading(&mut self, index: usize, op: EditOp) -> bool {
        let Some(doc) = self.docs.get_mut(index) else {
            return false;
        };
        doc.sync_clip_context();
        let (result, steps) = ops::run_upgrading(&mut doc.edit, &doc.selection, &op);
        for v in steps {
            self.info(format!("{}: upgraded the map to v{v}", op.name()));
        }
        self.finish_op(index, op, result)
    }

    // ───────── undo / redo ─────────

    pub fn undo(&mut self) {
        if let Some(doc) = self.active_doc_mut() {
            doc.edit.undo();
            doc.sync();
        }
    }

    pub fn redo(&mut self) {
        if let Some(doc) = self.active_doc_mut() {
            doc.edit.redo();
            doc.sync();
        }
    }

    // ───────── selection helpers ─────────

    /// Clear the selection of the active map.
    pub fn deselect(&mut self) {
        if let Some(doc) = self.active_doc_mut() {
            doc.selection.clear();
            doc.object_sel.clear();
        }
    }

    // ───────── clipboard ─────────

    fn set_clipboard(&mut self, clip: Clipboard) {
        if self.edit.use_os_clipboard
            && let Err(e) = clip::write_os(&clip.to_json())
        {
            self.warn(format!("Cannot reach the OS clipboard: {e}"));
        }
        self.edit.clipboard = Some(clip);
    }

    /// Ctrl+C: copy the selection of the active map.
    pub fn copy_selection(&mut self) {
        let Some(doc) = self.active_doc_mut() else {
            return;
        };
        if doc.selection.is_empty() {
            self.notify("Nothing selected to copy");
            return;
        }
        doc.sync_clip_context();
        let clip = doc.edit.copy(&doc.selection);
        let (cells, objects) = (clip.cells.len(), clip.objects.len());
        self.set_clipboard(clip);
        self.notify(format!("Copied {cells} cell(s) and {objects} object(s)"));
    }

    /// Ctrl+X.
    pub fn cut_selection(&mut self) {
        let Some(index) = self.active else { return };
        if self.docs[index].selection.is_empty() {
            self.notify("Nothing selected to cut");
            return;
        }
        self.exec(index, EditOp::Cut);
    }

    /// Ctrl+V: start the floating paste preview with the clip from the OS
    /// clipboard (another instance may have put it there) or the app's own.
    pub fn begin_paste(&mut self) {
        let Some(centre) = self.active_doc().map(|d| d.view.center) else {
            return;
        };
        let os_text = if self.edit.use_os_clipboard {
            clip::read_os()
        } else {
            None
        };
        let Some((clip, from_os)) = clip::choose(os_text.as_deref(), self.edit.clipboard.as_ref())
        else {
            self.notify("The clipboard holds no map cells");
            return;
        };
        let at = self
            .hover_cell
            .unwrap_or_else(|| iso::screen_to_cell(centre));
        self.edit.paste = Some(PasteSession::new(clip, at, self.edit.paste_mode, from_os));
        self.edit.paste_cache = None;
    }

    /// Click while previewing: write the clip into the active map.
    pub fn commit_paste(&mut self) {
        let Some(session) = self.edit.paste.take() else {
            return;
        };
        let Some(index) = self.active else { return };
        self.edit.paste_mode = session.mode;
        self.exec(
            index,
            EditOp::Paste {
                clip: Box::new(session.clip),
                at: session.at,
                mode: session.mode,
            },
        );
    }

    pub fn cancel_paste(&mut self) {
        self.edit.paste = None;
        self.edit.paste_cache = None;
    }

    /// The preview of the floating paste for the active map, recomputed only
    /// when the map, the position or the mode changed.
    pub fn paste_preview(&mut self) -> Option<capy_edit::PastePreview> {
        let session = self.edit.paste.as_ref()?;
        let index = self.active?;
        let doc = self.docs.get_mut(index)?;
        let key = (doc.id, doc.edit.revision(), session.at, session.mode);
        if let Some(c) = &self.edit.paste_cache
            && c.key == key
        {
            return Some(c.preview.clone());
        }
        doc.sync_clip_context();
        let preview = doc
            .edit
            .preview_paste(&session.clip, session.at, session.mode);
        self.edit.paste_cache = Some(CachedPreview {
            key,
            preview: preview.clone(),
        });
        Some(preview)
    }

    pub fn set_paste_mode(&mut self, mode: PasteMode) {
        if let Some(s) = &mut self.edit.paste {
            s.mode = mode;
        }
        self.edit.paste_mode = mode;
    }

    // ───────── saving ─────────

    /// Entries in CHANGED.txt of the open profile, re-read from disk.
    pub fn refresh_changed(&mut self) {
        self.edit.changed = match self.active_profile_id() {
            Some(id) => save::changed_count(&self.paths, id),
            None => 0,
        };
    }

    /// Ctrl+S: write document `index` to `capybara\saves\<profile>\`.
    pub fn save_doc(&mut self, index: usize) -> SaveStatus {
        let Some(doc) = self.docs.get(index) else {
            return SaveStatus::Failed;
        };
        let (id, profile, path) = (doc.id, doc.profile_id.clone(), doc.path.clone());
        match save::save_ds1(&self.paths, &profile, &path, doc.edit.ds1()) {
            Ok(report) => {
                if let Some(doc) = self.docs.get_mut(index) {
                    doc.edit.mark_saved();
                }
                for w in &report.warnings {
                    self.warn(format!("{path}: {w}"));
                }
                self.edit.changed = report.changed_total;
                self.history.invalidate();
                self.after_save(index, &path);
                self.notify(format!(
                    "Saved {path} ({} bytes). CHANGED.txt lists {} file(s).",
                    report.bytes, report.changed_total
                ));
                SaveStatus::Saved
            }
            Err(SaveError::NotRepresentable {
                detail,
                min_version,
            }) => {
                self.edit.save_problem = Some(SaveProblem {
                    doc: id,
                    detail,
                    min_version,
                    then: None,
                });
                SaveStatus::NeedsUpgrade
            }
            Err(SaveError::Failed(text)) => {
                self.error(format!("Cannot save {path}: {text}"));
                SaveStatus::Failed
            }
        }
    }

    /// The map now has an edited copy in the profile's saves: show it as
    /// such, and bring the asset index up to date in the background.
    fn after_save(&mut self, index: usize, path: &d2_assets::GamePath) {
        if let Some(info) = self.source_of(path)
            && let Some(doc) = self.docs.get_mut(index)
        {
            doc.source = info;
        }
        self.maps_list.source_cache.clear();
        self.start_saves_refresh();
    }

    pub fn save_active(&mut self) {
        if let Some(i) = self.active {
            self.save_doc(i);
        }
    }

    /// The user chose to upgrade the map so it can be saved.
    pub fn accept_save_upgrade(&mut self) {
        let Some(problem) = self.edit.save_problem.take() else {
            return;
        };
        let Some(index) = self.doc_index(problem.doc) else {
            return;
        };
        if !self.exec(index, EditOp::UpgradeVersion(problem.min_version)) {
            return;
        }
        if self.save_doc(index) == SaveStatus::Saved
            && let Some(target) = problem.then
        {
            self.request_close_target(target);
        }
    }

    // ───────── "mark as packed" ─────────

    pub fn confirm_mark_packed(&mut self) {
        self.edit.confirm_packed = false;
        let Some(profile) = self.active_profile_id().map(str::to_owned) else {
            return;
        };
        match save::mark_packed(&self.paths, &profile) {
            Ok(n) => {
                self.edit.changed = 0;
                self.notify(format!(
                    "CHANGED.txt cleared ({n} file(s) marked as packed)"
                ));
            }
            Err(e) => self.error(format!("Cannot clear CHANGED.txt: {e}")),
        }
    }

    // ───────── closing ─────────

    /// Close tab `index`, asking first when it has unsaved changes.
    pub fn request_close(&mut self, index: usize) {
        if let Some(id) = self.docs.get(index).map(|d| d.id) {
            self.request_close_target(CloseTarget::Doc(id));
        }
    }

    /// Quit, asking first when any map has unsaved changes.
    pub fn request_quit(&mut self) {
        // The sprite editor's unsaved work is asked about first.
        if self.sprites.editor_dirty() && !self.sprites.quit_confirmed {
            self.sprites.quit_prompt = true;
            return;
        }
        self.request_close_target(CloseTarget::Quit);
    }

    fn request_close_target(&mut self, target: CloseTarget) {
        let docs = facts(&self.docs);
        let plan = match target {
            CloseTarget::Doc(id) => plan_close(&docs, id),
            CloseTarget::Quit => plan_quit(&docs),
        };
        match plan {
            ClosePlan::CloseNow => self.perform_close(target),
            ClosePlan::Ask(prompt) => self.edit.close_prompt = Some(prompt),
        }
    }

    fn perform_close(&mut self, target: CloseTarget) {
        match target {
            CloseTarget::Doc(id) => {
                if let Some(index) = self.doc_index(id) {
                    self.close_doc(index);
                }
            }
            CloseTarget::Quit => self.quit_requested = true,
        }
    }

    /// The user answered the Save / Discard / Cancel question.
    pub fn answer_close(&mut self, choice: CloseChoice) {
        let Some(prompt): Option<ClosePrompt> = self.edit.close_prompt.take() else {
            return;
        };
        match resolve(&prompt, choice) {
            CloseResolution::Stay => {}
            CloseResolution::Close { target } => self.perform_close(target),
            CloseResolution::SaveThenClose { save, target } => {
                for id in save {
                    let Some(index) = self.doc_index(id) else {
                        continue;
                    };
                    match self.save_doc(index) {
                        SaveStatus::Saved => {}
                        SaveStatus::NeedsUpgrade => {
                            if let Some(p) = &mut self.edit.save_problem {
                                p.then = Some(target);
                            }
                            return;
                        }
                        SaveStatus::Failed => return,
                    }
                }
                self.perform_close(target);
            }
        }
    }
}
