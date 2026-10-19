//! The tool strip above the map: tools, selection scope, brush size and the
//! value Paint will write.

use bevy_egui::egui::{self, Ui};

use super::thumbs;
use crate::doc::layer_label;
use crate::edit::tools::{SelectScope, Tool};
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    if state.active_doc().is_none() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for tool in Tool::ALL {
            let selected = state.edit.tool == tool;
            let label = egui::RichText::new(tool.glyph());
            let button = egui::Button::new(label)
                .selected(selected)
                .min_size(egui::vec2(28.0, 22.0));
            if ui
                .add(button)
                .on_hover_text(format!("{} ({})", tool.label(), tool.shortcut()))
                .clicked()
            {
                super::map_tools::cancel_gesture(state);
                state.edit.tool = tool;
            }
        }
        ui.separator();

        let tool = state.edit.tool;
        if tool.selects() {
            ui.label("On");
            egui::ComboBox::from_id_salt("select_scope")
                .selected_text(state.edit.scope.label())
                .width(130.0)
                .show_ui(ui, |ui| {
                    for scope in [SelectScope::Active, SelectScope::Visible] {
                        ui.selectable_value(&mut state.edit.scope, scope, scope.label());
                    }
                });
            ui.weak(crate::edit::tools::SELECT_MODS_HINT);
        }
        if tool.brushes() {
            ui.label("Brush");
            ui.add(
                egui::DragValue::new(&mut state.edit.brush)
                    .range(1..=16)
                    .suffix(" cells"),
            );
        }
        if tool == Tool::Paint {
            ui.label("Value");
            let current = state.edit.current;
            let layer = state
                .edit
                .current_layer
                .or_else(|| state.active_doc().map(|d| d.active_layer));
            match layer {
                Some(layer) if !layer.is_empty_value(current) => {
                    thumbs::show(ui, state, layer, current, true).on_hover_text(format!(
                        "{} cell {:#010x}, orientation {}",
                        layer_label(layer),
                        current.cell,
                        current.orientation
                    ));
                }
                _ => {
                    ui.weak("none: pick with the Eyedropper (I)");
                }
            }
        }
        if tool == Tool::Objects {
            ui.weak("Click an object, drag to move, Delete removes");
        }
        if let Some(doc) = state.active_doc() {
            ui.separator();
            let n = doc.selection.len();
            ui.label(format!("{n} cell(s) selected"));
            if !doc.object_sel.is_empty() {
                ui.label(format!("{} object(s)", doc.object_sel.len()));
            }
        }
    });
}
