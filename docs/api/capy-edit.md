# `capy-edit` public API (owner: lead)

The editing model over a `d2_formats::ds1::Ds1`: invertible patches with
unlimited undo/redo, selection, a cross-map clipboard with conflict preview,
layer moves, and a DS1 diff. Depends on `d2-formats` only. No Bevy, no I/O.

Coordinates are tile cells `(x, y)` with `0 <= x < width`, `0 <= y < height`.
Lasso polygons are in tile space (floating point); the app converts screen
coordinates.

```rust
// ───────────── lib.rs ─────────────
pub mod error;     pub use error::{EditError, Result};
pub mod layer;     pub use layer::{LayerRef, CellValue};
pub mod patch;     pub use patch::Patch;
pub mod document;  pub use document::{Document, MoveMode, MoveReport};
pub mod selection; pub use selection::{Selection, SelectOp, CellRect};
pub mod clipboard; pub use clipboard::{Clipboard, ClipSource, PasteMode, PastePreview, PasteConflict};
pub mod diff;      pub use diff::{Ds1Diff, CellChange, diff};

// ───────────── error.rs ─────────────
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    #[error("layer {0:?} does not exist")]
    NoSuchLayer(LayerRef),
    #[error("cell ({x}, {y}) is outside the {width}x{height} map")]
    OutOfBounds { x: i64, y: i64, width: u32, height: u32 },
    /// The change needs a newer DS1 version (e.g. a 2nd wall layer in a v3 file).
    /// The app offers `Document::upgrade_version(required)`.
    #[error("needs DS1 version {required} (file is version {current})")]
    NeedsVersion { current: i32, required: i32 },
    #[error("layer limit reached: {0}")]
    LayerLimit(String),
    #[error("nothing to do")]
    Empty,
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
}
pub type Result<T, E = EditError> = std::result::Result<T, E>;

// ───────────── layer.rs ─────────────
/// A cell layer. Wall indices are 0..4, floor 0..2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum LayerRef { Floor(u8), Wall(u8), Shadow, Tag }
/// One cell's full content. For walls `orientation` is the raw orientation
/// dword that travels with the tile; for every other layer it is 0.
/// Tag layer cells store their u32 in `cell`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CellValue { pub cell: u32, pub orientation: u32 }
impl CellValue {
    pub const EMPTY: CellValue = CellValue { cell: 0, orientation: 0 };
    pub fn is_empty(self) -> bool; // cell's prop1 == 0 (Cell::is_empty), for Tag: cell == 0
}
impl LayerRef {
    pub fn exists_in(self, ds1: &d2_formats::ds1::Ds1) -> bool;
    pub fn all_in(ds1: &d2_formats::ds1::Ds1) -> Vec<LayerRef>; // floors, walls, shadow, tag (if any) — draw order not implied
    pub fn get(self, ds1: &d2_formats::ds1::Ds1, x: u32, y: u32) -> Option<CellValue>;
}

// ───────────── patch.rs ─────────────
/// An invertible change. `apply` then `revert` restores the DS1 exactly
/// (`PartialEq`), including layer counts, `version`, and object order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch { /* label: String, steps: Vec<Step> (private) */ }
impl Patch {
    pub fn label(&self) -> &str;
    pub fn is_empty(&self) -> bool;
    pub fn apply(&self, ds1: &mut d2_formats::ds1::Ds1) -> Result<()>;
    pub fn revert(&self, ds1: &mut d2_formats::ds1::Ds1) -> Result<()>;
}

// ───────────── document.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveMode { Overwrite, SkipOccupied }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveReport { pub moved: usize, pub skipped_occupied: usize, pub layer_added: Option<LayerRef> }

/// A DS1 plus its undo/redo history. Every mutating method builds one Patch,
/// applies it, and pushes it (clearing redo). A failed method changes nothing.
pub struct Document { /* ds1, undo: Vec<Patch>, redo: Vec<Patch>, revision, saved_revision, group */ }
impl Document {
    pub fn new(ds1: d2_formats::ds1::Ds1) -> Document;
    pub fn ds1(&self) -> &d2_formats::ds1::Ds1;
    pub fn into_ds1(self) -> d2_formats::ds1::Ds1;

    // history
    pub fn undo(&mut self) -> bool;
    pub fn redo(&mut self) -> bool;
    pub fn can_undo(&self) -> bool;
    pub fn can_redo(&self) -> bool;
    pub fn undo_label(&self) -> Option<&str>;
    pub fn redo_label(&self) -> Option<&str>;
    /// Monotonic; bumps on every apply/undo/redo. The renderer re-uploads on change.
    pub fn revision(&self) -> u64;
    /// Dirty = the current state differs from the last `mark_saved` point
    /// (undoing back to the saved state makes it clean again).
    pub fn is_dirty(&self) -> bool;
    pub fn mark_saved(&mut self);
    /// Merges every patch pushed until `end_group` into one undo step (a brush stroke).
    pub fn begin_group(&mut self, label: &str);
    pub fn end_group(&mut self);
    /// Apply a prebuilt patch (used by tests and by restore-from-snapshot).
    pub fn apply_patch(&mut self, patch: Patch) -> Result<()>;

    // edits
    pub fn paint(&mut self, layer: LayerRef, cells: &[(u32, u32, CellValue)]) -> Result<()>;
    pub fn erase(&mut self, selection: &Selection) -> Result<()>;               // sets selected cells to EMPTY
    pub fn set_cell_flags(&mut self, selection: &Selection, hidden: Option<bool>, unwalkable: Option<bool>) -> Result<()>;
    /// Move (or batch-move) the selected cells of `from` to the same (x, y) in `to`
    /// (e.g. Wall(0) -> Wall(2)), adding `to` if missing. Only the `from` cells in
    /// the selection move; their sources become EMPTY. Wall<->non-wall moves drop
    /// / zero the orientation. NeedsVersion / LayerLimit if `to` cannot be added.
    pub fn move_to_layer(&mut self, selection: &Selection, from: LayerRef, to: LayerRef, mode: MoveMode) -> Result<MoveReport>;
    pub fn add_layer(&mut self, layer: LayerRef) -> Result<()>;
    pub fn remove_layer(&mut self, layer: LayerRef) -> Result<()>;               // content restored on undo
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()>;             // keeps top-left; objects outside are kept as-is
    pub fn upgrade_version(&mut self, version: i32) -> Result<()>;               // >= current; undoable

    // clipboard
    pub fn copy(&self, selection: &Selection) -> Clipboard;
    pub fn cut(&mut self, selection: &Selection) -> Result<Clipboard>;
    pub fn preview_paste(&self, clip: &Clipboard, at: (i32, i32), mode: PasteMode) -> PastePreview;
    pub fn paste(&mut self, clip: &Clipboard, at: (i32, i32), mode: PasteMode) -> Result<PastePreview>;

    // objects (index into ds1.objects)
    pub fn add_object(&mut self, object: d2_formats::ds1::Object) -> Result<usize>;
    pub fn move_objects(&mut self, indices: &[usize], dx: i32, dy: i32) -> Result<()>;
    pub fn delete_objects(&mut self, indices: &[usize]) -> Result<()>;           // NPC paths anchored on them move to undo data
}

// ───────────── selection.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectOp { Replace, Add, Subtract, Intersect }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellRect { pub x0: u32, pub y0: u32, pub x1: u32, pub y1: u32 } // inclusive, normalized by constructors
impl CellRect { pub fn new(a: (u32, u32), b: (u32, u32)) -> CellRect; }

/// A set of (layer, x, y). Only cells that exist and are non-empty are ever selected,
/// except `box_select(.., include_empty = true)` (paste targets / erase areas).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection { /* BTreeSet<(LayerRef, u32, u32)> */ }
impl Selection {
    pub fn new() -> Selection;
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn contains(&self, layer: LayerRef, x: u32, y: u32) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = (LayerRef, u32, u32)> + '_;
    pub fn layers(&self) -> Vec<LayerRef>;
    pub fn bounds(&self) -> Option<CellRect>;
    pub fn clear(&mut self);
    pub fn insert(&mut self, layer: LayerRef, x: u32, y: u32) -> bool;
    pub fn box_select(&mut self, ds1: &d2_formats::ds1::Ds1, rect: CellRect, layers: &[LayerRef], include_empty: bool, op: SelectOp);
    /// Cells whose centre (x+0.5, y+0.5) lies inside the polygon (even-odd rule).
    pub fn lasso(&mut self, ds1: &d2_formats::ds1::Ds1, polygon: &[(f32, f32)], layers: &[LayerRef], op: SelectOp);
    /// Every cell whose (main, sub) — and orientation, when given — match.
    pub fn by_tile_id(&mut self, ds1: &d2_formats::ds1::Ds1, main: u8, sub: u8, orientation: Option<u32>, layers: &[LayerRef], op: SelectOp);
    /// Every cell in `layers` whose CellValue (indices, orientation and flag bits) equals the seed cell's.
    pub fn identical(&mut self, ds1: &d2_formats::ds1::Ds1, seed: (LayerRef, u32, u32), layers: &[LayerRef], op: SelectOp);
}

// ───────────── clipboard.rs ─────────────
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipSource { pub game_path: Option<String>, pub act: i32, pub lvl_type: Option<u32>, pub version: i32 }
/// Cells relative to the selection's top-left, plus objects inside the bounds.
/// Serializable so it can cross documents (and the OS clipboard as JSON).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Clipboard {
    pub width: u32,
    pub height: u32,
    pub cells: Vec<(LayerRef, u32, u32, CellValue)>,
    pub objects: Vec<d2_formats::ds1::Object>,   // x/y relative to the clip origin, in sub-tiles
    pub source: ClipSource,
}
impl Clipboard { pub fn to_json(&self) -> String; pub fn from_json(text: &str) -> Option<Clipboard>; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode { Overwrite, SkipOccupied, OnlyIntoEmptyLayers }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteConflict { pub layer: LayerRef, pub x: u32, pub y: u32, pub existing: CellValue, pub incoming: CellValue }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PastePreview {
    pub writes: usize,                    // cells that will change
    pub conflicts: Vec<PasteConflict>,    // non-empty targets with different content
    pub out_of_bounds: usize,             // clip cells that fall outside the map (dropped)
    pub layers_to_add: Vec<LayerRef>,
    pub needs_version: Option<i32>,
    pub warnings: Vec<String>,            // e.g. different act / LvlType: tiles may not exist in this map's DT1 set
}

// ───────────── diff.rs ─────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellChange { pub layer: LayerRef, pub x: u32, pub y: u32, pub before: CellValue, pub after: CellValue }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Diff {
    pub size: Option<((u32, u32), (u32, u32))>,
    pub version: Option<(i32, i32)>,
    pub layers_added: Vec<LayerRef>,
    pub layers_removed: Vec<LayerRef>,
    pub cells: Vec<CellChange>,            // over the common area of layers present in both
    pub objects_added: Vec<d2_formats::ds1::Object>,
    pub objects_removed: Vec<d2_formats::ds1::Object>,
    pub other_changed: bool,               // files, groups, paths, act, tag_type, preserved
}
impl Ds1Diff { pub fn is_empty(&self) -> bool; }
pub fn diff(before: &d2_formats::ds1::Ds1, after: &d2_formats::ds1::Ds1) -> Ds1Diff;
```
