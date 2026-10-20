//! The right column's CELL and TAGS & GROUPS sections (ds1-studio
//! `panels.tsx`): the raw bytes of every layer of one cell with its flags,
//! the tag value, and the object groups as editable rectangles.

use bevy_egui::egui::{self, RichText, Ui};
use capy_edit::{CellValue, LayerRef};
use d2_formats::ds1::{Cell, Group};

use crate::doc::layer_label;
use crate::edit::ops::EditOp;
use crate::state::AppState;

/// Bit 31: drawn by the editor only (`Cell::hidden`).
pub const HIDDEN_BIT: u32 = 1 << 31;
/// Bit 17: the whole cell blocks walking (`Cell::unwalkable`).
pub const UNWALKABLE_BIT: u32 = 1 << 17;
/// Bit 16: the cell blocks jumping (and light).
pub const JUMP_BIT: u32 = 1 << 16;

/// The cell flags the section offers, with their bit and help text.
pub const FLAGS: [(&str, u32, &str); 3] = [
    (
        "Hidden",
        HIDDEN_BIT,
        "Bit 31: the game does not draw the tile (markers, blockers)",
    ),
    (
        "Unwalkable",
        UNWALKABLE_BIT,
        "Bit 17: blocks all 25 sub-tiles of the cell",
    ),
    (
        "Block jump",
        JUMP_BIT,
        "Bit 16: adds the jump/light blocker",
    ),
];

/// Edit buffer of the groups list.
#[derive(Default)]
pub struct SectionsState {
    groups: Option<((u64, u64), Vec<Group>)>,
}

/// The cell the section shows: the hovered one, else the last clicked one.
pub fn shown_cell(state: &AppState) -> Option<(u32, u32)> {
    match state.hover_cell {
        Some((x, y)) if x >= 0 && y >= 0 => Some((x as u32, y as u32)),
        _ => state.studio.pinned_cell,
    }
}

/// The section header summary.
pub fn cell_summary(state: &AppState) -> String {
    shown_cell(state).map_or_else(String::new, |(x, y)| format!("{x}, {y}"))
}

fn set_cell(state: &mut AppState, index: usize, layer: LayerRef, x: u32, y: u32, v: CellValue) {
    state.exec(
        index,
        EditOp::Paint {
            layer,
            cells: vec![(x, y, v)],
        },
    );
}

pub fn cell_section(ui: &mut Ui, state: &mut AppState) {
    let Some(index) = state.active else {
        ui.weak("No map open");
        return;
    };
    let Some((x, y)) = shown_cell(state) else {
        ui.weak("Point at a cell on the map (a click keeps it here)");
        return;
    };
    let doc = &state.docs[index];
    if x >= doc.ds1.width || y >= doc.ds1.height {
        ui.weak("Point at a cell on the map");
        return;
    }
    let pinned = state.hover_cell.is_none();
    if pinned {
        ui.weak(format!("Cell {x}, {y} (last clicked)"));
    }
    let read_only = doc.snapshot_view.is_some();
    let layers: Vec<(LayerRef, CellValue)> = LayerRef::all_in(&doc.ds1)
        .into_iter()
        .filter_map(|l| l.get(&doc.ds1, x, y).map(|v| (l, v)))
        .collect();
    let mut change: Option<(LayerRef, CellValue)> = None;
    let mut any = false;
    for (layer, v) in layers {
        if layer == LayerRef::Tag {
            continue;
        }
        let empty = layer.is_empty_value(v);
        if empty {
            continue;
        }
        any = true;
        let c = Cell(v.cell);
        let [b0, b1, b2, b3] = v.cell.to_le_bytes();
        ui.horizontal(|ui| {
            ui.label(RichText::new(layer_label(layer)).strong());
            ui.label(
                RichText::new(format!(
                    "{}/{}{}",
                    c.main_index(),
                    c.sub_index(),
                    if matches!(layer, LayerRef::Wall(_)) {
                        format!(" o{}", v.orientation)
                    } else {
                        String::new()
                    }
                ))
                .monospace(),
            );
        });
        ui.label(
            RichText::new(format!(
                "{:#010x}  {b0:02x} {b1:02x} {b2:02x} {b3:02x}",
                v.cell
            ))
            .monospace()
            .size(11.5),
        )
        .on_hover_text("The cell dword and its bytes prop1..prop4 as stored");
        ui.horizontal_wrapped(|ui| {
            for (name, bit, help) in FLAGS {
                let mut on = v.cell & bit != 0;
                let resp = ui
                    .add_enabled(!read_only, egui::Checkbox::new(&mut on, name))
                    .on_hover_text(help);
                if resp.changed() {
                    let cell = if on { v.cell | bit } else { v.cell & !bit };
                    change = Some((layer, CellValue { cell, ..v }));
                }
            }
        });
        ui.add_space(4.0);
    }
    if !any {
        ui.weak("Empty cell");
    }
    if let Some((layer, v)) = change {
        set_cell(state, index, layer, x, y, v);
    }
}

/// TAGS & GROUPS: the tag value of the cell and the object groups.
pub fn tags_section(ui: &mut Ui, state: &mut AppState) {
    let Some(index) = state.active else {
        ui.weak("No map open");
        return;
    };
    let doc = &state.docs[index];
    let read_only = doc.snapshot_view.is_some();
    let has_tag = doc.ds1.tag.is_some();
    let cell = shown_cell(state);
    match (has_tag, cell) {
        (true, Some((x, y))) if x < doc.ds1.width && y < doc.ds1.height => {
            let value = doc.ds1.tag.as_ref().and_then(|t| t.get(x, y)).unwrap_or(0);
            let mut edit = value;
            ui.horizontal(|ui| {
                ui.label(format!("Tag at {x}, {y}"));
                let resp = ui.add_enabled(
                    !read_only,
                    egui::DragValue::new(&mut edit).hexadecimal(8, false, true),
                );
                if (resp.drag_stopped() || resp.lost_focus()) && edit != value {
                    set_cell(
                        state,
                        index,
                        LayerRef::Tag,
                        x,
                        y,
                        CellValue {
                            cell: edit,
                            orientation: 0,
                        },
                    );
                }
            });
        }
        (true, _) => {
            ui.weak("Tag layer: point at a cell to read it");
        }
        (false, _) => {
            ui.weak("No tag layer (Map > Add tag layer)");
        }
    }
    groups(ui, state, index, read_only);
}

fn groups(ui: &mut Ui, state: &mut AppState, index: usize, read_only: bool) {
    let doc = &state.docs[index];
    let key = (doc.id, doc.rev);
    let fresh = state
        .studio
        .sections
        .groups
        .as_ref()
        .is_some_and(|(k, _)| *k == key);
    if !fresh {
        state.studio.sections.groups = Some((key, doc.ds1.groups.clone()));
    }
    let (w, h) = (doc.ds1.width as i32, doc.ds1.height as i32);
    let version = doc.ds1.version;
    ui.add_space(4.0);
    ui.label(RichText::new(format!("Object groups (v{version})")).strong())
        .on_hover_text(
            "Rectangles in map cells that group objects (DS1 v12+, tag type 1 or 2). \
             Edits are undoable.",
        );
    let Some((_, buffer)) = state.studio.sections.groups.as_mut() else {
        return;
    };
    let mut commit: Option<usize> = None;
    let mut delete: Option<usize> = None;
    if buffer.is_empty() {
        ui.weak("No object groups");
    }
    egui::Grid::new("studio_groups")
        .num_columns(6)
        .spacing([4.0, 3.0])
        .show(ui, |ui| {
            if !buffer.is_empty() {
                for head in ["#", "x", "y", "w", "h", ""] {
                    ui.weak(head);
                }
                ui.end_row();
            }
            for (i, g) in buffer.iter_mut().enumerate() {
                ui.weak(i.to_string());
                let mut done = false;
                ui.add_enabled_ui(!read_only, |ui| {
                    for (v, lo) in [
                        (&mut g.x, 0),
                        (&mut g.y, 0),
                        (&mut g.width, 1),
                        (&mut g.height, 1),
                    ] {
                        let r = ui.add(egui::DragValue::new(v).range(lo..=w.max(h).max(1)));
                        done |= r.drag_stopped() || r.lost_focus();
                    }
                });
                if done {
                    commit = Some(i);
                }
                if ui
                    .add_enabled(!read_only, egui::Button::new("x").small())
                    .on_hover_text("Delete this group")
                    .clicked()
                {
                    delete = Some(i);
                }
                ui.end_row();
            }
        });
    let add = ui
        .add_enabled(!read_only, egui::Button::new("Add group"))
        .on_hover_text("A new 8 x 8 group at the top corner")
        .clicked();
    let changed = commit.and_then(|i| {
        let g = buffer.get(i).copied()?;
        let old = state.docs[index].ds1.groups.get(i).copied();
        (old != Some(g)).then_some((i, g))
    });
    if let Some((index_g, group)) = changed {
        state.exec(
            index,
            EditOp::SetGroup {
                index: index_g,
                group,
            },
        );
    }
    if let Some(i) = delete {
        state.exec(index, EditOp::DeleteGroup(i));
    }
    if add {
        state.exec(
            index,
            EditOp::AddGroup(Group {
                x: 0,
                y: 0,
                width: 8.min(w.max(1)),
                height: 8.min(h.max(1)),
                unknown: 0,
            }),
        );
    }
}
