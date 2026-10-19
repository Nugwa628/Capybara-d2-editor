//! Tiles panel: a placeholder grid until the DT1 renderer lands.

use bevy_egui::egui::{self, Color32, Ui};

use crate::state::AppState;

pub fn show(ui: &mut Ui, _state: &mut AppState) {
    ui.weak("Tile browser comes with the textured renderer.");
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for i in 0..48u32 {
                let shade = 60 + (i * 37 % 90) as u8;
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 3.0, Color32::from_gray(shade));
                resp.on_hover_text(format!("placeholder tile {i}"));
            }
        });
    });
}
