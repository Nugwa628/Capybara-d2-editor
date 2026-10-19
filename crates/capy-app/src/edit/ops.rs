//! Editing operations as data, so a failed attempt can be shown to the user
//! (for a missing DS1 version) and tried again after the map was upgraded.

use capy_edit::{
    CellValue, Clipboard, Document, EditError, LayerRef, MoveMode, MoveReport, PasteMode,
    PastePreview, Selection,
};

use crate::doc::layer_label;

/// One undoable change, or a copy/cut that may need a version.
#[derive(Clone, Debug)]
pub enum EditOp {
    Paint {
        layer: LayerRef,
        cells: Vec<(u32, u32, CellValue)>,
    },
    /// Empty the selected cells.
    Erase,
    SetFlags {
        hidden: Option<bool>,
        unwalkable: Option<bool>,
    },
    MoveToLayer {
        from: LayerRef,
        to: LayerRef,
        mode: MoveMode,
    },
    AddLayer(LayerRef),
    RemoveLayer(LayerRef),
    Resize {
        width: u32,
        height: u32,
    },
    UpgradeVersion(i32),
    Cut,
    Paste {
        clip: Box<Clipboard>,
        at: (i32, i32),
        mode: PasteMode,
    },
    MoveObjects {
        indices: Vec<usize>,
        dx: i32,
        dy: i32,
    },
    DeleteObjects(Vec<usize>),
    /// Place a new object (its index comes back as [`OpOutcome::Added`]).
    AddObject(d2_formats::ds1::Object),
    /// Replace one object's type, id, flags or position.
    SetObject {
        index: usize,
        object: d2_formats::ds1::Object,
    },
    AddPathPoint {
        object: usize,
        point: d2_formats::ds1::PathPoint,
        at: Option<usize>,
    },
    MovePathPoint {
        object: usize,
        index: usize,
        x: i32,
        y: i32,
    },
    SetPathAction {
        object: usize,
        index: usize,
        action: i32,
    },
    DeletePathPoints {
        object: usize,
        indices: Vec<usize>,
    },
}

impl EditOp {
    /// A short name for messages.
    pub fn name(&self) -> String {
        match self {
            EditOp::Paint { .. } => "Paint".to_owned(),
            EditOp::Erase => "Erase".to_owned(),
            EditOp::SetFlags { .. } => "Set flags".to_owned(),
            EditOp::MoveToLayer { from, to, .. } => {
                format!("Move {} to {}", layer_label(*from), layer_label(*to))
            }
            EditOp::AddLayer(l) => format!("Add {}", layer_label(*l)),
            EditOp::RemoveLayer(l) => format!("Remove {}", layer_label(*l)),
            EditOp::Resize { width, height } => format!("Resize to {width} x {height}"),
            EditOp::UpgradeVersion(v) => format!("Upgrade to v{v}"),
            EditOp::Cut => "Cut".to_owned(),
            EditOp::Paste { .. } => "Paste".to_owned(),
            EditOp::MoveObjects { .. } => "Move objects".to_owned(),
            EditOp::DeleteObjects(_) => "Delete objects".to_owned(),
            EditOp::AddObject(_) => "Add object".to_owned(),
            EditOp::SetObject { .. } => "Object properties".to_owned(),
            EditOp::AddPathPoint { .. } => "Add path point".to_owned(),
            EditOp::MovePathPoint { .. } => "Move path point".to_owned(),
            EditOp::SetPathAction { .. } => "Path action".to_owned(),
            EditOp::DeletePathPoints { .. } => "Delete path points".to_owned(),
        }
    }
}

/// What a successful operation produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpOutcome {
    Done,
    Moved(MoveReport),
    Pasted(PastePreview),
    Cut(Clipboard),
    /// The index of a placed object.
    Added(usize),
}

/// Result of one attempt.
#[derive(Debug)]
pub enum OpResult {
    Done(OpOutcome),
    /// The map's DS1 version is too old for the change; the user may upgrade.
    NeedsVersion {
        current: i32,
        required: i32,
    },
    Failed(EditError),
}

impl OpResult {
    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        matches!(self, OpResult::Done(_))
    }
}

fn wrap<T>(r: Result<T, EditError>, ok: impl FnOnce(T) -> OpOutcome) -> OpResult {
    match r {
        Ok(v) => OpResult::Done(ok(v)),
        Err(EditError::NeedsVersion { current, required }) => {
            OpResult::NeedsVersion { current, required }
        }
        Err(e) => OpResult::Failed(e),
    }
}

/// Runs `op` on `doc` once. A failure changes nothing (the document
/// guarantees that), so the same call can simply be repeated later.
pub fn run(doc: &mut Document, selection: &Selection, op: &EditOp) -> OpResult {
    match op {
        EditOp::Paint { layer, cells } => wrap(doc.paint(*layer, cells), |()| OpOutcome::Done),
        EditOp::Erase => wrap(doc.erase(selection), |()| OpOutcome::Done),
        EditOp::SetFlags { hidden, unwalkable } => {
            wrap(doc.set_cell_flags(selection, *hidden, *unwalkable), |()| {
                OpOutcome::Done
            })
        }
        EditOp::MoveToLayer { from, to, mode } => wrap(
            doc.move_to_layer(selection, *from, *to, *mode),
            OpOutcome::Moved,
        ),
        EditOp::AddLayer(l) => wrap(doc.add_layer(*l), |()| OpOutcome::Done),
        EditOp::RemoveLayer(l) => wrap(doc.remove_layer(*l), |()| OpOutcome::Done),
        EditOp::Resize { width, height } => wrap(doc.resize(*width, *height), |()| OpOutcome::Done),
        EditOp::UpgradeVersion(v) => wrap(doc.upgrade_version(*v), |()| OpOutcome::Done),
        EditOp::Cut => wrap(doc.cut(selection), OpOutcome::Cut),
        EditOp::Paste { clip, at, mode } => wrap(doc.paste(clip, *at, *mode), OpOutcome::Pasted),
        EditOp::MoveObjects { indices, dx, dy } => {
            wrap(doc.move_objects(indices, *dx, *dy), |()| OpOutcome::Done)
        }
        EditOp::DeleteObjects(indices) => wrap(doc.delete_objects(indices), |()| OpOutcome::Done),
        EditOp::AddObject(object) => wrap(doc.add_object(*object), OpOutcome::Added),
        EditOp::SetObject { index, object } => {
            wrap(doc.set_object(*index, *object), |()| OpOutcome::Done)
        }
        EditOp::AddPathPoint { object, point, at } => {
            wrap(doc.add_path_point(*object, *point, *at), |()| OpOutcome::Done)
        }
        EditOp::MovePathPoint { object, index, x, y } => {
            wrap(doc.move_path_point(*object, *index, *x, *y), |()| {
                OpOutcome::Done
            })
        }
        EditOp::SetPathAction {
            object,
            index,
            action,
        } => wrap(doc.set_path_action(*object, *index, *action), |()| {
            OpOutcome::Done
        }),
        EditOp::DeletePathPoints { object, indices } => {
            wrap(doc.delete_path_points(*object, indices), |()| OpOutcome::Done)
        }
    }
}

/// Upgrades the map to `required` and tries `op` again. The answer can be
/// `NeedsVersion` once more (the first hint is not always the final version,
/// or the upgrade itself needs a higher one); the caller then asks again.
pub fn upgrade_and_retry(
    doc: &mut Document,
    selection: &Selection,
    op: &EditOp,
    required: i32,
) -> OpResult {
    if let Err(e) = doc.upgrade_version(required) {
        return match e {
            EditError::NeedsVersion { current, required } => {
                OpResult::NeedsVersion { current, required }
            }
            other => OpResult::Failed(other),
        };
    }
    run(doc, selection, op)
}

/// Most upgrade rounds [`run_upgrading`] will follow; the DS1 versions that
/// matter form a short ladder, so more than this means something is wrong.
pub const MAX_UPGRADE_STEPS: usize = 6;

/// `op`, upgrading the map by itself whenever a newer version is asked for.
/// Returns the result and the versions upgraded to, in order. Used where
/// nobody can be asked (scripts, tests).
pub fn run_upgrading(
    doc: &mut Document,
    selection: &Selection,
    op: &EditOp,
) -> (OpResult, Vec<i32>) {
    let mut steps = Vec::new();
    let mut result = run(doc, selection, op);
    while let OpResult::NeedsVersion { current, required } = result {
        if steps.len() >= MAX_UPGRADE_STEPS || required <= current {
            result = OpResult::Failed(EditError::NeedsVersion { current, required });
            break;
        }
        steps.push(required);
        result = upgrade_and_retry(doc, selection, op, required);
    }
    (result, steps)
}

/// One-line summary of a move for the toast.
pub fn move_summary(from: LayerRef, to: LayerRef, report: &MoveReport) -> String {
    let mut text = format!(
        "Moved {} cell(s) from {} to {}",
        report.moved,
        layer_label(from),
        layer_label(to)
    );
    if report.skipped_occupied > 0 {
        text.push_str(&format!(
            "; {} skipped (target occupied)",
            report.skipped_occupied
        ));
    }
    if let Some(added) = report.layer_added {
        text.push_str(&format!("; added {}", layer_label(added)));
    }
    text
}

/// One-line summary of a finished paste for the toast.
pub fn paste_summary(preview: &PastePreview) -> String {
    let mut text = format!("Pasted: {} cell(s) written", preview.writes);
    if !preview.conflicts.is_empty() {
        text.push_str(&format!(", {} conflict(s)", preview.conflicts.len()));
    }
    if preview.out_of_bounds > 0 {
        text.push_str(&format!(", {} outside the map", preview.out_of_bounds));
    }
    if !preview.layers_to_add.is_empty() {
        let names: Vec<String> = preview
            .layers_to_add
            .iter()
            .map(|l| layer_label(*l))
            .collect();
        text.push_str(&format!(", added {}", names.join(", ")));
    }
    text
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use d2_formats::Grid;
    use d2_formats::ds1::{Cell, Ds1};

    fn doc_with_walls(version: i32) -> Document {
        let mut ds1 = Ds1::new(6, 6, 0);
        ds1.version = version;
        for y in 0..3 {
            for x in 0..3 {
                ds1.walls[0].tiles.set(x, y, Cell(1).with_indices(2, 1));
            }
        }
        Document::new(ds1)
    }

    fn whole_wall0(doc: &Document) -> Selection {
        let mut sel = Selection::new();
        sel.box_select(
            doc.ds1(),
            capy_edit::CellRect::new((0, 0), (5, 5)),
            &[LayerRef::Wall(0)],
            false,
            capy_edit::SelectOp::Replace,
        );
        sel
    }

    #[test]
    fn a_plain_move_needs_no_upgrade() {
        let mut doc = doc_with_walls(18);
        let sel = whole_wall0(&doc);
        let op = EditOp::MoveToLayer {
            from: LayerRef::Wall(0),
            to: LayerRef::Wall(2),
            mode: MoveMode::Overwrite,
        };
        let (result, steps) = run_upgrading(&mut doc, &sel, &op);
        assert!(steps.is_empty());
        let OpResult::Done(OpOutcome::Moved(report)) = result else {
            panic!("expected a move");
        };
        assert_eq!(report.moved, 9);
        assert_eq!(report.layer_added, Some(LayerRef::Wall(2)));
        assert!(doc.is_dirty());
    }

    #[test]
    fn an_old_map_asks_for_a_version_and_the_retry_succeeds() {
        // A v3 file has a single wall layer: a second one needs an upgrade.
        let mut doc = doc_with_walls(3);
        let sel = whole_wall0(&doc);
        let op = EditOp::MoveToLayer {
            from: LayerRef::Wall(0),
            to: LayerRef::Wall(1),
            mode: MoveMode::SkipOccupied,
        };
        let OpResult::NeedsVersion { current, required } = run(&mut doc, &sel, &op) else {
            panic!("a v3 map cannot take a second wall layer");
        };
        assert_eq!(current, 3);
        assert!(required > 3);
        assert!(!doc.is_dirty(), "the failed attempt changed nothing");
        let result = upgrade_and_retry(&mut doc, &sel, &op, required);
        assert!(result.is_done(), "{result:?}");
        assert!(doc.ds1().version >= required);
        assert!(LayerRef::Wall(1).exists_in(doc.ds1()));
    }

    #[test]
    fn the_chain_follows_a_hint_that_was_too_low() {
        // A v3 map that carries a tag layer: raising it to the wall layer's
        // minimum (4) is not enough, the tag layer then asks for 10.
        let mut ds1 = Ds1::new(6, 6, 0);
        ds1.version = 3;
        let mut tag = Grid::new(6, 6);
        tag.set(1, 1, 7u32);
        ds1.tag = Some(tag);
        ds1.tag_type = 1;
        for y in 0..3 {
            for x in 0..3 {
                ds1.walls[0].tiles.set(x, y, Cell(1).with_indices(2, 1));
            }
        }
        let mut doc = Document::new(ds1);
        let sel = whole_wall0(&doc);
        let op = EditOp::MoveToLayer {
            from: LayerRef::Wall(0),
            to: LayerRef::Wall(1),
            mode: MoveMode::Overwrite,
        };
        let (result, steps) = run_upgrading(&mut doc, &sel, &op);
        assert!(result.is_done(), "{result:?} after {steps:?}");
        assert!(!steps.is_empty());
        assert!(
            steps.windows(2).all(|w| w[1] > w[0]),
            "every step asks for more: {steps:?}"
        );
        assert!(doc.ds1().version >= 4);
        assert!(doc.ds1().to_bytes().is_ok(), "the upgraded map is writable");
    }

    #[test]
    fn a_hopeless_request_stops_instead_of_looping() {
        let mut doc = doc_with_walls(18);
        // Wall 5 does not exist as a layer slot: this fails for another reason.
        let op = EditOp::AddLayer(LayerRef::Wall(9));
        let (result, steps) = run_upgrading(&mut doc, &Selection::new(), &op);
        assert!(matches!(result, OpResult::Failed(_)));
        assert!(steps.is_empty());
    }

    #[test]
    fn summaries_mention_what_happened() {
        let report = MoveReport {
            moved: 4,
            skipped_occupied: 2,
            layer_added: Some(LayerRef::Wall(2)),
        };
        let text = move_summary(LayerRef::Wall(0), LayerRef::Wall(2), &report);
        assert!(text.contains("Moved 4"), "{text}");
        assert!(text.contains("Wall 1") && text.contains("Wall 3"), "{text}");
        assert!(text.contains("2 skipped") && text.contains("added Wall 3"));
        let preview = PastePreview {
            writes: 5,
            out_of_bounds: 1,
            ..PastePreview::default()
        };
        let text = paste_summary(&preview);
        assert!(
            text.contains("5 cell") && text.contains("1 outside"),
            "{text}"
        );
    }
}
