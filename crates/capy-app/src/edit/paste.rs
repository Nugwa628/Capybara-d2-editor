//! The floating paste preview: where the clip lands, which cells are green
//! (they will be written) and which red (they collide with existing tiles),
//! and the lines of the count panel.

use std::collections::HashSet;

use capy_edit::{Clipboard, PasteMode, PastePreview};

/// How a clip cell shows in the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteTint {
    /// Lands on an empty target or changes nothing.
    Write,
    /// Lands on a different, occupied tile.
    Conflict,
}

/// A clip being placed. It follows the mouse while the pointer is over the
/// map and stays where it was otherwise.
#[derive(Clone, Debug)]
pub struct PasteSession {
    pub clip: Clipboard,
    /// The map cell under the clip's top-left corner.
    pub at: (i32, i32),
    pub mode: PasteMode,
    /// Where the clip came from, for the panel.
    pub from_os: bool,
}

impl PasteSession {
    pub fn new(clip: Clipboard, at: (i32, i32), mode: PasteMode, from_os: bool) -> PasteSession {
        PasteSession {
            clip,
            at,
            mode,
            from_os,
        }
    }
}

/// The cells to tint: every in-map cell of the clip, red when `preview` lists
/// a conflict there and green otherwise. Cells outside the map are left out
/// (the panel counts them). Each map cell appears once; a conflict on any of
/// its layers makes it red.
pub fn tint_cells(
    clip: &Clipboard,
    at: (i32, i32),
    preview: &PastePreview,
    width: u32,
    height: u32,
) -> Vec<((u32, u32), PasteTint)> {
    let conflicts: HashSet<(u32, u32)> = preview.conflicts.iter().map(|c| (c.x, c.y)).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &(_, cx, cy, _) in &clip.cells {
        let (Some(x), Some(y)) = (at.0.checked_add_unsigned(cx), at.1.checked_add_unsigned(cy))
        else {
            continue;
        };
        let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y)) else {
            continue;
        };
        if x >= width || y >= height || !seen.insert((x, y)) {
            continue;
        }
        let tint = if conflicts.contains(&(x, y)) {
            PasteTint::Conflict
        } else {
            PasteTint::Write
        };
        out.push(((x, y), tint));
    }
    out
}

/// The rows of the count panel: label and text, in a fixed order. Rows that
/// would show nothing interesting (no layers to add, no version needed) are
/// still present so the panel does not jump around; warnings follow as extra
/// rows.
pub fn summary_rows(preview: &PastePreview) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        ("Writes", preview.writes.to_string()),
        ("Conflicts", preview.conflicts.len().to_string()),
        ("Outside the map", preview.out_of_bounds.to_string()),
        (
            "Layers to add",
            if preview.layers_to_add.is_empty() {
                "none".to_owned()
            } else {
                preview
                    .layers_to_add
                    .iter()
                    .map(|l| crate::doc::layer_label(*l))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        ),
        (
            "Needs DS1 version",
            preview
                .needs_version
                .map_or("no".to_owned(), |v| format!("v{v}")),
        ),
    ];
    for w in &preview.warnings {
        rows.push(("Warning", w.clone()));
    }
    rows
}

pub fn mode_label(mode: PasteMode) -> &'static str {
    match mode {
        PasteMode::Overwrite => "Overwrite",
        PasteMode::SkipOccupied => "Skip occupied cells",
        PasteMode::OnlyIntoEmptyLayers => "Only into empty layers",
    }
}

pub const PASTE_MODES: [PasteMode; 3] = [
    PasteMode::Overwrite,
    PasteMode::SkipOccupied,
    PasteMode::OnlyIntoEmptyLayers,
];

#[cfg(test)]
mod tests {
    use super::*;
    use capy_edit::{
        CellRect, CellValue, ClipSource, Document, LayerRef, PasteConflict, SelectOp, Selection,
    };
    use d2_formats::ds1::{Cell, Ds1};

    fn clip_of(cells: &[(u32, u32)]) -> Clipboard {
        Clipboard {
            width: 4,
            height: 4,
            cells: cells
                .iter()
                .map(|&(x, y)| {
                    (
                        LayerRef::Floor(0),
                        x,
                        y,
                        CellValue {
                            cell: 1,
                            orientation: 0,
                        },
                    )
                })
                .collect(),
            objects: Vec::new(),
            source: ClipSource {
                game_path: None,
                act: 0,
                lvl_type: None,
                version: 18,
            },
        }
    }

    #[test]
    fn conflicts_are_red_and_the_rest_green() {
        // The clip is placed at (2, 2): clip cells (0,0) and (1,0) land on
        // map cells (2,2) and (3,2); (2,2) holds a different tile.
        let clip = clip_of(&[(0, 0), (1, 0), (0, 1)]);
        let preview = PastePreview {
            writes: 3,
            conflicts: vec![PasteConflict {
                layer: LayerRef::Floor(0),
                x: 2,
                y: 2,
                existing: CellValue::EMPTY,
                incoming: CellValue::EMPTY,
            }],
            ..PastePreview::default()
        };
        let tints = tint_cells(&clip, (2, 2), &preview, 8, 8);
        assert_eq!(
            tints,
            vec![
                ((2, 2), PasteTint::Conflict),
                ((3, 2), PasteTint::Write),
                ((2, 3), PasteTint::Write),
            ]
        );
    }

    #[test]
    fn cells_outside_the_map_are_not_tinted() {
        let clip = clip_of(&[(0, 0), (3, 0), (0, 3)]);
        let preview = PastePreview::default();
        // Top-left at (-1, 5) in a 6 x 6 map: (0,0) -> (-1,5) out, (3,0) -> (2,5)
        // in, (0,3) -> (-1,8) out.
        let tints = tint_cells(&clip, (-1, 5), &preview, 6, 6);
        assert_eq!(tints, vec![((2, 5), PasteTint::Write)]);
        // Far away: nothing, and no overflow.
        assert!(tint_cells(&clip, (i32::MAX, i32::MAX), &preview, 6, 6).is_empty());
        assert!(tint_cells(&clip, (i32::MIN, i32::MIN), &preview, 6, 6).is_empty());
    }

    #[test]
    fn a_conflict_on_one_layer_reddens_the_cell_once() {
        let mut clip = clip_of(&[(0, 0)]);
        clip.cells.push((
            LayerRef::Wall(0),
            0,
            0,
            CellValue {
                cell: 1,
                orientation: 3,
            },
        ));
        let preview = PastePreview {
            conflicts: vec![PasteConflict {
                layer: LayerRef::Wall(0),
                x: 1,
                y: 1,
                existing: CellValue::EMPTY,
                incoming: CellValue::EMPTY,
            }],
            ..PastePreview::default()
        };
        let tints = tint_cells(&clip, (1, 1), &preview, 4, 4);
        assert_eq!(tints, vec![((1, 1), PasteTint::Conflict)]);
    }

    #[test]
    fn tints_follow_a_real_preview() {
        let mut ds1 = Ds1::new(6, 6, 0);
        ds1.floors[0].set(3, 3, Cell(1).with_indices(5, 0));
        ds1.floors[0].set(0, 0, Cell(1).with_indices(1, 0));
        ds1.floors[0].set(1, 0, Cell(1).with_indices(2, 0));
        let doc = Document::new(ds1);
        let mut sel = Selection::new();
        sel.box_select(
            doc.ds1(),
            CellRect::new((0, 0), (1, 0)),
            &[LayerRef::Floor(0)],
            false,
            SelectOp::Replace,
        );
        let clip = doc.copy(&sel);
        let preview = doc.preview_paste(&clip, (3, 3), PasteMode::Overwrite);
        let tints = tint_cells(&clip, (3, 3), &preview, 6, 6);
        assert_eq!(
            tints,
            vec![((3, 3), PasteTint::Conflict), ((4, 3), PasteTint::Write)]
        );
        let rows = summary_rows(&preview);
        assert_eq!(rows[0], ("Writes", "2".to_owned()));
        assert_eq!(rows[1], ("Conflicts", "1".to_owned()));
    }

    #[test]
    fn the_panel_lists_every_count_and_warning() {
        let preview = PastePreview {
            writes: 12,
            conflicts: Vec::new(),
            out_of_bounds: 3,
            layers_to_add: vec![LayerRef::Wall(1), LayerRef::Wall(2)],
            needs_version: Some(10),
            warnings: vec!["different act".to_owned()],
        };
        let rows = summary_rows(&preview);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0], ("Writes", "12".to_owned()));
        assert_eq!(rows[2], ("Outside the map", "3".to_owned()));
        assert_eq!(rows[3], ("Layers to add", "Wall 2, Wall 3".to_owned()));
        assert_eq!(rows[4], ("Needs DS1 version", "v10".to_owned()));
        assert_eq!(rows[5], ("Warning", "different act".to_owned()));
        let quiet = summary_rows(&PastePreview::default());
        assert_eq!(quiet[3].1, "none");
        assert_eq!(quiet[4].1, "no");
        assert_eq!(PASTE_MODES.len(), 3);
        assert_eq!(mode_label(PasteMode::Overwrite), "Overwrite");
    }
}
