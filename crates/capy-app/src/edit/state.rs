//! Editor-wide state that is not part of one document: the current tool,
//! the brush value and its recent list, the clipboard, the floating paste,
//! the open dialogs and the gesture in progress.

use std::collections::HashMap;

use bevy_egui::egui;
use capy_edit::{CellValue, Clipboard, LayerRef, MoveMode, PasteMode, PastePreview, SelectOp};
use d2_formats::palette::Palette;

use super::ops::EditOp;
use super::paste::PasteSession;
use super::prompt::{ClosePrompt, CloseTarget};
use super::tools::{BoxDrag, LassoPath, SelectScope, Tool};

/// How many picked values the "Recent tiles" strip keeps.
pub const MAX_RECENT: usize = 16;

/// A mouse gesture that has started and not yet ended. Each variant names
/// the document (by id) it belongs to, so switching tabs mid-gesture cannot
/// apply it to the wrong map.
#[derive(Clone, Debug)]
pub enum Gesture {
    Box {
        doc: u64,
        drag: BoxDrag,
        op: SelectOp,
    },
    Lasso {
        doc: u64,
        path: LassoPath,
        op: SelectOp,
    },
    /// A paint or erase stroke; the document's undo group is open.
    Brush {
        doc: u64,
        layer: LayerRef,
        last: (i32, i32),
        erase: bool,
    },
    /// Dragging the selected objects. Positions are continuous tile space.
    Objects {
        doc: u64,
        start: (f64, f64),
        current: (f64, f64),
        indices: Vec<usize>,
    },
}

impl Gesture {
    pub fn doc(&self) -> u64 {
        match self {
            Gesture::Box { doc, .. }
            | Gesture::Lasso { doc, .. }
            | Gesture::Brush { doc, .. }
            | Gesture::Objects { doc, .. } => *doc,
        }
    }
}

/// The "Move selection to layer" dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveDialog {
    pub from: LayerRef,
    pub to: LayerRef,
    pub mode: MoveMode,
}

/// The "Resize map" dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeDialog {
    pub width: u32,
    pub height: u32,
}

/// An operation that needs a newer DS1 version, waiting for the user.
#[derive(Clone, Debug)]
pub struct PendingUpgrade {
    pub doc: u64,
    pub op: EditOp,
    pub current: i32,
    pub required: i32,
}

/// A save that failed because the DS1 version cannot store the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveProblem {
    pub doc: u64,
    pub detail: String,
    pub min_version: i32,
    /// Closing that was waiting for this save, resumed once it works.
    pub then: Option<CloseTarget>,
}

/// The paste preview for one (document, revision, position, mode).
#[derive(Clone, Debug)]
pub struct CachedPreview {
    pub key: (u64, u64, (i32, i32), PasteMode),
    pub preview: PastePreview,
}

/// Thumbnails of recent tiles, rebuilt when the open map's tile set changes.
#[derive(Default)]
pub struct ThumbCache {
    /// Document id and tile-set identity the cache was filled for.
    pub owner: Option<(u64, usize)>,
    pub palette: Option<Palette>,
    pub textures: HashMap<(LayerRef, CellValue), Option<egui::TextureHandle>>,
}

pub struct EditState {
    pub tool: Tool,
    pub scope: SelectScope,
    /// Square brush edge in cells.
    pub brush: u32,
    /// The value Paint writes.
    pub current: CellValue,
    /// The layer `current` was picked from.
    pub current_layer: Option<LayerRef>,
    pub recent: Vec<(LayerRef, CellValue)>,
    pub grid: bool,
    /// The app's own copy of the last copied/cut clip.
    pub clipboard: Option<Clipboard>,
    /// Mirror clips to the OS clipboard (off in scripted runs, which must not
    /// disturb the user's clipboard).
    pub use_os_clipboard: bool,
    pub paste: Option<PasteSession>,
    pub paste_cache: Option<CachedPreview>,
    /// Mode a new paste starts with.
    pub paste_mode: PasteMode,
    pub gesture: Option<Gesture>,
    pub move_dialog: Option<MoveDialog>,
    pub resize_dialog: Option<ResizeDialog>,
    pub upgrade: Option<PendingUpgrade>,
    pub close_prompt: Option<ClosePrompt>,
    pub confirm_packed: bool,
    pub save_problem: Option<SaveProblem>,
    /// Entries in the open profile's CHANGED.txt.
    pub changed: usize,
    pub thumbs: ThumbCache,
}

impl Default for EditState {
    fn default() -> Self {
        EditState {
            tool: Tool::default(),
            scope: SelectScope::default(),
            brush: 1,
            current: CellValue::EMPTY,
            current_layer: None,
            recent: Vec::new(),
            grid: false,
            clipboard: None,
            use_os_clipboard: true,
            paste: None,
            paste_cache: None,
            paste_mode: PasteMode::Overwrite,
            gesture: None,
            move_dialog: None,
            resize_dialog: None,
            upgrade: None,
            close_prompt: None,
            confirm_packed: false,
            save_problem: None,
            changed: 0,
            thumbs: ThumbCache::default(),
        }
    }
}

impl EditState {
    /// Remember a picked value at the front of the recent list.
    pub fn push_recent(&mut self, layer: LayerRef, value: CellValue) {
        self.recent.retain(|&(l, v)| !(l == layer && v == value));
        self.recent.insert(0, (layer, value));
        self.recent.truncate(MAX_RECENT);
    }

    /// Make `value` (picked from `layer`) the brush.
    pub fn pick(&mut self, layer: LayerRef, value: CellValue) {
        self.current = value;
        self.current_layer = Some(layer);
        self.push_recent(layer, value);
    }

    /// True while a modal dialog is open (it takes the keyboard).
    pub fn modal_open(&self) -> bool {
        self.upgrade.is_some()
            || self.close_prompt.is_some()
            || self.confirm_packed
            || self.save_problem.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(cell: u32) -> CellValue {
        CellValue {
            cell,
            orientation: 0,
        }
    }

    #[test]
    fn recent_is_most_recent_first_without_duplicates() {
        let mut s = EditState::default();
        s.pick(LayerRef::Floor(0), v(1));
        s.pick(LayerRef::Floor(0), v(2));
        s.pick(LayerRef::Floor(0), v(1));
        assert_eq!(
            s.recent,
            vec![(LayerRef::Floor(0), v(1)), (LayerRef::Floor(0), v(2))]
        );
        // The same value on another layer is a different entry.
        s.pick(LayerRef::Wall(0), v(1));
        assert_eq!(s.recent.len(), 3);
        assert_eq!(s.current, v(1));
        assert_eq!(s.current_layer, Some(LayerRef::Wall(0)));
    }

    #[test]
    fn recent_keeps_sixteen() {
        let mut s = EditState::default();
        for i in 1..=40 {
            s.pick(LayerRef::Floor(0), v(i));
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0].1, v(40));
        assert_eq!(s.recent[MAX_RECENT - 1].1, v(25));
    }
}
