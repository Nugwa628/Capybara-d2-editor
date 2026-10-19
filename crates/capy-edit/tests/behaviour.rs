#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use capy_edit::{
    CellRect, CellValue, ClipSource, Clipboard, Document, EditError, LayerRef, MoveMode, PasteMode,
    Patch, SelectOp, Selection, diff,
};
use common::*;
use d2_formats::Grid;
use d2_formats::ds1::{Cell, Ds1, Group, NpcPath, Object, PathPoint};

/// A call that would change nothing must succeed and leave no trace: same
/// revision, same undo/redo state, same dirty flag, same document.
macro_rules! noop {
    ($doc:ident, $call:expr) => {{
        let snapshot = |d: &Document| {
            (
                d.revision(),
                d.can_undo(),
                d.can_redo(),
                d.undo_label().map(str::to_string),
                d.redo_label().map(str::to_string),
                d.is_dirty(),
                d.ds1().clone(),
            )
        };
        let before = snapshot(&$doc);
        let result = $call;
        assert!(
            result.is_ok(),
            "expected a no-op success: {:?}",
            result.err()
        );
        assert!(before == snapshot(&$doc), "a no-op must not leave a trace");
    }};
}

fn select_all(doc: &Document, layers: &[LayerRef]) -> Selection {
    let mut sel = Selection::new();
    let ds = doc.ds1();
    sel.box_select(
        ds,
        CellRect::new((0, 0), (ds.width - 1, ds.height - 1)),
        layers,
        false,
        SelectOp::Replace,
    );
    sel
}

// ----- paint / erase / flags / history --------------------------------------

#[test]
fn paint_undo_redo_and_labels() {
    let mut doc = Document::new(Ds1::new(4, 4, 0));
    let before = doc.ds1().clone();
    assert!(!doc.is_dirty());
    doc.paint(LayerRef::Floor(0), &[(1, 1, cv(2, 3)), (2, 2, cv(1, 1))])
        .unwrap();
    assert_eq!(doc.undo_label(), Some("Paint"));
    assert!(doc.is_dirty());
    assert_eq!(LayerRef::Floor(0).get(doc.ds1(), 1, 1), Some(cv(2, 3)));
    assert!(doc.undo());
    assert_eq!(doc.ds1(), &before);
    assert!(!doc.is_dirty());
    assert_eq!(doc.redo_label(), Some("Paint"));
    assert!(doc.redo());
    assert_eq!(LayerRef::Floor(0).get(doc.ds1(), 2, 2), Some(cv(1, 1)));
    assert_eq!(doc.revision(), 3);
}

#[test]
fn paint_errors_change_nothing() {
    let mut doc = Document::new(Ds1::new(4, 4, 0));
    let before = doc.ds1().clone();
    let err = doc
        .paint(LayerRef::Floor(0), &[(1, 1, cv(1, 1)), (4, 0, cv(1, 1))])
        .unwrap_err();
    assert!(matches!(err, EditError::OutOfBounds { x: 4, y: 0, .. }));
    assert!(matches!(
        doc.paint(LayerRef::Wall(1), &[(0, 0, cv(1, 1))]),
        Err(EditError::NoSuchLayer(LayerRef::Wall(1)))
    ));
    assert!(matches!(
        doc.paint(LayerRef::Tag, &[(0, 0, cv(1, 1))]),
        Err(EditError::NoSuchLayer(LayerRef::Tag))
    ));
    // painting what is already there records nothing
    noop!(
        doc,
        doc.paint(LayerRef::Floor(0), &[(0, 0, CellValue::EMPTY)])
    );
    assert_eq!(doc.ds1(), &before);
    assert!(!doc.can_undo());
    assert_eq!(doc.revision(), 0);
}

#[test]
fn paint_drops_orientation_outside_walls_and_last_write_wins() {
    let mut doc = Document::new(Ds1::new(3, 3, 0));
    let mut v = cv(1, 1);
    v.orientation = 9;
    doc.paint(
        LayerRef::Floor(0),
        &[(0, 0, v), (1, 0, cv(1, 2)), (1, 0, cv(1, 3))],
    )
    .unwrap();
    assert_eq!(LayerRef::Floor(0).get(doc.ds1(), 0, 0), Some(cv(1, 1)));
    assert_eq!(LayerRef::Floor(0).get(doc.ds1(), 1, 0), Some(cv(1, 3)));
    doc.undo();
    assert_eq!(
        LayerRef::Floor(0).get(doc.ds1(), 1, 0),
        Some(CellValue::EMPTY)
    );
}

#[test]
fn erase_and_flags() {
    let mut doc = Document::new(Ds1::new(4, 4, 0));
    doc.paint(
        LayerRef::Wall(0),
        &[(1, 1, wall(1, 1, 7)), (2, 1, wall(1, 2, 8))],
    )
    .unwrap();
    let sel = select_all(&doc, &[LayerRef::Wall(0)]);
    assert_eq!(sel.len(), 2);

    doc.set_cell_flags(&sel, Some(true), Some(true)).unwrap();
    let c = Cell(LayerRef::Wall(0).get(doc.ds1(), 1, 1).unwrap().cell);
    assert!(c.hidden() && c.unwalkable());
    noop!(doc, doc.set_cell_flags(&sel, Some(true), None));
    doc.set_cell_flags(&sel, Some(false), None).unwrap();
    let c = Cell(LayerRef::Wall(0).get(doc.ds1(), 1, 1).unwrap().cell);
    assert!(!c.hidden() && c.unwalkable());

    doc.erase(&sel).unwrap();
    assert_eq!(
        LayerRef::Wall(0).get(doc.ds1(), 1, 1),
        Some(CellValue::EMPTY)
    );
    noop!(doc, doc.erase(&sel));
    doc.undo();
    assert_eq!(doc.ds1().walls[0].orientations.get(1, 1), Some(7));
    // an empty selection is nothing to do
    noop!(doc, doc.erase(&Selection::new()));
}

#[test]
fn groups_merge_into_one_undo_step() {
    let mut doc = Document::new(Ds1::new(8, 2, 0));
    let before = doc.ds1().clone();
    doc.begin_group("Brush");
    for x in 0..8 {
        doc.paint(LayerRef::Floor(0), &[(x, 0, cv(1, x as u8 + 1))])
            .unwrap();
    }
    doc.paint(LayerRef::Shadow, &[(0, 1, cv(3, 3))]).unwrap();
    doc.end_group();
    assert_eq!(doc.undo_label(), Some("Brush"));
    let after = doc.ds1().clone();
    assert!(doc.undo());
    assert_eq!(doc.ds1(), &before);
    assert!(!doc.can_undo());
    assert!(doc.redo());
    assert_eq!(doc.ds1(), &after);
    // an empty group leaves no step
    doc.begin_group("nothing");
    doc.end_group();
    assert_eq!(doc.undo_label(), Some("Brush"));
}

#[test]
fn dirty_follows_the_saved_state() {
    let mut doc = Document::new(Ds1::new(3, 3, 0));
    doc.paint(LayerRef::Floor(0), &[(0, 0, cv(1, 1))]).unwrap();
    doc.mark_saved();
    assert!(!doc.is_dirty());
    doc.paint(LayerRef::Floor(0), &[(1, 0, cv(1, 1))]).unwrap();
    assert!(doc.is_dirty());
    doc.undo();
    assert!(!doc.is_dirty());
    doc.undo();
    assert!(doc.is_dirty());
    doc.redo();
    assert!(!doc.is_dirty());
    // a different edit after undoing past the saved point stays dirty
    doc.undo();
    doc.paint(LayerRef::Floor(0), &[(2, 2, cv(2, 2))]).unwrap();
    assert!(doc.is_dirty());
    doc.undo();
    assert!(doc.is_dirty());
    doc.redo();
    // painting a change and painting it back is clean
    doc.mark_saved();
    doc.paint(LayerRef::Floor(0), &[(2, 2, cv(5, 5))]).unwrap();
    assert!(doc.is_dirty());
    doc.paint(LayerRef::Floor(0), &[(2, 2, cv(2, 2))]).unwrap();
    assert!(!doc.is_dirty());
}

#[test]
fn apply_patch_replace_restores_a_snapshot() {
    let mut doc = Document::new(Ds1::new(3, 3, 0));
    let snapshot = doc.ds1().clone();
    doc.paint(LayerRef::Floor(0), &[(0, 0, cv(1, 1))]).unwrap();
    doc.resize(5, 2).unwrap();
    let now = doc.ds1().clone();
    doc.apply_patch(Patch::replace("Restore", now, snapshot.clone()))
        .unwrap();
    assert_eq!(doc.ds1(), &snapshot);
    doc.undo();
    assert_eq!(doc.ds1().width, 5);
    noop!(
        doc,
        doc.apply_patch(Patch::replace("noop", snapshot.clone(), snapshot))
    );
}

// ----- layers ----------------------------------------------------------------

#[test]
fn move_wall0_to_wall2_adds_layers() {
    let mut ds = Ds1::new(6, 6, 0);
    ds.walls[0].tiles.set(1, 1, Cell(tile(1, 1)));
    ds.walls[0].orientations.set(1, 1, 5);
    ds.walls[0].tiles.set(2, 1, Cell(tile(1, 2)));
    ds.walls[0].orientations.set(2, 1, 6);
    ds.walls[0].tiles.set(4, 4, Cell(tile(3, 3)));
    let original = ds.clone();
    let mut doc = Document::new(ds);
    let mut sel = Selection::new();
    sel.box_select(
        doc.ds1(),
        CellRect::new((0, 0), (3, 3)),
        &[LayerRef::Wall(0)],
        false,
        SelectOp::Replace,
    );
    let report = doc
        .move_to_layer(
            &sel,
            LayerRef::Wall(0),
            LayerRef::Wall(2),
            MoveMode::Overwrite,
        )
        .unwrap();
    assert_eq!(report.moved, 2);
    assert_eq!(report.skipped_occupied, 0);
    assert_eq!(report.layer_added, Some(LayerRef::Wall(2)));
    let ds = doc.ds1();
    assert_eq!(ds.walls.len(), 3);
    assert!(ds.walls[1].tiles.as_slice().iter().all(|c| c.is_empty()));
    assert_eq!(LayerRef::Wall(2).get(ds, 1, 1), Some(wall(1, 1, 5)));
    assert_eq!(LayerRef::Wall(2).get(ds, 2, 1), Some(wall(1, 2, 6)));
    assert_eq!(LayerRef::Wall(0).get(ds, 1, 1), Some(CellValue::EMPTY));
    // not selected: stays
    assert_eq!(LayerRef::Wall(0).get(ds, 4, 4), Some(wall(3, 3, 0)));
    assert_roundtrips(ds);
    doc.undo();
    assert_eq!(doc.ds1(), &original);
    doc.redo();
    assert_eq!(doc.ds1().walls.len(), 3);
}

#[test]
fn move_needs_version_on_v3() {
    let mut ds = blank_v3(4, 4);
    ds.walls[0].tiles.set(1, 1, Cell(tile(1, 1)));
    let mut doc = Document::new(ds);
    let sel = select_all(&doc, &[LayerRef::Wall(0)]);
    let err = doc
        .move_to_layer(
            &sel,
            LayerRef::Wall(0),
            LayerRef::Wall(1),
            MoveMode::Overwrite,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        EditError::NeedsVersion {
            current: 3,
            required: 4
        }
    ));
    assert_eq!(doc.ds1().walls.len(), 1);
    assert!(!doc.can_undo());
    // the app then offers the upgrade (the empty tag layer cannot live in v4)
    doc.upgrade_version(4).unwrap();
    assert!(doc.ds1().tag.is_none());
    assert_roundtrips(doc.ds1());
    let r = doc
        .move_to_layer(
            &sel,
            LayerRef::Wall(0),
            LayerRef::Wall(1),
            MoveMode::Overwrite,
        )
        .unwrap();
    assert_eq!(r.layer_added, Some(LayerRef::Wall(1)));
    assert_roundtrips(doc.ds1());
    assert!(doc.undo() && doc.undo());
    assert_eq!(doc.ds1().version, 3);
    assert!(doc.ds1().tag.is_some());
}

#[test]
fn move_skip_occupied_counts_skipped() {
    let mut ds = Ds1::new(5, 1, 0);
    ds.add_wall_layer().unwrap();
    ds.walls[0].tiles.set(0, 0, Cell(tile(1, 1)));
    ds.walls[0].tiles.set(1, 0, Cell(tile(1, 2)));
    ds.walls[0].tiles.set(2, 0, Cell(tile(1, 3)));
    ds.walls[1].tiles.set(1, 0, Cell(tile(9, 9)));
    let mut doc = Document::new(ds);
    let sel = select_all(&doc, &[LayerRef::Wall(0)]);
    let r = doc
        .move_to_layer(
            &sel,
            LayerRef::Wall(0),
            LayerRef::Wall(1),
            MoveMode::SkipOccupied,
        )
        .unwrap();
    assert_eq!((r.moved, r.skipped_occupied, r.layer_added), (2, 1, None));
    let ds = doc.ds1();
    assert_eq!(LayerRef::Wall(1).get(ds, 1, 0), Some(cv(9, 9)));
    assert_eq!(LayerRef::Wall(0).get(ds, 1, 0), Some(cv(1, 2)));
    assert_eq!(LayerRef::Wall(1).get(ds, 0, 0), Some(cv(1, 1)));
    // overwrite takes the occupied one too
    let sel = select_all(&doc, &[LayerRef::Wall(0)]);
    let r = doc
        .move_to_layer(
            &sel,
            LayerRef::Wall(0),
            LayerRef::Wall(1),
            MoveMode::Overwrite,
        )
        .unwrap();
    assert_eq!(r.moved, 1);
    assert_eq!(LayerRef::Wall(1).get(doc.ds1(), 1, 0), Some(cv(1, 2)));
    doc.undo();
    assert_eq!(LayerRef::Wall(1).get(doc.ds1(), 1, 0), Some(cv(9, 9)));
}

#[test]
fn move_between_wall_and_floor_zeroes_orientation() {
    let mut ds = Ds1::new(3, 1, 0);
    ds.walls[0].tiles.set(0, 0, Cell(tile(1, 1)));
    ds.walls[0].orientations.set(0, 0, 4);
    ds.floors[0].set(1, 0, Cell(tile(2, 2)));
    let mut doc = Document::new(ds);
    let sel = select_all(&doc, &[LayerRef::Wall(0)]);
    doc.move_to_layer(
        &sel,
        LayerRef::Wall(0),
        LayerRef::Floor(0),
        MoveMode::Overwrite,
    )
    .unwrap();
    assert_eq!(LayerRef::Floor(0).get(doc.ds1(), 0, 0), Some(cv(1, 1)));
    assert_eq!(doc.ds1().walls[0].orientations.get(0, 0), Some(0));
    let sel = select_all(&doc, &[LayerRef::Floor(0)]);
    doc.move_to_layer(
        &sel,
        LayerRef::Floor(0),
        LayerRef::Wall(0),
        MoveMode::Overwrite,
    )
    .unwrap();
    assert_eq!(LayerRef::Wall(0).get(doc.ds1(), 1, 0), Some(wall(2, 2, 0)));
    // misc errors
    assert!(matches!(
        doc.move_to_layer(
            &sel,
            LayerRef::Wall(3),
            LayerRef::Wall(0),
            MoveMode::Overwrite
        ),
        Err(EditError::NoSuchLayer(_))
    ));
    assert!(matches!(
        doc.move_to_layer(
            &sel,
            LayerRef::Wall(0),
            LayerRef::Wall(4),
            MoveMode::Overwrite
        ),
        Err(EditError::LayerLimit(_))
    ));
    assert!(matches!(
        doc.move_to_layer(&sel, LayerRef::Wall(0), LayerRef::Tag, MoveMode::Overwrite),
        Err(EditError::LayerLimit(_))
    ));
    noop!(
        doc,
        doc.move_to_layer(
            &Selection::new(),
            LayerRef::Wall(0),
            LayerRef::Wall(1),
            MoveMode::Overwrite
        )
    );
    assert_eq!(doc.ds1().walls.len(), 1, "nothing to move adds no layer");
}

#[test]
fn add_and_remove_layers() {
    let mut ds = Ds1::new(3, 3, 0);
    ds.add_wall_layer().unwrap();
    ds.add_wall_layer().unwrap();
    ds.walls[1].tiles.set(0, 0, Cell(tile(1, 1)));
    ds.walls[2].tiles.set(1, 1, Cell(tile(2, 2)));
    ds.walls[2].orientations.set(1, 1, 3);
    let original = ds.clone();
    let mut doc = Document::new(ds);

    doc.remove_layer(LayerRef::Wall(1)).unwrap();
    assert_eq!(doc.ds1().walls.len(), 2);
    assert_eq!(LayerRef::Wall(1).get(doc.ds1(), 1, 1), Some(wall(2, 2, 3)));
    assert_roundtrips(doc.ds1());
    doc.undo();
    assert_eq!(doc.ds1(), &original);

    noop!(doc, doc.add_layer(LayerRef::Wall(1)));
    doc.add_layer(LayerRef::Wall(3)).unwrap();
    assert!(matches!(
        doc.add_layer(LayerRef::Wall(4)),
        Err(EditError::LayerLimit(_))
    ));
    doc.add_layer(LayerRef::Floor(1)).unwrap();
    assert!(matches!(
        doc.add_layer(LayerRef::Floor(2)),
        Err(EditError::LayerLimit(_))
    ));
    assert_eq!(doc.ds1().floors.len(), 2);
    doc.remove_layer(LayerRef::Floor(0)).unwrap();
    assert!(matches!(
        doc.remove_layer(LayerRef::Floor(1)),
        Err(EditError::NoSuchLayer(_))
    ));
    assert!(matches!(
        doc.remove_layer(LayerRef::Shadow),
        Err(EditError::LayerLimit(_))
    ));
    assert_roundtrips(doc.ds1());

    // old versions keep exactly one of each
    let mut old = Document::new(blank_v3(2, 2));
    assert!(matches!(
        old.remove_layer(LayerRef::Wall(0)),
        Err(EditError::LayerLimit(_))
    ));
    assert!(matches!(
        old.remove_layer(LayerRef::Tag),
        Err(EditError::LayerLimit(_))
    ));
    assert!(matches!(
        old.add_layer(LayerRef::Floor(1)),
        Err(EditError::NeedsVersion { required: 16, .. })
    ));
    noop!(old, old.add_layer(LayerRef::Tag));
}

#[test]
fn tag_layer_add_remove_and_groups() {
    let mut ds = Ds1::new(3, 3, 0);
    let mut doc = Document::new(ds.clone());
    doc.add_layer(LayerRef::Tag).unwrap();
    assert_eq!(doc.ds1().tag_type, 1);
    assert!(doc.ds1().tag.is_some());
    assert_roundtrips(doc.ds1());
    doc.paint(
        LayerRef::Tag,
        &[(
            1,
            1,
            CellValue {
                cell: 0x100,
                orientation: 5,
            },
        )],
    )
    .unwrap();
    assert_eq!(
        LayerRef::Tag.get(doc.ds1(), 1, 1),
        Some(CellValue {
            cell: 0x100,
            orientation: 0
        })
    );
    // tag value 0x100 has prop1 == 0 yet is not empty in the tag layer
    assert!(LayerRef::Tag.get(doc.ds1(), 1, 1).unwrap().is_empty());
    assert!(!LayerRef::Tag.is_empty_value(LayerRef::Tag.get(doc.ds1(), 1, 1).unwrap()));
    doc.undo();

    ds.tag_type = 2;
    ds.tag = Some(Grid::new(3, 3));
    ds.groups.push(Group {
        x: 1,
        y: 1,
        width: 2,
        height: 2,
        unknown: 0,
    });
    ds.preserved.pre_group_dword = 3;
    assert_roundtrips(&ds);
    let mut doc = Document::new(ds.clone());
    doc.remove_layer(LayerRef::Tag).unwrap();
    assert!(doc.ds1().tag.is_none());
    assert_eq!(doc.ds1().tag_type, 0);
    assert!(doc.ds1().groups.is_empty());
    assert_roundtrips(doc.ds1());
    doc.undo();
    assert_eq!(doc.ds1(), &ds);
}

#[test]
fn resize_keeps_top_left_and_undo_restores() {
    let mut ds = Ds1::new(4, 4, 0);
    ds.add_wall_layer().unwrap();
    ds.walls[1].tiles.set(3, 3, Cell(tile(1, 1)));
    ds.walls[1].orientations.set(3, 3, 9);
    ds.floors[0].set(0, 0, Cell(tile(2, 2)));
    ds.floors[0].set(3, 0, Cell(tile(2, 3)));
    ds.shadow.set(1, 3, Cell(tile(4, 4)));
    let original = ds.clone();
    let mut doc = Document::new(ds);
    doc.resize(2, 3).unwrap();
    let d = doc.ds1();
    assert_eq!((d.width, d.height), (2, 3));
    assert_eq!(LayerRef::Floor(0).get(d, 0, 0), Some(cv(2, 2)));
    assert_eq!(d.walls[1].tiles.width(), 2);
    assert_roundtrips(d);
    doc.undo();
    assert_eq!(doc.ds1(), &original);
    doc.resize(6, 6).unwrap();
    assert_eq!(LayerRef::Wall(1).get(doc.ds1(), 3, 3), Some(wall(1, 1, 9)));
    assert_eq!(
        LayerRef::Wall(1).get(doc.ds1(), 5, 5),
        Some(CellValue::EMPTY)
    );
    assert_roundtrips(doc.ds1());
    doc.undo();
    assert_eq!(doc.ds1(), &original);
    noop!(doc, doc.resize(4, 4));
    assert!(matches!(doc.resize(0, 4), Err(EditError::LayerLimit(_))));
    assert!(matches!(
        doc.resize(u32::MAX, u32::MAX),
        Err(EditError::LayerLimit(_))
    ));
}

#[test]
fn upgrade_version_rules() {
    // v3 -> v7 remaps orientations through DIR_LOOKUP and keeps the tag layer out of v4..9
    let mut ds = blank_v3(2, 2);
    ds.walls[0].tiles.set(0, 0, Cell(tile(1, 1)));
    ds.walls[0].orientations.set(0, 0, 3); // DIR_LOOKUP[3] == 1
    ds.walls[0].tiles.set(1, 0, Cell(tile(1, 1)));
    ds.walls[0].orientations.set(1, 0, 0x0100_0007); // low byte 7 -> 5, high bits kept
    ds.walls[0].orientations.set(1, 1, 200); // outside the table: untouched
    ds.tag.as_mut().unwrap().set(1, 1, 5);
    let original = ds.clone();
    let before_orientation = (ds.orientation(0, 0, 0), ds.orientation(0, 1, 0));
    let mut doc = Document::new(ds);
    assert!(matches!(
        doc.upgrade_version(6),
        Err(EditError::NeedsVersion {
            current: 3,
            required: 10
        })
    ));
    assert!(matches!(doc.upgrade_version(2), Err(EditError::Format(_))));
    noop!(doc, doc.upgrade_version(3));
    doc.upgrade_version(18).unwrap();
    let d = doc.ds1();
    assert_eq!(d.version, 18);
    assert_eq!(d.tag_type, 1);
    assert_eq!(d.tag.as_ref().unwrap().get(1, 1), Some(5));
    assert_eq!(
        (d.orientation(0, 0, 0), d.orientation(0, 1, 0)),
        before_orientation
    );
    assert_eq!(d.walls[0].orientations.get(1, 0), Some(0x0100_0005));
    assert_eq!(d.walls[0].orientations.get(1, 1), Some(200));
    assert_roundtrips(d);
    assert!(doc.undo());
    assert_eq!(doc.ds1(), &original);

    // an empty tag layer is dropped; unknown v9..13 words are zeroed above 13
    let mut ds = Ds1::new(2, 2, 0);
    ds.version = 12;
    ds.preserved.unknown_v9_13 = [5000, 5000];
    assert_roundtrips(&ds);
    let mut doc = Document::new(ds.clone());
    doc.upgrade_version(14).unwrap();
    assert_eq!(doc.ds1().preserved.unknown_v9_13, [0, 0]);
    assert_roundtrips(doc.ds1());
    doc.undo();
    assert_eq!(doc.ds1(), &ds);
}

// ----- selection -------------------------------------------------------------

fn full_floor_doc(w: u32, h: u32) -> Document {
    let mut ds = Ds1::new(w, h, 0);
    fill_floor(&mut ds, 0, cv(1, 1));
    Document::new(ds)
}

#[test]
fn box_select_and_ops() {
    let doc = full_floor_doc(6, 6);
    let ds = doc.ds1();
    let floor = [LayerRef::Floor(0)];
    let mut sel = Selection::new();
    sel.box_select(
        ds,
        CellRect::new((3, 3), (1, 1)),
        &floor,
        false,
        SelectOp::Replace,
    );
    assert_eq!(sel.len(), 9);
    assert_eq!(sel.bounds(), Some(CellRect::new((1, 1), (3, 3))));
    assert_eq!(sel.layers(), vec![LayerRef::Floor(0)]);
    assert!(sel.contains(LayerRef::Floor(0), 2, 2));

    // Add
    sel.box_select(
        ds,
        CellRect::new((3, 3), (4, 4)),
        &floor,
        false,
        SelectOp::Add,
    );
    assert_eq!(sel.len(), 9 + 3);
    // Subtract
    sel.box_select(
        ds,
        CellRect::new((0, 0), (2, 2)),
        &floor,
        false,
        SelectOp::Subtract,
    );
    assert_eq!(sel.len(), 12 - 4);
    assert!(!sel.contains(LayerRef::Floor(0), 1, 1));
    // Intersect
    sel.box_select(
        ds,
        CellRect::new((3, 0), (5, 5)),
        &floor,
        false,
        SelectOp::Intersect,
    );
    assert_eq!(sel.len(), 6); // (3,1) (3,2) (3,3) (4,3) (3,4) (4,4)
    assert!(sel.contains(LayerRef::Floor(0), 4, 4));
    assert!(!sel.contains(LayerRef::Floor(0), 2, 3));
    // Replace
    sel.box_select(
        ds,
        CellRect::new((0, 0), (0, 0)),
        &floor,
        false,
        SelectOp::Replace,
    );
    assert_eq!(sel.len(), 1);
    // clipped to the map, missing layers skipped
    sel.box_select(
        ds,
        CellRect::new((4, 4), (50, 50)),
        &[LayerRef::Floor(0), LayerRef::Wall(2)],
        false,
        SelectOp::Replace,
    );
    assert_eq!(sel.len(), 4);
    // empty cells only with include_empty
    sel.box_select(
        ds,
        CellRect::new((0, 0), (1, 1)),
        &[LayerRef::Shadow],
        false,
        SelectOp::Replace,
    );
    assert!(sel.is_empty());
    sel.box_select(
        ds,
        CellRect::new((0, 0), (1, 1)),
        &[LayerRef::Shadow],
        true,
        SelectOp::Replace,
    );
    assert_eq!(sel.len(), 4);
    assert!(sel.insert(LayerRef::Floor(0), 5, 5));
    assert!(!sel.insert(LayerRef::Floor(0), 5, 5));
    sel.clear();
    assert!(sel.bounds().is_none());
}

#[test]
fn lasso_concave_polygon() {
    let doc = full_floor_doc(8, 8);
    let ds = doc.ds1();
    // an L shape: the top bar (6x2) plus the left bar (2x4 below it)
    let poly = [
        (0.0, 0.0),
        (6.0, 0.0),
        (6.0, 2.0),
        (2.0, 2.0),
        (2.0, 6.0),
        (0.0, 6.0),
    ];
    let mut sel = Selection::new();
    sel.lasso(ds, &poly, &[LayerRef::Floor(0)], SelectOp::Replace);
    assert_eq!(sel.len(), 12 + 8);
    assert!(sel.contains(LayerRef::Floor(0), 5, 1));
    assert!(sel.contains(LayerRef::Floor(0), 1, 5));
    assert!(!sel.contains(LayerRef::Floor(0), 3, 3), "inside the notch");
    assert!(!sel.contains(LayerRef::Floor(0), 6, 0));
    // a cell is in when its centre is: this triangle holds (0,0)'s centre (0.5,0.5) only
    let tri = [(0.0, 0.0), (1.2, 0.0), (0.0, 1.2)];
    sel.lasso(ds, &tri, &[LayerRef::Floor(0)], SelectOp::Replace);
    assert_eq!(sel.len(), 1);
    // degenerate input selects nothing
    sel.lasso(ds, &poly[..2], &[LayerRef::Floor(0)], SelectOp::Replace);
    assert!(sel.is_empty());
    sel.lasso(
        ds,
        &[(f32::NAN, 0.0), (1.0, 0.0), (1.0, 1.0)],
        &[LayerRef::Floor(0)],
        SelectOp::Replace,
    );
    assert!(sel.is_empty());
    // polygons far outside the map or with huge coordinates are fine
    sel.lasso(
        ds,
        &[(-1e30, -1e30), (1e30, -1e30), (1e30, 1e30), (-1e30, 1e30)],
        &[LayerRef::Floor(0)],
        SelectOp::Replace,
    );
    assert_eq!(sel.len(), 64);
    sel.lasso(
        ds,
        &[(-9.0, -9.0), (-5.0, -9.0), (-5.0, -5.0)],
        &[LayerRef::Floor(0)],
        SelectOp::Replace,
    );
    assert!(sel.is_empty());
    // Add / Subtract / Intersect
    sel.lasso(ds, &poly, &[LayerRef::Floor(0)], SelectOp::Replace);
    let right = [(4.0, 0.0), (8.0, 0.0), (8.0, 8.0), (4.0, 8.0)];
    sel.lasso(ds, &right, &[LayerRef::Floor(0)], SelectOp::Intersect);
    assert_eq!(sel.len(), 4); // x 4..6, y 0..2
    sel.lasso(ds, &right, &[LayerRef::Floor(0)], SelectOp::Add);
    assert_eq!(sel.len(), 32);
    sel.lasso(ds, &right, &[LayerRef::Floor(0)], SelectOp::Subtract);
    assert!(sel.is_empty());
}

#[test]
fn select_by_tile_id_and_identical() {
    let mut ds = Ds1::new(5, 2, 0);
    ds.add_wall_layer().unwrap();
    ds.floors[0].set(0, 0, Cell(tile(2, 3)));
    ds.floors[0].set(1, 0, Cell(tile(2, 4)));
    ds.floors[0].set(2, 0, Cell(tile(2, 3) | (1 << 31))); // hidden: same tile, other bits
    ds.walls[0].tiles.set(0, 1, Cell(tile(2, 3)));
    ds.walls[0].orientations.set(0, 1, 4);
    ds.walls[1].tiles.set(1, 1, Cell(tile(2, 3)));
    ds.walls[1].orientations.set(1, 1, 5);
    ds.walls[1].tiles.set(2, 1, Cell(tile(2, 3)));
    ds.walls[1].orientations.set(2, 1, 4);
    let doc = Document::new(ds);
    let ds = doc.ds1();
    let layers = [
        LayerRef::Floor(0),
        LayerRef::Wall(0),
        LayerRef::Wall(1),
        LayerRef::Shadow,
    ];
    let mut sel = Selection::new();

    sel.by_tile_id(ds, 2, 3, None, &layers, SelectOp::Replace);
    assert_eq!(sel.len(), 5);
    sel.by_tile_id(ds, 2, 3, Some(4), &layers, SelectOp::Replace);
    assert_eq!(
        sel.iter().collect::<Vec<_>>(),
        vec![(LayerRef::Wall(0), 0, 1), (LayerRef::Wall(1), 2, 1),]
    );
    sel.by_tile_id(ds, 2, 3, Some(0), &layers, SelectOp::Replace);
    assert_eq!(sel.len(), 2, "floors have orientation 0");
    sel.by_tile_id(ds, 9, 9, None, &layers, SelectOp::Replace);
    assert!(sel.is_empty());

    // identical: same indices, orientation and flags
    sel.identical(ds, (LayerRef::Wall(0), 0, 1), &layers, SelectOp::Replace);
    assert_eq!(
        sel.iter().collect::<Vec<_>>(),
        vec![(LayerRef::Wall(0), 0, 1), (LayerRef::Wall(1), 2, 1)]
    );
    sel.identical(ds, (LayerRef::Floor(0), 0, 0), &layers, SelectOp::Replace);
    assert_eq!(sel.len(), 1, "the hidden copy differs");
    sel.identical(ds, (LayerRef::Floor(0), 0, 0), &layers, SelectOp::Add);
    sel.identical(ds, (LayerRef::Floor(0), 1, 0), &layers, SelectOp::Add);
    assert_eq!(sel.len(), 2);
    sel.identical(ds, (LayerRef::Floor(0), 1, 0), &layers, SelectOp::Subtract);
    assert_eq!(sel.len(), 1);
    sel.identical(ds, (LayerRef::Floor(0), 1, 0), &layers, SelectOp::Intersect);
    assert!(sel.is_empty());
    // empty or missing seeds select nothing
    sel.identical(ds, (LayerRef::Shadow, 0, 0), &layers, SelectOp::Replace);
    assert!(sel.is_empty());
    sel.identical(ds, (LayerRef::Wall(3), 0, 0), &layers, SelectOp::Replace);
    assert!(sel.is_empty());
}

// ----- clipboard -------------------------------------------------------------

fn source_doc() -> Document {
    let mut ds = Ds1::new(8, 8, 1);
    ds.add_wall_layer().unwrap();
    ds.add_wall_layer().unwrap();
    ds.floors[0].set(2, 2, Cell(tile(1, 1)));
    ds.floors[0].set(3, 2, Cell(tile(1, 2)));
    ds.walls[0].tiles.set(2, 3, Cell(tile(3, 1)));
    ds.walls[0].orientations.set(2, 3, 4);
    ds.walls[2].tiles.set(3, 3, Cell(tile(3, 2)));
    ds.walls[2].orientations.set(3, 3, 6);
    ds.objects.push(Object {
        kind: 1,
        id: 7,
        x: 11,
        y: 12,
        flags: 2,
    }); // inside cells (2..3, 2..3)
    ds.objects.push(Object {
        kind: 2,
        id: 8,
        x: 40,
        y: 40,
        flags: 0,
    }); // outside
    Document::new(ds)
}

#[test]
fn copy_is_relative_and_carries_objects() {
    let mut src = source_doc();
    src.set_clip_context(Some("data/global/x.ds1".into()), Some(4));
    let sel = select_all(
        &src,
        &[LayerRef::Floor(0), LayerRef::Wall(0), LayerRef::Wall(2)],
    );
    let clip = src.copy(&sel);
    assert_eq!((clip.width, clip.height), (2, 2));
    assert_eq!(clip.cells.len(), 4);
    assert!(clip.cells.contains(&(LayerRef::Floor(0), 0, 0, cv(1, 1))));
    assert!(
        clip.cells
            .contains(&(LayerRef::Wall(2), 1, 1, wall(3, 2, 6)))
    );
    assert_eq!(
        clip.objects,
        vec![Object {
            kind: 1,
            id: 7,
            x: 1,
            y: 2,
            flags: 2
        }]
    );
    assert_eq!(
        clip.source,
        ClipSource {
            game_path: Some("data/global/x.ds1".into()),
            act: 1,
            lvl_type: Some(4),
            version: 18
        }
    );
    assert_eq!(Clipboard::from_json(&clip.to_json()), Some(clip.clone()));
    assert_eq!(Clipboard::from_json("not json"), None);
    assert_eq!(Clipboard::from_json("{\"a\":1}"), None);
    // copy of nothing is an empty clip
    let none = src.copy(&Selection::new());
    assert!(none.cells.is_empty() && none.objects.is_empty());
}

#[test]
fn cross_document_paste_with_offset() {
    let src = source_doc();
    let sel = select_all(&src, &[LayerRef::Floor(0), LayerRef::Wall(0)]);
    let clip = src.copy(&sel);
    let clip = Clipboard::from_json(&clip.to_json()).unwrap();

    let mut dst = Document::new(Ds1::new(5, 5, 1));
    let before = dst.ds1().clone();
    let preview = dst.paste(&clip, (3, 1), PasteMode::Overwrite).unwrap();
    // clip cells: floor (0,0) (1,0), wall0 (0,1); the right column x=4 fits (3+1)
    assert_eq!(preview.writes, 3);
    assert_eq!(preview.out_of_bounds, 0);
    assert!(preview.conflicts.is_empty() && preview.layers_to_add.is_empty());
    assert_eq!(LayerRef::Floor(0).get(dst.ds1(), 3, 1), Some(cv(1, 1)));
    assert_eq!(LayerRef::Floor(0).get(dst.ds1(), 4, 1), Some(cv(1, 2)));
    assert_eq!(LayerRef::Wall(0).get(dst.ds1(), 3, 2), Some(wall(3, 1, 4)));
    assert_eq!(
        dst.ds1().objects,
        vec![Object {
            kind: 1,
            id: 7,
            x: 3 * 5 + 1,
            y: 5 + 2,
            flags: 2
        }]
    );
    assert_roundtrips(dst.ds1());
    dst.undo();
    assert_eq!(dst.ds1(), &before);

    // out of bounds: shifted so only part lands
    let p = dst.preview_paste(&clip, (4, 4), PasteMode::Overwrite);
    assert_eq!(p.writes, 1);
    assert_eq!(p.out_of_bounds, 2);
    assert!(p.warnings.is_empty(), "{p:?}");
    let p = dst.preview_paste(&clip, (-1, -1), PasteMode::Overwrite);
    assert_eq!(p.out_of_bounds, 3);
    assert_eq!(p.writes, 0);
    assert!(p.warnings.iter().any(|w| w.contains("object")), "{p:?}");
    noop!(dst, dst.paste(&clip, (-9, -9), PasteMode::Overwrite));
    let none = dst.paste(&clip, (-9, -9), PasteMode::Overwrite).unwrap();
    assert_eq!((none.writes, none.out_of_bounds), (0, 3));
    assert!(!dst.can_undo());
    // a preview never changes anything
    assert_eq!(dst.ds1(), &before);
}

#[test]
fn paste_conflicts_and_modes() {
    let mut src = Document::new(Ds1::new(4, 4, 0));
    src.paint(LayerRef::Floor(0), &[(0, 0, cv(1, 1)), (1, 0, cv(1, 2))])
        .unwrap();
    src.paint(LayerRef::Wall(0), &[(0, 0, wall(5, 5, 2))])
        .unwrap();
    let sel = select_all(&src, &[LayerRef::Floor(0), LayerRef::Wall(0)]);
    let clip = src.copy(&sel);

    let mut ds = Ds1::new(4, 4, 0);
    ds.floors[0].set(0, 0, Cell(tile(9, 9))); // differs: conflict
    ds.floors[0].set(1, 0, Cell(tile(1, 2))); // identical: no conflict, no write
    let mut dst = Document::new(ds);
    let p = dst.preview_paste(&clip, (0, 0), PasteMode::Overwrite);
    assert_eq!(p.writes, 2);
    assert_eq!(
        p.conflicts,
        vec![capy_edit::PasteConflict {
            layer: LayerRef::Floor(0),
            x: 0,
            y: 0,
            existing: cv(9, 9),
            incoming: cv(1, 1)
        }]
    );
    let p = dst.preview_paste(&clip, (0, 0), PasteMode::SkipOccupied);
    assert_eq!(p.writes, 1, "only the wall lands");
    assert_eq!(p.conflicts.len(), 1);
    let p = dst.preview_paste(&clip, (0, 0), PasteMode::OnlyIntoEmptyLayers);
    assert_eq!(p.writes, 1, "the floor layer is occupied");

    dst.paste(&clip, (0, 0), PasteMode::SkipOccupied).unwrap();
    assert_eq!(LayerRef::Floor(0).get(dst.ds1(), 0, 0), Some(cv(9, 9)));
    assert_eq!(LayerRef::Wall(0).get(dst.ds1(), 0, 0), Some(wall(5, 5, 2)));
    dst.undo();
    dst.paste(&clip, (0, 0), PasteMode::Overwrite).unwrap();
    assert_eq!(LayerRef::Floor(0).get(dst.ds1(), 0, 0), Some(cv(1, 1)));
}

#[test]
fn paste_adds_layers_and_reports_versions() {
    let src = source_doc();
    let sel = select_all(&src, &[LayerRef::Wall(2), LayerRef::Wall(0)]);
    let clip = src.copy(&sel);

    // v18 target with one wall: Wall(1) and Wall(2) get created
    let mut dst = Document::new(Ds1::new(8, 8, 1));
    let p = dst.preview_paste(&clip, (0, 0), PasteMode::Overwrite);
    assert_eq!(p.layers_to_add, vec![LayerRef::Wall(1), LayerRef::Wall(2)]);
    assert_eq!(p.needs_version, None);
    let before = dst.ds1().clone();
    dst.paste(&clip, (0, 0), PasteMode::Overwrite).unwrap();
    assert_eq!(dst.ds1().walls.len(), 3);
    assert_roundtrips(dst.ds1());
    dst.undo();
    assert_eq!(dst.ds1(), &before);

    // v3 target
    let mut v3 = Document::new(blank_v3(8, 8));
    let p = v3.preview_paste(&clip, (0, 0), PasteMode::Overwrite);
    assert_eq!(p.needs_version, Some(4));
    assert_eq!(p.layers_to_add, vec![LayerRef::Wall(1), LayerRef::Wall(2)]);
    assert!(matches!(
        v3.paste(&clip, (0, 0), PasteMode::Overwrite),
        Err(EditError::NeedsVersion {
            current: 3,
            required: 4
        })
    ));
    assert!(!v3.can_undo());

    // flags on pasted objects need version 6; objects need version 2
    let mut obj_clip = clip.clone();
    obj_clip.cells.clear();
    obj_clip.objects = vec![Object {
        kind: 1,
        id: 1,
        x: 0,
        y: 0,
        flags: 1,
    }];
    let mut v5 = Ds1::new(8, 8, 0);
    v5.version = 5;
    v5.tag = Some(Grid::new(8, 8));
    v5.tag_type = 0;
    // v5 cannot hold a tag layer; drop it for a valid map
    v5.tag = None;
    let v5 = Document::new(v5);
    assert_eq!(
        v5.preview_paste(&obj_clip, (0, 0), PasteMode::Overwrite)
            .needs_version,
        Some(6)
    );
}

#[test]
fn paste_warns_about_source_mismatch() {
    let mut src = source_doc();
    src.set_clip_context(None, Some(3));
    let sel = select_all(&src, &[LayerRef::Floor(0)]);
    let clip = src.copy(&sel);
    let mut dst = Document::new(Ds1::new(8, 8, 2));
    dst.set_clip_context(None, Some(5));
    let p = dst.preview_paste(&clip, (0, 0), PasteMode::Overwrite);
    assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
    assert!(p.warnings[0].contains("act 2") && p.warnings[0].contains("act 3"));
    assert!(p.warnings[1].contains("LvlType 3"));
    // same act and unknown LvlType: nothing to warn about
    let mut same = Document::new(Ds1::new(8, 8, 1));
    same.set_clip_context(None, None);
    assert!(
        same.preview_paste(&clip, (0, 0), PasteMode::Overwrite)
            .warnings
            .is_empty()
    );
    // clips from a hostile source never panic
    let mut bad = clip.clone();
    bad.cells.push((LayerRef::Wall(9), 0, 0, cv(1, 1)));
    bad.cells
        .push((LayerRef::Floor(0), u32::MAX, u32::MAX, cv(1, 1)));
    bad.objects.push(Object {
        kind: 0,
        id: 0,
        x: i32::MAX,
        y: i32::MIN,
        flags: 0,
    });
    let p = dst.preview_paste(&bad, (i32::MAX, i32::MIN), PasteMode::Overwrite);
    assert_eq!(p.writes, 0);
    let _ = dst.preview_paste(&bad, (0, 0), PasteMode::Overwrite);
}

#[test]
fn cut_removes_cells_and_objects() {
    let mut doc = source_doc();
    doc.paint(LayerRef::Floor(0), &[(7, 7, cv(8, 8))]).unwrap();
    let before = doc.ds1().clone();
    let sel = select_all(&doc, &[LayerRef::Floor(0), LayerRef::Wall(0)]);
    let n = sel.len();
    let clip = doc.cut(&sel).unwrap();
    assert_eq!(clip.cells.len(), n);
    assert_eq!(clip.width, 6);
    assert_eq!(
        doc.ds1().objects.len(),
        1,
        "the object inside the bounds is cut"
    );
    assert_eq!(
        LayerRef::Floor(0).get(doc.ds1(), 2, 2),
        Some(CellValue::EMPTY)
    );
    assert_roundtrips(doc.ds1());
    assert!(doc.undo());
    assert_eq!(doc.ds1(), &before);
    noop!(doc, doc.cut(&Selection::new()));
}

// ----- objects ---------------------------------------------------------------

#[test]
fn object_edits_and_npc_paths() {
    let mut ds = Ds1::new(4, 4, 0);
    ds.objects = vec![
        Object {
            kind: 1,
            id: 1,
            x: 3,
            y: 3,
            flags: 0,
        },
        Object {
            kind: 1,
            id: 2,
            x: 6,
            y: 6,
            flags: 0,
        },
        Object {
            kind: 1,
            id: 3,
            x: 9,
            y: 9,
            flags: 0,
        },
    ];
    ds.npc_paths = vec![
        NpcPath {
            x: 6,
            y: 6,
            points: vec![PathPoint {
                x: 7,
                y: 7,
                action: 1,
            }],
        },
        NpcPath {
            x: 9,
            y: 9,
            points: vec![PathPoint {
                x: 8,
                y: 8,
                action: 2,
            }],
        },
    ];
    assert_roundtrips(&ds);
    let original = ds.clone();
    let mut doc = Document::new(ds);

    let i = doc
        .add_object(Object {
            kind: 2,
            id: 9,
            x: 1,
            y: 1,
            flags: 4,
        })
        .unwrap();
    assert_eq!(i, 3);
    doc.undo();
    assert_eq!(doc.ds1(), &original);

    doc.move_objects(&[1, 1, 0], 2, -1).unwrap();
    let d = doc.ds1();
    assert_eq!((d.objects[0].x, d.objects[0].y), (5, 2));
    assert_eq!((d.objects[1].x, d.objects[1].y), (8, 5));
    assert_eq!(
        (d.npc_paths[0].x, d.npc_paths[0].y),
        (8, 5),
        "path follows its object"
    );
    assert_eq!((d.npc_paths[1].x, d.npc_paths[1].y), (9, 9));
    assert_roundtrips(d);
    doc.undo();
    assert_eq!(doc.ds1(), &original);

    doc.delete_objects(&[2, 0]).unwrap();
    let d = doc.ds1();
    assert_eq!(d.objects.len(), 1);
    assert_eq!(d.npc_paths.len(), 1);
    assert_eq!(d.npc_paths[0].x, 6);
    assert_roundtrips(d);
    doc.undo();
    assert_eq!(doc.ds1(), &original);

    assert!(matches!(
        doc.delete_objects(&[3]),
        Err(EditError::Format(_))
    ));
    noop!(doc, doc.move_objects(&[0], 0, 0));
    noop!(doc, doc.move_objects(&[], 1, 1));
    assert!(matches!(
        doc.move_objects(&[0], i32::MAX, 0),
        Err(EditError::Format(_))
    ));
    noop!(doc, doc.delete_objects(&[]));

    // an object with flags does not fit version 3; the whole map is v3 here
    let mut v3 = Document::new(blank_v3(2, 2));
    assert!(matches!(
        v3.add_object(Object {
            kind: 1,
            id: 1,
            x: 0,
            y: 0,
            flags: 1
        }),
        Err(EditError::NeedsVersion {
            current: 3,
            required: 6
        })
    ));
    v3.add_object(Object {
        kind: 1,
        id: 1,
        x: 0,
        y: 0,
        flags: 0,
    })
    .unwrap();
    assert_roundtrips(v3.ds1());
}

// ----- diff -------------------------------------------------------------------

#[test]
fn diff_detects_each_field_class() {
    let mut base = Ds1::new(3, 3, 0);
    base.objects.push(Object {
        kind: 1,
        id: 1,
        x: 0,
        y: 0,
        flags: 0,
    });
    base.objects.push(Object {
        kind: 1,
        id: 2,
        x: 5,
        y: 5,
        flags: 0,
    });
    assert!(diff(&base, &base).is_empty());
    assert!(diff(&base, &base.clone()).is_empty());

    // cells
    let mut b = base.clone();
    b.floors[0].set(1, 1, Cell(tile(1, 1)));
    b.walls[0].orientations.set(2, 0, 3);
    let d = diff(&base, &b);
    assert_eq!(d.cells.len(), 2);
    assert!(d.cells.iter().any(|c| c.layer == LayerRef::Floor(0)
        && (c.x, c.y) == (1, 1)
        && c.before == CellValue::EMPTY
        && c.after == cv(1, 1)));
    assert!(!d.is_empty());
    assert!(d.size.is_none() && d.version.is_none() && !d.other_changed);

    // size: cells only over the common area
    let mut b = base.clone();
    b.floors[0].set(2, 2, Cell(tile(1, 1)));
    let mut doc = Document::new(b.clone());
    doc.resize(2, 2).unwrap();
    let d = diff(&b, doc.ds1());
    assert_eq!(d.size, Some(((3, 3), (2, 2))));
    assert!(d.cells.is_empty());

    // layers
    let mut b = base.clone();
    b.add_wall_layer().unwrap();
    b.tag_type = 0;
    let d = diff(&base, &b);
    assert_eq!(d.layers_added, vec![LayerRef::Wall(1)]);
    assert!(d.layers_removed.is_empty());
    let d = diff(&b, &base);
    assert_eq!(d.layers_removed, vec![LayerRef::Wall(1)]);

    // version
    let mut b = base.clone();
    b.version = 19;
    assert_eq!(diff(&base, &b).version, Some((18, 19)));

    // objects (multiset, order-independent)
    let mut b = base.clone();
    b.objects.reverse();
    let d = diff(&base, &b);
    assert!(d.objects_added.is_empty() && d.objects_removed.is_empty());
    assert!(d.other_changed, "a reorder is still a change");
    let mut b = base.clone();
    b.objects.remove(0);
    b.objects.push(Object {
        kind: 9,
        id: 9,
        x: 1,
        y: 1,
        flags: 0,
    });
    let d = diff(&base, &b);
    assert_eq!(
        d.objects_added,
        vec![Object {
            kind: 9,
            id: 9,
            x: 1,
            y: 1,
            flags: 0
        }]
    );
    assert_eq!(
        d.objects_removed,
        vec![Object {
            kind: 1,
            id: 1,
            x: 0,
            y: 0,
            flags: 0
        }]
    );

    // other: each of files, groups, paths, act, tag_type, preserved
    let tweaks: [fn(&mut Ds1); 6] = [
        |d| d.files.push(b"x".to_vec()),
        |d| {
            d.groups.push(Group {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                unknown: 0,
            })
        },
        |d| {
            d.npc_paths.push(NpcPath {
                x: 0,
                y: 0,
                points: vec![],
            })
        },
        |d| d.act = 3,
        |d| d.tag_type = 2,
        |d| d.preserved.trailing = vec![1],
    ];
    for tweak in tweaks {
        let mut b = base.clone();
        tweak(&mut b);
        let d = diff(&base, &b);
        assert!(d.other_changed);
        assert!(!d.is_empty());
    }
}

#[test]
fn diff_of_edit_matches_the_edit() {
    let mut doc = full_floor_doc(4, 4);
    let before = doc.ds1().clone();
    doc.paint(LayerRef::Floor(0), &[(0, 0, cv(7, 7))]).unwrap();
    doc.add_layer(LayerRef::Wall(1)).unwrap();
    let d = diff(&before, doc.ds1());
    assert_eq!(d.cells.len(), 1);
    assert_eq!(d.layers_added, vec![LayerRef::Wall(1)]);
    doc.undo();
    doc.undo();
    assert!(diff(&before, doc.ds1()).is_empty());
}

// ----- misc ---------------------------------------------------------------------

#[test]
fn layer_ref_helpers() {
    let mut ds = Ds1::new(2, 2, 0);
    ds.add_wall_layer().unwrap();
    ds.tag_type = 1;
    ds.tag = Some(Grid::new(2, 2));
    assert_eq!(
        LayerRef::all_in(&ds),
        vec![
            LayerRef::Floor(0),
            LayerRef::Wall(0),
            LayerRef::Wall(1),
            LayerRef::Shadow,
            LayerRef::Tag
        ]
    );
    assert!(LayerRef::Wall(1).exists_in(&ds) && !LayerRef::Wall(2).exists_in(&ds));
    assert_eq!(LayerRef::Shadow.get(&ds, 2, 0), None);
    assert_eq!(LayerRef::Wall(5).get(&ds, 0, 0), None);
    assert!(CellValue::EMPTY.is_empty());
    assert!(!cv(1, 1).is_empty());
}

#[test]
fn edits_on_a_map_without_walls_or_floors() {
    // layer counts of 0 are legal in v18
    let mut ds = Ds1::new(2, 2, 0);
    ds.walls.clear();
    ds.floors.clear();
    assert_roundtrips(&ds);
    let mut doc = Document::new(ds);
    assert!(matches!(
        doc.paint(LayerRef::Floor(0), &[(0, 0, cv(1, 1))]),
        Err(EditError::NoSuchLayer(_))
    ));
    doc.add_layer(LayerRef::Wall(0)).unwrap();
    doc.add_layer(LayerRef::Floor(0)).unwrap();
    assert_roundtrips(doc.ds1());
    doc.remove_layer(LayerRef::Wall(0)).unwrap();
    assert_roundtrips(doc.ds1());
}
