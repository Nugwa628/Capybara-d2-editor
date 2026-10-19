//! Editing model over a [`d2_formats::ds1::Ds1`]: invertible patches with
//! unlimited undo/redo, selections, a cross-map clipboard with conflict
//! preview, layer moves and a DS1 diff. No I/O, no UI.
//!
//! The public API is specified in `docs/api/capy-edit.md`.
//!
//! # Behaviours chosen where the contract was silent
//!
//! - **No-ops succeed.** An edit that would change nothing (painting identical
//!   tiles, erasing an empty selection, `resize` to the current size,
//!   `move_objects` by `(0, 0)`, a paste with `writes == 0`, ...) returns `Ok`,
//!   records no history entry and leaves `revision()`, the undo depth and
//!   `is_dirty()` alone. `EditError::Empty` exists for compatibility only.
//! - **Stale selections.** Selection-based operations ignore selected cells
//!   outside the map; explicit coordinate lists (`paint`) fail with `OutOfBounds`.
//!   A `layers` argument that is empty selects nothing, and missing layers are skipped.
//! - **Cell values.** Painting drops the orientation on layers that are not
//!   walls. `LayerRef::is_empty_value` is the layer-aware emptiness test (the
//!   tag layer is empty at `cell == 0`; `CellValue::is_empty` uses `prop1`).
//!   `set_cell_flags` skips empty cells and the tag layer.
//! - **Selections.** `by_tile_id(.., Some(o), ..)` compares `o` with the raw
//!   orientation dword (0 outside walls) and never matches the tag layer.
//!   `identical` compares whole `CellValue`s and ignores an empty seed.
//! - **Layers.** Adding a layer also adds the lower-numbered ones of its kind;
//!   minimum versions are 4 (wall), 16 (floor), 10 (tag) and the result is
//!   `NeedsVersion`. Adding the tag layer sets `tag_type` to 1; removing it sets
//!   `tag_type` to 0 and drops the groups (restored by undo). The shadow layer,
//!   the only wall/floor of a file below v4/v16 and the tag layer below v4
//!   cannot be removed (`LayerLimit`). The tag layer cannot take part in
//!   `move_to_layer`. Empty source cells are not moved; when every target was
//!   occupied in `SkipOccupied` mode the report has `moved == 0`.
//! - **`resize`** accepts 1x1 up to 1,048,576 cells (`LayerLimit` otherwise).
//! - **`upgrade_version`** only raises the version (`Format(Invalid)` otherwise).
//!   An all-zero tag layer the new version cannot store is dropped; a non-empty
//!   one needs v10+ (`NeedsVersion`), where `tag_type` becomes 1. Crossing
//!   v7 remaps wall orientations through `DIR_LOOKUP`; the v9..13 unknown
//!   words are zeroed above v13. The result is checked with `to_bytes()` and
//!   rolled back if it cannot be written.
//! - **Objects.** Adding objects needs v2 (v6 for non-zero flags). A bad index
//!   is `Format(Invalid)`. Moving or deleting objects moves or removes the NPC
//!   paths anchored only on them (undo restores both).
//! - **NPC paths.** A path belongs to the object on its anchor sub-tile
//!   (`path_of` returns the first path anchored there). Creating a path below
//!   v14 is `NeedsVersion { required: 14 }`; an action other than 1 below v15
//!   is `required: 15`. `add_path_point` creates the path when missing
//!   (appended to the path list; `at` must then be `None` or `Some(0)`) and
//!   fails with `Format(Invalid)` when another object stands on the same
//!   sub-tile, since the game drops such a path. `move_path_point`,
//!   `set_path_action` and `delete_path_points` work on an existing path
//!   (`Format(Invalid)` for a missing path or point, so on every map below
//!   v14; an empty index list is a no-op) and `delete_path_points` removes the path with its last point.
//!   `set_object` moves the anchor of the path that sits only on that object;
//!   a path on a shared sub-tile stays, as with `move_objects`. A failed
//!   path edit changes nothing, an identical value is a no-op.
//! - **Clipboard.** `copy` takes the selected existing cells plus the objects
//!   whose sub-tile lies inside the selection's bounding rectangle; `cut` also
//!   deletes those objects. Conflicts are reported in every paste mode;
//!   `OnlyIntoEmptyLayers` writes only into layers that are entirely empty or
//!   new. `layers_to_add` includes intermediate layers. Clip objects outside
//!   the map and cells on impossible layers are dropped with a warning.
//!   Act/LvlType warnings need `Document::set_clip_context`.
//! - **`NeedsVersion.required`** is the lowest version the document can be
//!   upgraded to so that the same call then succeeds and the result can be
//!   written: never below the call's own gate, and higher when
//!   `upgrade_version` cannot reach the gate or changes the map on the way (a
//!   v3 map with tag cells asks for 10 for a 2nd wall or object flags; an
//!   all-zero v3 tag layer is dropped below v10, so a paste that writes tag
//!   cells asks for 10). Requests are checked against the document before the
//!   version, so one that would fail anyway (bad index, ambiguous anchor,
//!   missing path or point) gets that error instead.
//!   `PastePreview::needs_version` is the same number.
//! - **Ambiguous path anchors.** `set_object` and `move_objects` return
//!   `Format(Invalid)` and change nothing when a moved object would land on a
//!   sub-tile holding an object or path that is not moving with it, leaving
//!   two objects and a path, or two paths, there. Moves with no path involved,
//!   a sharer leaving an ambiguous sub-tile and moving a whole ambiguous
//!   sub-tile together stay allowed, as do path edits and deletes on an
//!   already ambiguous path (so it can be cleaned up). `add_object` and
//!   `paste` refuse the same way (`Format(Invalid)`, same wording, nothing
//!   changes) an object placed on a sub-tile holding an object with a path or
//!   two paths, or two pasted objects landing on one path; an object may
//!   still join objects without a path or take over a stray path. The paste
//!   preview lists each such sub-tile in `warnings`, and the check comes
//!   before the version check.
//! - **Restore.** `Patch::replace(label, before, after)` builds a whole-model
//!   patch for `apply_patch`.

pub mod clipboard;
pub mod diff;
pub mod document;
pub mod error;
pub mod layer;
pub mod patch;
pub mod selection;

mod layer_ops;
mod version;

pub use clipboard::{ClipSource, Clipboard, PasteConflict, PasteMode, PastePreview};
pub use diff::{CellChange, Ds1Diff, diff};
pub use document::{Document, MoveMode, MoveReport};
pub use error::{EditError, Result};
pub use layer::{CellValue, LayerRef};
pub use patch::Patch;
pub use selection::{CellRect, SelectOp, Selection};
