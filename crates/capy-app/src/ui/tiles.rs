//! Tiles panel: a placeholder grid until the tile browser (P3); it already
//! shows how many assets the background index holds.

use bevy_egui::egui::{self, Color32, Ui};

use crate::index_task::IndexState;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    match &state.index.state {
        IndexState::Ready(s) => {
            ui.label(format!("{} assets indexed", s.assets));
        }
        IndexState::Building(_) | IndexState::Checking => {
            ui.weak("Indexing assets in the background...");
        }
        _ => {}
    }
    ui.weak("The tile browser comes with P3.");
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
