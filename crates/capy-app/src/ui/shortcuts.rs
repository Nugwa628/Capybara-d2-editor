//! Keyboard shortcuts. Read once per frame, before the panels, and skipped
//! while a text field has focus or a modal dialog is open.
//!
//! | Keys | Action |
//! |---|---|
//! | Ctrl+S | save the active map |
//! | Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z | undo, redo |
//! | Ctrl+C, Ctrl+X, Ctrl+V | copy, cut, paste preview |
//! | Ctrl+A, Ctrl+D | select all on the active layer, deselect |
//! | Delete | delete selected objects, else erase the selected cells |
//! | Esc | cancel the paste preview, the gesture, or a dialog; else deselect |
//! | M L T S P E I O | tools: box, lasso, tile ID, identical, paint, erase, eyedropper, objects |
//! | G | grid |
//! | Space + drag, middle drag, arrows, wheel | pan and zoom (Map panel) |

use bevy_egui::egui::{self, Key, Modifiers};
use capy_edit::{LayerRef, SelectOp};

use super::map_tools;
use crate::edit::ops::EditOp;
use crate::edit::tools::Tool;
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

/// Delete: the selected objects, else the selected cells.
pub fn delete(state: &mut AppState) {
    let Some(index) = state.active else { return };
    let doc = &state.docs[index];
    if !doc.object_sel.is_empty() {
        let indices = doc.object_sel.clone();
        state.exec(index, EditOp::DeleteObjects(indices));
    } else if !doc.selection.is_empty() {
        state.exec(index, EditOp::Erase);
    }
}
