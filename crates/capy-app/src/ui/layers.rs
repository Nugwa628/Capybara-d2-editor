//! Layers panel: visibility toggles for the active map.

use bevy_egui::egui::{self, Ui};

use crate::doc::LayerKind;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let Some(doc) = state.active_doc_mut() else {
        ui.weak("No map open");
        return;
    };
    ui.label(egui::RichText::new(doc.path.file_name()).strong());
    ui.separator();

    let mut changed: Vec<(crate::doc::LayerKey, bool)> = Vec::new();
    for key in doc.keys() {
        let mut visible = doc.is_visible(key);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut visible, key.label()).changed() {
                changed.push((key, visible));
            }
            let swatch = match key.kind() {
                LayerKind::Floor => egui::Color32::from_rgb(84, 128, 77),
                LayerKind::Wall => egui::Color32::from_rgb(158, 140, 120),
                LayerKind::Shadow => egui::Color32::from_rgb(30, 30, 30),
                LayerKind::Tag => egui::Color32::from_rgb(217, 64, 204),
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, swatch);
        });
    }
    for (key, visible) in changed {
        doc.set_visible(key, visible);
    }

    ui.checkbox(&mut doc.roofs_visible, "Roofs")
        .on_hover_text("Roof tiles (orientation 15)");
    ui.checkbox(&mut doc.special_visible, "Special tiles")
        .on_hover_text("Orientation 10 and 11 markers; hidden by default");

    ui.separator();
    ui.horizontal(|ui| {
        ui.checkbox(&mut doc.objects_visible, "Objects");
        ui.weak(format!("({})", doc.ds1.objects.len()));
    });
    ui.weak("Objects are not drawn yet.");
}
