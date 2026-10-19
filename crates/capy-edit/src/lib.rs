//! Editing model over a [`d2_formats::ds1::Ds1`]: invertible patches with
//! unlimited undo/redo, selections, a cross-map clipboard with conflict
//! preview, layer moves and a DS1 diff. No I/O, no UI.
//!
//! The public API is specified in `docs/api/capy-edit.md`.

pub mod clipboard;
pub mod diff;
pub mod document;
pub mod error;
pub mod layer;
pub mod patch;
pub mod selection;

mod layer_ops;

pub use clipboard::{ClipSource, Clipboard, PasteConflict, PasteMode, PastePreview};
pub use diff::{CellChange, Ds1Diff, diff};
pub use document::{Document, MoveMode, MoveReport};
pub use error::{EditError, Result};
pub use layer::{CellValue, LayerRef};
pub use patch::Patch;
pub use selection::{CellRect, SelectOp, Selection};
