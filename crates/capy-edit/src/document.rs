//! A DS1 plus its undo/redo history.
//!
//! Every mutating method builds one [`Patch`], applies it and pushes it
//! (clearing redo). A method that fails changes nothing. A method that would
//! change nothing returns [`EditError::Empty`] and records no history entry
//! (`move_to_layer` returns its report instead when it skipped occupied cells).

use d2_formats::ds1::Ds1;

use crate::error::{EditError, Result};
use crate::patch::Patch;

mod clip;
mod edits;
mod objects;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveMode {
    Overwrite,
    SkipOccupied,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveReport {
    pub moved: usize,
    pub skipped_occupied: usize,
    /// The requested target layer, when it had to be created.
    pub layer_added: Option<crate::layer::LayerRef>,
}

#[derive(Clone, Debug)]
struct OpenGroup {
    depth: u32,
    patch: Patch,
}

/// A DS1 plus its undo/redo history.
#[derive(Clone, Debug)]
pub struct Document {
    ds1: Ds1,
    undo: Vec<Patch>,
    redo: Vec<Patch>,
    revision: u64,
    /// The model at the last `mark_saved`.
    saved: Ds1,
    /// `undo.len()` at the last `mark_saved`, while that state is still on the
    /// history path (a fast path for `is_dirty`; `None` when unknown).
    saved_depth: Option<usize>,
    group: Option<OpenGroup>,
    game_path: Option<String>,
    lvl_type: Option<u32>,
}

impl Document {
    pub fn new(ds1: Ds1) -> Document {
        Document {
            saved: ds1.clone(),
            ds1,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            saved_depth: Some(0),
            group: None,
            game_path: None,
            lvl_type: None,
        }
    }

    pub fn ds1(&self) -> &Ds1 {
        &self.ds1
    }

    pub fn into_ds1(self) -> Ds1 {
        self.ds1
    }

    /// Where this map comes from and its LvlTypes id, so a clipboard knows its
    /// source and a paste can warn about a mismatch. Not part of the history.
    pub fn set_clip_context(&mut self, game_path: Option<String>, lvl_type: Option<u32>) {
        self.game_path = game_path;
        self.lvl_type = lvl_type;
    }

    // ----- history ---------------------------------------------------------

    pub fn undo(&mut self) -> bool {
        self.end_all_groups();
        let Some(patch) = self.undo.pop() else {
            return false;
        };
        if patch.revert(&mut self.ds1).is_err() {
            self.undo.push(patch);
            return false;
        }
        self.revision += 1;
        self.redo.push(patch);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_all_groups();
        let Some(patch) = self.redo.pop() else {
            return false;
        };
        if patch.apply(&mut self.ds1).is_err() {
            self.redo.push(patch);
            return false;
        }
        self.revision += 1;
        self.undo.push(patch);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(Patch::label)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(Patch::label)
    }

    /// Monotonic; bumps on every apply/undo/redo. The renderer re-uploads on change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Dirty = the current state differs from the last `mark_saved` point
    /// (undoing back to the saved state makes it clean again).
    pub fn is_dirty(&self) -> bool {
        if self.group.is_none() && self.saved_depth == Some(self.undo.len()) {
            return false;
        }
        self.ds1 != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.ds1.clone();
        self.saved_depth = self.group.is_none().then_some(self.undo.len());
    }

    /// Merges every patch pushed until the matching `end_group` into one undo
    /// step (a brush stroke). Groups nest; only the outermost label is kept.
    pub fn begin_group(&mut self, label: &str) {
        match &mut self.group {
            Some(g) => g.depth += 1,
            None => {
                self.group = Some(OpenGroup {
                    depth: 1,
                    patch: Patch::new(label),
                });
            }
        }
    }

    pub fn end_group(&mut self) {
        let Some(g) = &mut self.group else {
            return;
        };
        if g.depth > 1 {
            g.depth -= 1;
            return;
        }
        self.finish_group();
    }

    fn end_all_groups(&mut self) {
        if self.group.is_some() {
            self.finish_group();
        }
    }

    fn finish_group(&mut self) {
        if let Some(g) = self.group.take()
            && !g.patch.is_empty()
        {
            self.undo.push(g.patch);
        }
    }

    /// Apply a prebuilt patch (used by tests and by restore-from-snapshot).
    pub fn apply_patch(&mut self, patch: Patch) -> Result<()> {
        self.commit(patch)
    }

    // ----- internals -------------------------------------------------------

    /// Applies `patch` and records it. An empty patch is `EditError::Empty`.
    fn commit(&mut self, patch: Patch) -> Result<()> {
        if patch.is_empty() {
            return Err(EditError::Empty);
        }
        patch.apply(&mut self.ds1)?;
        self.record(patch);
        Ok(())
    }

    /// Records a patch that has just been applied.
    fn record(&mut self, patch: Patch) {
        self.revision += 1;
        if !self.redo.is_empty() {
            self.redo.clear();
            if self.saved_depth.is_some_and(|d| d > self.undo.len()) {
                self.saved_depth = None;
            }
        }
        match &mut self.group {
            Some(g) => g.patch.absorb(patch),
            None => self.undo.push(patch),
        }
    }
}
