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
    /// The app offers `Document::upgrade_version(required)`. `required` is the
    /// lowest version that works: after `upgrade_version(required)` the same
    /// call succeeds, and no lower version would do (see "NeedsVersion" below).
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
    /// Layer-aware emptiness: Tag → cell == 0; others → prop1 == 0. Prefer this over CellValue::is_empty.
    pub fn is_empty_value(self, value: CellValue) -> bool;
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
    /// Whole-model swap (restore from snapshot, external reload).
    pub fn replace(label: &str, before: d2_formats::ds1::Ds1, after: d2_formats::ds1::Ds1) -> Patch;
}

// ───────────── document.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveMode { Overwrite, SkipOccupied }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveReport { pub moved: usize, pub skipped_occupied: usize, pub layer_added: Option<LayerRef> }

/// A DS1 plus its undo/redo history. Every mutating method builds one Patch,
/// applies it, and pushes it (clearing redo). A failed method changes nothing.
/// A method whose effect is no change returns Ok and records nothing (no
/// method returns EditError::Empty). Behaviours chosen where this contract is
/// silent are listed in the crate docs (capy-edit/src/lib.rs).
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
    /// Fills ClipSource on copy and drives act/LvlType mismatch warnings on paste.
    pub fn set_clip_context(&mut self, game_path: Option<String>, lvl_type: Option<u32>);

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
    pub fn paste(&mut self, clip: &Clipboard, at: (i32, i32), mode: PasteMode) -> Result<PastePreview>; // Format(Invalid) on an ambiguous path anchor (below)

    // objects (index into ds1.objects)
    pub fn add_object(&mut self, object: d2_formats::ds1::Object) -> Result<usize>;  // Format(Invalid) on an ambiguous path anchor (below)
    pub fn move_objects(&mut self, indices: &[usize], dx: i32, dy: i32) -> Result<()>; // Format(Invalid) on an ambiguous path anchor (below)
    pub fn delete_objects(&mut self, indices: &[usize]) -> Result<()>;           // NPC paths anchored on them move to undo data
    /// Replace one object's type/id/flags/position (position change moves its anchored path).
    /// Format(Invalid) on an ambiguous path anchor (below).
    pub fn set_object(&mut self, index: usize, object: d2_formats::ds1::Object) -> Result<()>;
    // NPC paths (keyed by the anchor object's sub-tile coordinate; creating one needs DS1 v>=14,
    // an action other than 1 needs v>=15, else NeedsVersion). Editing a path that does not exist
    // (every map below v14) or a missing point is Format(Invalid).
    pub fn path_of(&self, object_index: usize) -> Option<&d2_formats::ds1::NpcPath>;
    pub fn add_path_point(&mut self, object_index: usize, point: d2_formats::ds1::PathPoint, at: Option<usize>) -> Result<()>; // creates the path if missing
    pub fn move_path_point(&mut self, object_index: usize, point_index: usize, x: i32, y: i32) -> Result<()>;
    pub fn set_path_action(&mut self, object_index: usize, point_index: usize, action: i32) -> Result<()>;
    pub fn delete_path_points(&mut self, object_index: usize, point_indices: &[usize]) -> Result<()>; // removes the path when empty
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
    pub needs_version: Option<i32>,        // the `required` that `paste` would return
    pub warnings: Vec<String>,            // e.g. different act / LvlType: tiles may not exist in this map's DT1 set;
                                          // one entry per ambiguous NPC path anchor (paste refuses those)
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

## NeedsVersion

`EditError::NeedsVersion { required }` is the lowest version the document can
be upgraded to so that the same call then succeeds and the result can be
written. It is never lower than the call's own gate (a 2nd wall needs 4, a 2nd
floor 16, the tag layer 10, objects 2, object flags 6, NPC paths 14, a path
action other than 1 15) and is higher when `upgrade_version` cannot reach that
gate or changes the map on the way:

- A v3 map whose tag layer holds data cannot be v4..=9 (those store no tag
  layer), so a 2nd wall or object flags on it ask for 10, not 4 or 6.
- An all-zero v3 tag layer is dropped by an upgrade to v4..=9, so a paste that
  writes tag cells and needs v6 for object flags asks for 10.
- `upgrade_version` itself answers `NeedsVersion { required: 10 }` when asked
  for v4..=9 on such a map.

Every method checks the request against the document first and the version
last: a request that would fail anyway (a bad index, an ambiguous path anchor,
a path or point that does not exist) gets that error, not `NeedsVersion`.
`PastePreview::needs_version` is the `required` that `paste` returns.

## Ambiguous NPC path anchors

A path belongs to the object on its anchor sub-tile and the file stores no
other link, so two objects on one sub-tile with a path there (or two paths on
one sub-tile) leave the path's owner undecidable.

- `set_object` and `move_objects` return `Format(Invalid)` and change nothing
  when a moved object would land on a sub-tile holding an object or a path
  that is not moving with it, and the result would hold two objects and a
  path, or two paths. The detail names the sub-tile and the objects, e.g.
  "object 0 would share sub-tile (2, 2) with object 1 that has an NPC path" or
  "object 1 has an NPC path and would share sub-tile (1, 1) with object 0".
- Allowed: moves where no path is involved (objects may share a sub-tile),
  moving an object without a path onto a stray path that has no object (it
  takes the path over), a sharer leaving an ambiguous sub-tile, and moving
  every object of an already ambiguous sub-tile together (it stays as it was).
- `add_path_point` refuses an object that shares its sub-tile with another
  object. `move_path_point`, `set_path_action` and `delete_path_points` work on
  an already ambiguous path (the first path anchored there) so it can be
  cleaned up, and `delete_objects` removes paths anchored only on the deleted
  objects.
- `add_object` and `paste` refuse the same way (`Format(Invalid)` with the
  same wording, nothing changes) when a new object would land on a sub-tile
  holding an object with a path, or two paths, or when two pasted objects
  would land together on a path. Joining objects without a path, and taking
  over a stray path, stay allowed. `PastePreview::warnings` lists each such
  sub-tile (one message per sub-tile, the same text `paste` fails with). As
  with moves, the anchor check comes before `NeedsVersion`.

## Map-editing operations (P9)

Module `capy_edit::tools` (re-exported at the crate root) holds the pure data
types; the operations are `Document` methods and each one is a single undoable
patch (`Patch` steps are the existing `Cells` / `Header` / `Replace`). Game
data (tile candidate lists, the object catalog) is always passed in by the
caller. No new dependencies.

```rust
pub struct Rng;                       // SplitMix64: Rng::new(seed), next_u64(), below(n)
pub struct Brush { pub main: u8, pub sub: u8, pub orientation: u32 }   // Brush::floor(main, sub)
pub struct TileKey { pub orientation: u32, pub main: u8, pub sub: u8 } // floors 0, shadows 13, walls: raw orientation dword
pub fn tile_key(layer: LayerRef, value: CellValue) -> Option<TileKey>; // None: empty cell / tag layer

impl Document {
    // 1. rectangle and flood. `mix` empty = erase; one entry = plain paint; several = 2. random mix.
    pub fn paint_rect(&mut self, layer, rect: CellRect, mix: &[Brush], rng: &mut Rng) -> Result<()>;
    pub fn paint_mix(&mut self, layer, cells: &[(u32, u32)], mix: &[Brush], rng: &mut Rng) -> Result<()>;
    pub fn erase_rect(&mut self, layer, rect: CellRect) -> Result<()>;
    pub fn flood_region(&self, layer, x, y, within: Option<&Selection>) -> Vec<(u32, u32)>;
    pub fn flood_fill(&mut self, layer, x, y, mix: &[Brush], within: Option<&Selection>, rng: &mut Rng) -> Result<()>;
    // 3. find & replace (layers + optional selection scope)
    pub fn find_tiles(&self, layers: &[LayerRef], from: TileKey, within: Option<&Selection>) -> Vec<(LayerRef, u32, u32)>;
    pub fn replace_tiles(&mut self, layers: &[LayerRef], from: TileKey, to: TileKey, within: Option<&Selection>) -> Result<usize>;
    // 4. floor re-roll and tile reassignment
    pub fn reroll_floors(&mut self, layer: u8, area: Option<&Selection>, choices: &[FloorChoice],
                         opts: &RerollOptions, walk_of: &dyn Fn(u8, u8) -> Option<u32>) -> Result<RerollReport>;
    pub fn reassign_tiles(&mut self, moves: &BTreeMap<TileKey, TileKey>) -> Result<usize>;
    // resize on any edge
    pub fn resize_edges(&mut self, left: i32, top: i32, right: i32, bottom: i32) -> Result<()>;
    // 6. tag groups
    pub fn add_group(&mut self, g: Group) -> Result<usize>;
    pub fn set_group(&mut self, index: usize, g: Group) -> Result<()>;
    pub fn move_group(&mut self, index: usize, x: i32, y: i32) -> Result<()>;
    pub fn resize_group(&mut self, index: usize, width: i32, height: i32) -> Result<()>;
    pub fn delete_group(&mut self, index: usize) -> Result<()>;
}
// 5. new map
pub struct NewMapSpec { pub width, pub height, pub act: i32, pub walls: usize, pub floors: usize,
                        pub tag: bool, pub version: Option<i32> }   // NewMapSpec::new(w, h, act): 1 wall, 1 floor, v18
pub fn new_map(spec: &NewMapSpec) -> Result<Ds1>;
// 7. object checks
pub fn check_objects(ds1: &Ds1, opts: &ObjectCheckOptions) -> Vec<ObjectFinding>;
```

Behaviour:

- **Painting.** Floor, wall and shadow layers only (`Format(Invalid)` for the
  tag layer, `NoSuchLayer` for a missing layer). A fresh cell gets `prop1`
  0xC2 (floor), 0x81 (wall) or 0x80 (shadow); painting over a tile keeps its
  other bits (hidden, unwalkable). A wall brush sets the orientation dword,
  other layers drop it. Rectangles are clipped to the map; `paint_mix` ignores
  cells outside it. The random pick is one `rng.below(mix.len())` per cell in
  row-major order, so a seed reproduces the result. Unchanged cells are not
  recorded (a no-op records no history entry).
- **Connected** (as ds1-studio): 4-connected (edge neighbours), same
  `TileKey` (orientation, main, sub; other cell bits ignored); an empty cell
  floods all connected empty cells. `within` limits the area per layer; a seed
  cell outside the map or `within` gives an empty region. `flood_fill` with an
  empty mix is flood erase.
- **Find & replace** compares `TileKey`s on the chosen layers (the tag layer
  and missing layers are skipped); the replacement keeps other cell bits.
  Returns the number of cells that changed.
- **Floor re-roll** is ds1-studio's deterministic balanced pick over the
  caller's `FloorChoice { main, sub, rarity, walk }` list: it avoids repeating
  the left/upper neighbour (`avoid_repeats`), balances use counts scaled by
  `rarity`, breaks ties with a seeded hash, and never picks rarity-0 choices
  while a choice with rarity exists. With `preserve_walkability` only choices
  whose `walk` equals `walk_of(main, sub)` of the replaced tile qualify (no
  answer or no match: the cell is counted in `RerollReport::skipped`).
  Caller builds the choice list from the DT1 group of the same main/sub family.
- **Reassign** applies an old-number to new-number map (the caller derives it
  from its DT1 renumbering) to floors, walls (orientation follows the new key)
  and the shadow layer. Composing renumberings is the caller's job.
- **`new_map`** defaults to version 18. A size outside 1x1..=1,048,576 cells
  or counts outside 1..=4 walls / 1..=2 floors is `LayerLimit`; a version
  that cannot hold the layers is `NeedsVersion` (below v4 a single wall and
  floor with a tag layer; the tag layer needs v10 for `tag_type` 1).
- **Groups** are rectangles in map cells. They need v12+ and `tag_type` 1 or 2
  (`NeedsVersion { required: 12 }` / `Format(Invalid)`); a non-zero `unknown`
  needs v13. `width`/`height` must be >= 1 and `x`/`y` >= 0 for any group the
  call adds or changes (existing odd groups of real files are left alone).
  A bad index is `Format(Invalid)`. Edits go through the existing header step,
  so the model needed no change in `d2-formats`; every corpus DS1 still
  round-trips byte-identically, including after group edits and undo.
- **Resize by dragging an edge.** `resize` covers the right and bottom edges
  only (it keeps the top-left). Dragging the left or top edge needs
  `resize_edges`, which shifts cells, objects (x5 sub-tiles), NPC paths and
  groups, drops what falls off the map and clips groups (as ds1-studio's
  `resizeDs1`). It is a whole-map `Patch::replace`, so it holds two copies of
  the map in the undo stack.
- **Object checks** return `ObjectFinding { index, problem }` in object order:
  `OutsideMap` (sub-tile position outside `width*5` x `height*5`),
  `Duplicate { of }` (same kind, id and position as an earlier object),
  `UnknownId` (only with `ObjectCheckOptions::is_known`) and `UnpairedPad` (a
  teleportation pad, from `teleport_pad_row`, with no pad of the same row in
  its own or a touching 40-sub-tile block, ds1-studio's `unpairedPads`).
  Not ported: the objects.txt checks that need the table itself
  (`directIdsOnGameRows`, `looseOnlyRows`).

## Presets (P11, `capy_edit::presets`)

A preset is a block of cells across floor, wall and shadow layers (never tags)
plus the objects on it, in tile space relative to its top-left cell. It is
serde JSON (`format: "ds1studio-preset"`, saved as
`data/ds1studio/presets/<name>-<id>.json`); `id` is a hash of the content.

```rust
impl Preset {
    pub fn from_selection(&Document, &Selection, name, category) -> Option<Preset>; // objects on selected cells come along
    pub fn from_clipboard(&Clipboard, name, category) -> Preset;
    pub fn to_clipboard(&self, target: &Ds1) -> Clipboard;   // stamped with the target's act/version
    pub fn to_json(&self) -> String;  pub fn from_json(&str) -> Option<Preset>;
    pub fn path(&self) -> String;     pub fn content_id(&self) -> String;
}
pub fn preview_place(&Document, &Preset, at: (i32, i32), PasteMode) -> PastePreview;
/// One undo step "Place preset <name>" (Document::paste inside a group).
pub fn place_preset(&mut Document, &Preset, at, PasteMode) -> Result<PastePreview>;
pub fn sort_presets(&mut [Preset]);  pub fn categories(&[Preset]) -> Vec<String>;
// 20x20 builder: a scratch map (2 floors, 4 walls) painted with the normal tools
pub struct PresetBuilder;  // new(act), document(), document_mut(), is_empty(), build(name, category, trim) -> Option<Preset>
// suggestions: recurring wall structures (<= 16x16, >= 2 parts) merged by identity
pub fn suggestion_groups(&Ds1) -> Vec<Vec<u32>>;
pub fn suggest_presets(maps: &[(String, &Ds1)], &SuggestOptions) -> Vec<Preset>; // limit, drawable(orientation, main, sub), category
```

`capy_edit::no_spawn` (ds1-studio `noSpawnPlan`): `no_spawn_plan(&Ds1, cells: &[(u32,u32)], NoSpawnOptions { whole_rooms, void_tile }) -> NoSpawnPlan { floors, edits: Vec<(floor, x, y, Cell)>, rooms, copied, voids, skipped }` gives every region seed of the 8x8 rooms touching the cells a hidden floor-1 tile (a floored seed is copied to floor 2 first; a seed with no floor gets a hidden unwalkable tile); `Document::apply_no_spawn(&plan)` applies it as one undo step, adding floor 2 when needed. `Document::history() -> (done labels, undone labels)`.

Placing follows `Document::paste` (conflicts, version gates, dropped
out-of-bounds cells), so save -> place -> undo returns the original map
exactly. Suggestions match shadows to walls by main/sub index only (ds1-studio
also compares the DT1 each comes from; the caller can filter with `drawable`).
Identity ignores the `prop1` byte and objects. Not ported: preset packages and
copying a preset's tiles between libraries.
