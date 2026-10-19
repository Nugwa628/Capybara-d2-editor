//! The Objects panel: the selected object's properties and NPC path, and
//! the "Add object" picker (searchable list with an animated preview).

use bevy_egui::egui::{self, Color32, Pos2, Rect, Sense, Ui, Vec2};
use capy_render::frame_index;
use d2_assets::objects::ObjectInfo;

use super::sprites::{paint_checker, rgba_texture};
use crate::edit::ops::EditOp;
use crate::edit::tools::Tool;
use crate::objects::path_tool::{ACTIONS, action_label};
use crate::objects::{ObjectMode, PickerPreview, PropsEdit, picker};
use crate::sprites::viewer::Tex;
use crate::state::AppState;

const ERROR: Color32 = Color32::from_rgb(235, 90, 90);

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let Some(index) = state.active else {
        ui.weak("Open a map to place and edit objects.");
        return;
    };
    mode_row(ui, state);
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("objects_panel")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // While placing, the picker comes first.
            if state.edit.tool == Tool::Objects && state.objects.mode == ObjectMode::Place {
                add_picker(ui, state, index);
                ui.separator();
                properties(ui, state, index);
            } else {
                properties(ui, state, index);
                ui.separator();
                add_picker(ui, state, index);
            }
        });
}

/// Select / Add / Path, the three things the Objects tool's click does.
fn mode_row(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        for (mode, label, hint) in [
            (
                ObjectMode::Select,
                "Select",
                "Click an object (sprite or marker), drag to move, Delete removes",
            ),
            (
                ObjectMode::Place,
                "Add",
                "Click on the map to place the object chosen below",
            ),
            (
                ObjectMode::Path,
                "Path",
                "NPC path of the selected object: click adds a point, drag moves one, Delete removes",
            ),
        ] {
            let on = state.edit.tool == Tool::Objects && state.objects.mode == mode;
            if ui.selectable_label(on, label).on_hover_text(hint).clicked() {
                set_mode(state, mode);
            }
        }
    });
}

/// Switches the Objects tool to `mode` (and to the Objects tool).
pub fn set_mode(state: &mut AppState, mode: ObjectMode) {
    super::map_tools::cancel_gesture(state);
    state.edit.tool = Tool::Objects;
    state.objects.mode = mode;
    state.objects.path.drag = None;
}

fn properties(ui: &mut Ui, state: &mut AppState, index: usize) {
    let doc = &state.docs[index];
    let selected = doc.object_sel.clone();
    let object = match selected.as_slice() {
        [] => {
            ui.weak("No object selected. Use the Objects tool (O) and click one.");
            return;
        }
        [one] => match doc.ds1.objects.get(*one) {
            Some(o) => (*one, *o),
            None => return,
        },
        many => {
            ui.label(format!("{} objects selected", many.len()));
            return;
        }
    };
    let (object_index, current) = object;
    let doc_id = doc.id;
    let (act, version) = (doc.ds1.effective_act(), doc.ds1.version);
    let missing = doc
        .render
        .missing_objects
        .iter()
        .find(|(i, _)| *i == object_index)
        .map(|(_, why)| why.clone());
    let info: Option<ObjectInfo> = state
        .object_catalog()
        .map(|c| c.resolve(act, version, &current));

    // The edit buffer follows the selection and the document.
    let fresh = state.objects.props.is_none_or(|p| {
        p.doc != doc_id || p.index != object_index || p.original != current
    });
    if fresh {
        state.objects.props = Some(PropsEdit {
            doc: doc_id,
            index: object_index,
            original: current,
            edited: current,
        });
    }
    let Some(mut edit) = state.objects.props else {
        return;
    };

    ui.strong(match &info {
        Some(i) => format!("{}  (object {object_index})", i.name),
        None => format!("Object {object_index}"),
    });
    if let Some(i) = &info {
        ui.weak(format!(
            "token {}  mode {}  class {}  order flag {}  ({})",
            i.token.as_deref().unwrap_or("-"),
            i.mode,
            i.weapon_class,
            i.order_flag,
            i.source
        ));
    } else {
        ui.weak("Loading the object tables...");
    }
    match &missing {
        Some(why) => {
            ui.colored_label(ERROR, format!("Draws nothing: {why}"));
        }
        None if info.is_some() => {
            ui.weak("Drawn as a sprite on the map");
        }
        None => {}
    }
    egui::Grid::new("object_props")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Type");
            ui.add(egui::DragValue::new(&mut edit.edited.kind).range(0..=255))
                .on_hover_text("1 = monster / NPC (MonPreset), 2 = object (act x 150 table)");
            ui.end_row();
            ui.label("Id");
            ui.add(egui::DragValue::new(&mut edit.edited.id).range(0..=1023));
            ui.end_row();
            ui.label("Flags");
            ui.add(
                egui::DragValue::new(&mut edit.edited.flags)
                    .hexadecimal(8, false, true)
                    .prefix("0x"),
            )
            .on_hover_text("Stored from DS1 version 6");
            ui.end_row();
            ui.label("Sub-tile");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut edit.edited.x).range(0..=i32::MAX));
                ui.add(egui::DragValue::new(&mut edit.edited.y).range(0..=i32::MAX));
            });
            ui.end_row();
        });
    let changed = edit.edited != edit.original;
    let mut apply = false;
    ui.horizontal(|ui| {
        apply = ui
            .add_enabled(changed, egui::Button::new("Apply"))
            .on_hover_text("One undo step (set_object)")
            .clicked();
        if ui
            .add_enabled(changed, egui::Button::new("Revert"))
            .clicked()
        {
            edit.edited = edit.original;
        }
    });
    state.objects.props = Some(edit);
    if apply {
        state.exec(
            index,
            EditOp::SetObject {
                index: object_index,
                object: edit.edited,
            },
        );
    }
    ui.separator();
    path_section(ui, state, index, object_index);
}

fn path_section(ui: &mut Ui, state: &mut AppState, index: usize, object: usize) {
    let doc = &state.docs[index];
    let version = doc.ds1.version;
    let points = doc
        .edit
        .path_of(object)
        .map(|p| p.points.clone())
        .unwrap_or_default();
    state.objects.path.follow_object(Some(object));
    state.objects.path.clamp(points.len());
    ui.horizontal(|ui| {
        ui.strong("NPC path");
        ui.weak(match points.len() {
            0 => "none".to_owned(),
            n => format!("{n} point(s)"),
        });
    });
    if version < 14 {
        ui.weak(format!(
            "DS1 v{version} stores no NPC paths: adding a point offers an upgrade to v14"
        ));
    }
    ui.horizontal(|ui| {
        ui.label("New points");
        action_combo(ui, "new_point_action", &mut state.objects.path.action);
        if ui
            .selectable_label(
                state.edit.tool == Tool::Objects && state.objects.mode == ObjectMode::Path,
                "Edit path on the map",
            )
            .clicked()
        {
            set_mode(state, ObjectMode::Path);
        }
    });
    let mut op: Option<EditOp> = None;
    egui::Grid::new("path_points")
        .num_columns(5)
        .striped(true)
        .show(ui, |ui| {
            for (i, p) in points.iter().enumerate() {
                let selected = state.objects.path.selected == Some(i);
                if ui.selectable_label(selected, format!("{}", i + 1)).clicked() {
                    state.objects.path.selected = Some(i);
                }
                ui.monospace(format!("{:>4} {:>4}", p.x, p.y));
                let mut action = p.action;
                action_combo(ui, ("point_action", i), &mut action);
                if action != p.action {
                    op = Some(EditOp::SetPathAction {
                        object,
                        index: i,
                        action,
                    });
                }
                if ui.small_button("x").on_hover_text("Delete this point").clicked() {
                    op = Some(EditOp::DeletePathPoints {
                        object,
                        indices: vec![i],
                    });
                }
                ui.end_row();
            }
        });
    if let Some(op) = op {
        state.exec(index, op);
    }
}

fn action_combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, action: &mut i32) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(action_label(*action))
        .width(90.0)
        .show_ui(ui, |ui| {
            for (a, label) in ACTIONS {
                ui.selectable_value(action, a, label);
            }
        })
        .response
        .on_hover_text("Path point action: v15+ stores it; older maps read 1");
}

fn add_picker(ui: &mut Ui, state: &mut AppState, index: usize) {
    let act = state.docs[index].ds1.effective_act();
    ui.strong(format!("Add object (act {})", act + 1));
    let Some(list) = state.placeables(act) else {
        ui.weak("Loading the object tables...");
        ui.ctx().request_repaint();
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(
            egui::TextEdit::singleline(&mut state.objects.picker.query)
                .hint_text("name, token, npc, type2, id 37")
                .desired_width(180.0),
        );
    });
    let hits = picker::filter(&list, &state.objects.picker.query);
    ui.weak(format!("{} of {} placeable", hits.len(), list.len()));
    let chosen = state.objects.picker.chosen;
    let mut pick = None;
    egui::ScrollArea::vertical()
        .id_salt("object_picker_list")
        .max_height(220.0)
        .auto_shrink([false, true])
        .show_rows(ui, 18.0, hits.len(), |ui, rows| {
            for &i in &hits[rows] {
                let info = &list[i];
                let key = (info.ds1_kind, info.ds1_id);
                let resp = ui.selectable_label(chosen == Some(key), picker::row_label(info));
                let resp = match &info.token {
                    Some(t) => resp.on_hover_text(format!("token {t}, mode {}", info.mode)),
                    None => resp.on_hover_text("no sprite: shown as a marker"),
                };
                if resp.clicked() {
                    pick = Some(key);
                }
            }
        });
    if let Some(key) = pick {
        state.objects.picker.chosen = Some(key);
        set_mode(state, ObjectMode::Place);
    }
    let Some((kind, id)) = state.objects.picker.chosen else {
        ui.weak("Pick an entry, then click on the map to place it.");
        return;
    };
    let Some(info) = list
        .iter()
        .find(|i| (i.ds1_kind, i.ds1_id) == (kind, id))
        .cloned()
    else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(format!("Placing: {}", picker::row_label(&info)));
    });
    if state.objects.mode == ObjectMode::Place && state.edit.tool == Tool::Objects {
        ui.weak("Click on the map: the object goes on the sub-tile under the cursor.");
    }
    preview(ui, state, &info, act);
}

/// The chosen entry, animated.
fn preview(ui: &mut Ui, state: &mut AppState, info: &ObjectInfo, act: u8) {
    let epoch = state.session_epoch;
    let gamma = state.config.gamma;
    let key = (epoch, info.ds1_kind, info.ds1_id, act, gamma.to_bits());
    if state.objects.preview.as_ref().is_none_or(|p| p.key != key) {
        let (Some(session), Some(catalog)) = (state.session.as_ref(), state.object_catalog())
        else {
            return;
        };
        let overlay = session.overlay.clone();
        let palette = state
            .sprites
            .palette
            .act(act, Some(&overlay), epoch)
            .with_gamma(gamma);
        let direction = info.direction.unwrap_or(0);
        let built = crate::sprites::preview::build_frames(&overlay, &catalog, info, direction, &palette);
        state.objects.preview = Some(match built {
            Ok((_, frames, delta)) => PickerPreview {
                key,
                frames: frames
                    .into_iter()
                    .enumerate()
                    .map(|(f, raw)| {
                        let (w, h, rgba, offset) = raw?;
                        rgba_texture(ui.ctx(), &format!("picker-{f}"), w, h, &rgba).map(
                            |handle| Tex {
                                handle,
                                offset,
                                size: (w, h),
                            },
                        )
                    })
                    .collect(),
                delta: delta.filter(|d| *d != 0).unwrap_or(256),
                error: None,
            },
            Err(e) => PickerPreview {
                key,
                frames: Vec::new(),
                delta: 256,
                error: Some(e),
            },
        });
    }
    let Some(p) = state.objects.preview.as_ref() else {
        return;
    };
    if let Some(e) = &p.error {
        ui.weak(format!("No preview: {e}"));
        return;
    }
    let count = p.frames.len() as u32;
    let tick = crate::objects::tick_at(ui.input(|i| i.time));
    let frame = usize::from(frame_index(Some(tick), p.delta, count));
    if count > 1 {
        ui.ctx().request_repaint();
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(140.0, 140.0), Sense::hover());
    let painter = ui.painter_at(rect);
    paint_checker(&painter, rect, 6.0);
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for t in p.frames.iter().flatten() {
        x0 = x0.min(t.offset.0);
        y0 = y0.min(t.offset.1);
        x1 = x1.max(t.offset.0 + t.size.0 as i32);
        y1 = y1.max(t.offset.1 + t.size.1 as i32);
    }
    if x0 > x1 {
        return;
    }
    let (bw, bh) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
    let z = ((rect.width() - 8.0) / bw)
        .min((rect.height() - 8.0) / bh)
        .clamp(0.2, 2.0);
    let centre = Vec2::new((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
    let anchor = rect.center() - centre * z;
    if let Some(Some(t)) = p.frames.get(frame) {
        let min = anchor + Vec2::new(t.offset.0 as f32, t.offset.1 as f32) * z;
        let r = Rect::from_min_size(min, Vec2::new(t.size.0 as f32, t.size.1 as f32) * z);
        painter.image(
            t.handle.id(),
            r,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}
