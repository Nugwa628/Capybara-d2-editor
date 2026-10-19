//! Keyboard shortcuts. Read once per frame, before the panels, and skipped
//! while a text field has focus or a modal dialog is open.
//!
//! | Keys | Action |
//! |---|---|
//! | Ctrl+S | save the active map |
//! | Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z | undo, redo |
//! | Ctrl+C, Ctrl+X, Ctrl+V | copy, cut, paste preview |
//! | Ctrl+A, Ctrl+D | select all on the active layer, deselect |
//! | Delete | Path tool: delete the selected path point; else delete selected objects, else erase the selected cells |
//! | Esc | cancel the paste preview, the gesture, or a dialog; else deselect |
//! | M L T S P E I O | tools: box, lasso, tile ID, identical, paint, erase, eyedropper, objects |
//! | G | grid |
//! | Space + drag, middle drag, arrows, wheel | pan and zoom (Map panel) |

use bevy_egui::egui::{self, Key, Modifiers};
use capy_edit::{LayerRef, SelectOp};

use super::map_tools;
use crate::edit::ops::EditOp;
use crate::edit::tools::Tool;
use crate::objects::ObjectMode;
use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Save,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Deselect,
    Delete,
    Escape,
    CommitPaste,
    Tool(Tool),
    Grid,
}

const TOOL_KEYS: [(Key, Tool); 8] = [
    (Key::M, Tool::SelectBox),
    (Key::L, Tool::Lasso),
    (Key::T, Tool::SelectById),
    (Key::S, Tool::SelectIdentical),
    (Key::P, Tool::Paint),
    (Key::E, Tool::Erase),
    (Key::I, Tool::Eyedropper),
    (Key::O, Tool::Objects),
];

fn read(ctx: &egui::Context, paste_active: bool) -> Vec<Action> {
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        // Shifted variants first: a shortcut with fewer modifiers also
        // matches when more are held.
        if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) {
            out.push(Action::Redo);
        }
        if i.consume_key(Modifiers::COMMAND, Key::Z) {
            out.push(Action::Undo);
        }
        if i.consume_key(Modifiers::COMMAND, Key::Y) {
            out.push(Action::Redo);
        }
        if i.consume_key(Modifiers::COMMAND, Key::S) {
            out.push(Action::Save);
        }
        if i.consume_key(Modifiers::COMMAND, Key::C) {
            out.push(Action::Copy);
        }
        if i.consume_key(Modifiers::COMMAND, Key::X) {
            out.push(Action::Cut);
        }
        if i.consume_key(Modifiers::COMMAND, Key::V) {
            out.push(Action::Paste);
        }
        if i.consume_key(Modifiers::COMMAND, Key::A) {
            out.push(Action::SelectAll);
        }
        if i.consume_key(Modifiers::COMMAND, Key::D) {
            out.push(Action::Deselect);
        }
        if i.consume_key(Modifiers::NONE, Key::Delete) {
            out.push(Action::Delete);
        }
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            out.push(Action::Escape);
        }
        if paste_active && i.consume_key(Modifiers::NONE, Key::Enter) {
            out.push(Action::CommitPaste);
        }
        if i.modifiers.is_none() {
            for (key, tool) in TOOL_KEYS {
                if i.key_pressed(key) {
                    out.push(Action::Tool(tool));
                }
            }
            if i.key_pressed(Key::G) {
                out.push(Action::Grid);
            }
        }
    });
    out
}

/// Actions that change the map, which a history snapshot shown in place of
/// the map (read-only) refuses, like the mouse tools do.
fn edits_the_map(action: Action) -> bool {
    matches!(
        action,
        Action::Undo
            | Action::Redo
            | Action::Cut
            | Action::Paste
            | Action::Delete
            | Action::CommitPaste
    )
}

/// The toast a refused keyboard edit shows.
pub const SNAPSHOT_READ_ONLY: &str = "Viewing a snapshot: turn it off to edit";

/// True (after a toast) when the active map shows a history snapshot.
pub fn refuse_in_snapshot(state: &mut AppState) -> bool {
    if state.active_doc().is_some_and(|d| d.snapshot_view.is_some()) {
        state.notify(SNAPSHOT_READ_ONLY);
        return true;
    }
    false
}

pub fn handle(ctx: &egui::Context, state: &mut AppState) {
    if state.edit.modal_open()
        || state.history.modal_open()
        || state.settings.modal_open()
        || ctx.text_edit_focused()
    {
        return;
    }
    for action in read(ctx, state.edit.paste.is_some()) {
        apply(state, action);
    }
}

fn apply(state: &mut AppState, action: Action) {
    if edits_the_map(action) && refuse_in_snapshot(state) {
        if action == Action::Paste || action == Action::CommitPaste {
            state.cancel_paste();
        }
        return;
    }
    match action {
        Action::Save => state.save_active(),
        Action::Undo => state.undo(),
        Action::Redo => state.redo(),
        Action::Copy => state.copy_selection(),
        Action::Cut => state.cut_selection(),
        Action::Paste => state.begin_paste(),
        Action::SelectAll => select_all(state),
        Action::Deselect => state.deselect(),
        Action::Delete => delete(state),
        Action::CommitPaste => {
            if state.edit.paste.is_some() {
                state.commit_paste();
            }
        }
        Action::Tool(tool) => {
            map_tools::cancel_gesture(state);
            state.edit.tool = tool;
            if tool == Tool::Objects {
                state.objects.focus = true;
            }
        }
        Action::Grid => state.edit.grid = !state.edit.grid,
        Action::Escape => {
            if state.edit.paste.is_some() {
                state.cancel_paste();
            } else if map_tools::cancel_gesture(state) {
            } else if state.edit.move_dialog.is_some() || state.edit.resize_dialog.is_some() {
                state.edit.move_dialog = None;
                state.edit.resize_dialog = None;
            } else {
                state.deselect();
            }
        }
    }
}

/// Select every non-empty cell of the active layer.
pub fn select_all(state: &mut AppState) {
    let Some(doc) = state.active_doc_mut() else {
        return;
    };
    let layer: LayerRef = doc.active_layer;
    let rect = capy_edit::CellRect::new(
        (0, 0),
        (
            doc.ds1.width.saturating_sub(1),
            doc.ds1.height.saturating_sub(1),
        ),
    );
    doc.selection
        .box_select(&doc.ds1, rect, &[layer], false, SelectOp::Replace);
}

/// Delete: the selected path point (Path tool), else the selected objects,
/// else the selected cells.
pub fn delete(state: &mut AppState) {
    let Some(index) = state.active else { return };
    if state.edit.tool == Tool::Objects && state.objects.mode == ObjectMode::Path {
        let len = state.selected_path().len();
        if let Some(cmd) = state.objects.path.delete(len) {
            state.run_path_command(cmd);
        }
        return;
    }
    let doc = &state.docs[index];
    if !doc.object_sel.is_empty() {
        let indices = doc.object_sel.clone();
        state.exec(index, EditOp::DeleteObjects(indices));
    } else if !doc.selection.is_empty() {
        state.exec(index, EditOp::Erase);
    }
}

#[cfg(test)]
mod tests {
    use capy_edit::CellValue;
    use d2_assets::GamePath;
    use d2_formats::ds1::{Cell, Ds1};

    use super::*;
    use crate::config::Config;
    use crate::doc::{MapDoc, SourceCategory, SourceInfo};
    use crate::paths::Paths;

    fn state_with_edit() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut state = AppState::new(
            Paths::at(tmp.path().join("capybara")),
            Config::default(),
            Vec::new(),
        );
        let mut ds1 = Ds1::new(4, 4, 0);
        ds1.floors[0].set(1, 1, Cell(1).with_indices(1, 0));
        state.docs.push(MapDoc::new(
            1,
            "p".into(),
            GamePath::new("data\\a.ds1"),
            SourceInfo {
                category: SourceCategory::Base,
                layer_name: "x".into(),
            },
            ds1,
            &Config::default().layer_visibility,
        ));
        state.active = Some(0);
        let value = CellValue {
            cell: Cell(1).with_indices(2, 0).0,
            orientation: 0,
        };
        assert!(state.exec(
            0,
            EditOp::Paint {
                layer: LayerRef::Floor(0),
                cells: vec![(2, 2, value)],
            }
        ));
        (state, tmp)
    }

    #[test]
    fn keyboard_edits_are_refused_while_a_snapshot_is_shown() {
        let (mut state, _tmp) = state_with_edit();
        state.docs[0].snapshot_view = Some(capy_history::SnapshotId(1));
        state.docs[0].object_sel = Vec::new();
        state.docs[0].selection.insert(LayerRef::Floor(0), 1, 1);
        let revision = state.docs[0].edit.revision();
        for action in [
            Action::Undo,
            Action::Redo,
            Action::Cut,
            Action::Delete,
            Action::Paste,
        ] {
            apply(&mut state, action);
        }
        assert_eq!(state.docs[0].edit.revision(), revision, "nothing changed");
        assert!(state.edit.paste.is_none());
        assert!(
            state
                .toasts
                .iter()
                .any(|t| t.text == SNAPSHOT_READ_ONLY),
            "the user is told why"
        );
        // Non-editing keys still work: tools, grid, deselect.
        apply(&mut state, Action::Tool(Tool::Paint));
        assert_eq!(state.edit.tool, Tool::Paint);
        apply(&mut state, Action::Deselect);
        assert!(state.docs[0].selection.is_empty());
        // Back on the map, Ctrl+Z works again.
        state.docs[0].show_current();
        apply(&mut state, Action::Undo);
        assert_ne!(state.docs[0].edit.revision(), revision);
        assert!(!state.docs[0].edit.can_undo());
    }

    #[test]
    fn only_map_changing_actions_are_blocked() {
        assert!(edits_the_map(Action::Undo) && edits_the_map(Action::Delete));
        assert!(edits_the_map(Action::Paste) && edits_the_map(Action::CommitPaste));
        assert!(!edits_the_map(Action::Copy));
        assert!(!edits_the_map(Action::Save));
        assert!(!edits_the_map(Action::Escape));
        assert!(!edits_the_map(Action::Tool(Tool::Objects)));
    }
}
