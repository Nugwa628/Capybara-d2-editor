//! Dialogs and floating panels: the questions (upgrade, unsaved changes,
//! mark as packed, unwritable map), the layer-move and resize windows, and
//! the paste preview's count panel.

use bevy_egui::egui::{self, Align2, Color32, Context, Id, RichText};
use capy_edit::{LayerRef, MoveMode};

use crate::doc::layer_label;
use crate::edit::ops::EditOp;
use crate::edit::paste::{PASTE_MODES, mode_label, summary_rows};
use crate::edit::prompt::CloseChoice;
use crate::edit::state::{MoveDialog, ResizeDialog};
use crate::state::AppState;

pub fn show_all(ctx: &Context, state: &mut AppState) {
    upgrade(ctx, state);
    save_problem(ctx, state);
    close_prompt(ctx, state);
    packed(ctx, state);
    move_window(ctx, state);
    resize_window(ctx, state);
    paste_panel(ctx, state);
}

// ───────────────────────── questions ─────────────────────────

fn upgrade(ctx: &Context, state: &mut AppState) {
    let Some(pending) = state.edit.upgrade.clone() else {
        return;
    };
    let mut accept = false;
    let mut cancel = false;
    let response = egui::Modal::new(Id::new("upgrade_modal")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading("This map needs a newer DS1 version");
        ui.add_space(6.0);
        ui.label(format!(
            "{} needs DS1 version {}, and this map is version {}.",
            pending.op.name(),
            pending.required,
            pending.current
        ));
        ui.weak(
            "Upgrading rewrites the map in the newer layout the next time you save. \
             The change can be undone with Ctrl+Z.",
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!(
                    "Upgrade this map to v{} and continue",
                    pending.required
                ))
                .clicked()
            {
                accept = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if accept {
        state.accept_upgrade();
    } else if cancel || response.should_close() {
        state.cancel_upgrade();
    }
}

fn save_problem(ctx: &Context, state: &mut AppState) {
    let Some(problem) = state.edit.save_problem.clone() else {
        return;
    };
    let version = state
        .doc_index(problem.doc)
        .map(|i| state.docs[i].ds1.version);
    let mut accept = false;
    let mut cancel = false;
    let response = egui::Modal::new(Id::new("save_problem_modal")).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading("This map cannot be saved in its DS1 version");
        ui.add_space(6.0);
        if let Some(v) = version {
            ui.label(format!("The map is version {v}."));
        }
        ui.label(format!("Why: {}", problem.detail));
        ui.weak(format!(
            "Version {} can store everything the map holds now.",
            problem.min_version
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("Upgrade to v{} and save", problem.min_version))
                .clicked()
            {
                accept = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if accept {
        state.accept_save_upgrade();
    } else if cancel || response.should_close() {
        state.edit.save_problem = None;
    }
}

fn close_prompt(ctx: &Context, state: &mut AppState) {
    let Some(prompt) = state.edit.close_prompt.clone() else {
        return;
    };
    let mut choice: Option<CloseChoice> = None;
    let response = egui::Modal::new(Id::new("close_modal")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading(prompt.title());
        ui.add_space(6.0);
        for d in &prompt.dirty {
            ui.label(format!("\u{25CF} {}", d.name));
        }
        ui.weak("Your changes are lost if you discard them.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                choice = Some(CloseChoice::Save);
            }
            if ui.button("Discard").clicked() {
                choice = Some(CloseChoice::Discard);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(CloseChoice::Cancel);
            }
        });
    });
    if choice.is_none() && response.should_close() {
        choice = Some(CloseChoice::Cancel);
    }
    if let Some(choice) = choice {
        state.answer_close(choice);
    }
}

fn packed(ctx: &Context, state: &mut AppState) {
    if !state.edit.confirm_packed {
        return;
    }
    let count = state.edit.changed;
    let mut accept = false;
    let mut cancel = false;
    let response = egui::Modal::new(Id::new("packed_modal")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.heading("Mark the saved maps as packed?");
        ui.add_space(6.0);
        ui.label(format!(
            "CHANGED.txt lists {count} file(s). Marking them as packed empties the list; \
             the saved .ds1 files stay where they are."
        ));
        ui.weak("Do this after you have packed the files into your MPQ.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Mark as packed").clicked() {
                accept = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if accept {
        state.confirm_mark_packed();
    } else if cancel || response.should_close() {
        state.edit.confirm_packed = false;
    }
}

// ───────────────────────── windows ─────────────────────────

/// Layers a selection can be moved to: every floor, wall and the shadow.
pub fn move_targets() -> Vec<LayerRef> {
    let mut v: Vec<LayerRef> = (0..2).map(LayerRef::Floor).collect();
    v.extend((0..4).map(LayerRef::Wall));
    v.push(LayerRef::Shadow);
    v
}

/// The initial state of the move dialog for the active map's selection.
pub fn new_move_dialog(state: &AppState) -> Option<MoveDialog> {
    let doc = state.active_doc()?;
    let from = doc
        .selection
        .layers()
        .into_iter()
        .find(|l| *l != LayerRef::Tag)
        .unwrap_or(doc.active_layer);
    let to = match from {
        LayerRef::Wall(i) if i < 3 => LayerRef::Wall(i + 1),
        LayerRef::Wall(_) => LayerRef::Wall(0),
        LayerRef::Floor(0) => LayerRef::Floor(1),
        LayerRef::Floor(_) => LayerRef::Floor(0),
        _ => LayerRef::Wall(0),
    };
    Some(MoveDialog {
        from,
        to,
        mode: MoveMode::SkipOccupied,
    })
}

fn layer_combo(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut LayerRef,
    options: &[LayerRef],
    exists: &dyn Fn(LayerRef) -> bool,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(layer_label(*value))
        .show_ui(ui, |ui| {
            for &l in options {
                let text = if exists(l) {
                    layer_label(l)
                } else {
                    format!("{} (new)", layer_label(l))
                };
                ui.selectable_value(value, l, text);
            }
        });
}

fn move_window(ctx: &Context, state: &mut AppState) {
    let Some(mut dialog) = state.edit.move_dialog else {
        return;
    };
    let Some(index) = state.active else {
        state.edit.move_dialog = None;
        return;
    };
    let doc = &state.docs[index];
    let existing = LayerRef::all_in(&doc.ds1);
    let selected_layers: Vec<LayerRef> = doc
        .selection
        .layers()
        .into_iter()
        .filter(|l| *l != LayerRef::Tag)
        .collect();
    let in_from = doc
        .selection
        .iter()
        .filter(|(l, _, _)| *l == dialog.from)
        .count();
    let sources: Vec<LayerRef> = if selected_layers.is_empty() {
        existing
            .iter()
            .copied()
            .filter(|l| *l != LayerRef::Tag)
            .collect()
    } else {
        selected_layers
    };
    let targets = move_targets();
    let mut open = true;
    let mut close = false;
    let mut go = false;
    egui::Window::new("Move selection to layer")
        .id(Id::new("move_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Grid::new("move_grid").num_columns(2).show(ui, |ui| {
                ui.label("From");
                layer_combo(ui, "move_from", &mut dialog.from, &sources, &|l| {
                    existing.contains(&l)
                });
                ui.end_row();
                ui.label("To");
                layer_combo(ui, "move_to", &mut dialog.to, &targets, &|l| {
                    existing.contains(&l)
                });
                ui.end_row();
                ui.label("Where the target is occupied");
                ui.vertical(|ui| {
                    ui.radio_value(&mut dialog.mode, MoveMode::SkipOccupied, "Skip those cells");
                    ui.radio_value(&mut dialog.mode, MoveMode::Overwrite, "Overwrite them");
                });
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(format!(
                "{in_from} selected cell(s) on {}",
                layer_label(dialog.from)
            ));
            if !existing.contains(&dialog.to) {
                ui.weak(format!(
                    "{} does not exist yet and will be added.",
                    layer_label(dialog.to)
                ));
            }
            if dialog.from == dialog.to {
                ui.colored_label(
                    Color32::from_rgb(230, 170, 90),
                    "Pick a different target layer.",
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ready = dialog.from != dialog.to && in_from > 0;
                if ui.add_enabled(ready, egui::Button::new("Move")).clicked() {
                    go = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    state.edit.move_dialog = Some(dialog);
    if go {
        let ok = state.exec(
            index,
            EditOp::MoveToLayer {
                from: dialog.from,
                to: dialog.to,
                mode: dialog.mode,
            },
        );
        if ok {
            close = true;
        }
    }
    if !open || close {
        state.edit.move_dialog = None;
    }
}

fn resize_window(ctx: &Context, state: &mut AppState) {
    let Some(mut dialog) = state.edit.resize_dialog else {
        return;
    };
    let Some(index) = state.active else {
        state.edit.resize_dialog = None;
        return;
    };
    let (cur_w, cur_h) = (state.docs[index].ds1.width, state.docs[index].ds1.height);
    let mut open = true;
    let mut close = false;
    let mut go = false;
    egui::Window::new("Resize map")
        .id(Id::new("resize_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!("Now {cur_w} x {cur_h} cells."));
            egui::Grid::new("resize_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut dialog.width).range(1..=1024));
                    ui.end_row();
                    ui.label("Height");
                    ui.add(egui::DragValue::new(&mut dialog.height).range(1..=1024));
                    ui.end_row();
                });
            ui.weak("The top-left corner stays; cells beyond the new edge are cut off.");
            if dialog.width < cur_w || dialog.height < cur_h {
                ui.colored_label(
                    Color32::from_rgb(230, 170, 90),
                    "Shrinking drops tiles (Ctrl+Z brings them back).",
                );
            }
            ui.horizontal(|ui| {
                let changed = (dialog.width, dialog.height) != (cur_w, cur_h);
                if ui
                    .add_enabled(changed, egui::Button::new("Resize"))
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    state.edit.resize_dialog = Some(dialog);
    if go {
        let ok = state.exec(
            index,
            EditOp::Resize {
                width: dialog.width,
                height: dialog.height,
            },
        );
        if ok {
            close = true;
        }
    }
    if !open || close {
        state.edit.resize_dialog = None;
    }
}

pub fn new_resize_dialog(state: &AppState) -> Option<ResizeDialog> {
    let doc = state.active_doc()?;
    Some(ResizeDialog {
        width: doc.ds1.width,
        height: doc.ds1.height,
    })
}

// ───────────────────────── paste panel ─────────────────────────

fn paste_panel(ctx: &Context, state: &mut AppState) {
    let Some(preview) = state.paste_preview() else {
        return;
    };
    let Some(rect) = state.map_rect else { return };
    let Some(mode) = state.edit.paste.as_ref().map(|s| s.mode) else {
        return;
    };
    let from_os = state.edit.paste.as_ref().is_some_and(|s| s.from_os);
    let mut new_mode = mode;
    let mut cancel = false;
    let mut commit = false;
    egui::Area::new(Id::new("paste_panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.right_top() + egui::vec2(-272.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(250.0);
                ui.label(RichText::new("Paste preview").strong());
                if from_os {
                    ui.weak("from the system clipboard");
                }
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(60, 220, 90), "\u{25A0} writes");
                    ui.colored_label(Color32::from_rgb(235, 60, 60), "\u{25A0} conflicts");
                });
                ui.separator();
                egui::Grid::new("paste_counts")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (label, text) in summary_rows(&preview) {
                            let warn = label == "Warning"
                                || (label == "Conflicts" && !preview.conflicts.is_empty())
                                || (label == "Outside the map" && preview.out_of_bounds > 0);
                            ui.label(label);
                            if warn {
                                ui.colored_label(Color32::from_rgb(235, 170, 80), text);
                            } else {
                                ui.label(text);
                            }
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.label("When the target is occupied");
                for m in PASTE_MODES {
                    ui.radio_value(&mut new_mode, m, mode_label(m));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Paste here").clicked() {
                        commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                ui.weak("Click the map to paste, Esc to cancel");
            });
        });
    if new_mode != mode {
        state.set_paste_mode(new_mode);
    }
    if commit {
        state.commit_paste();
    } else if cancel {
        state.cancel_paste();
    }
}
