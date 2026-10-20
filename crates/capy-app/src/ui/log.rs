//! Log panel.

use bevy_egui::egui::{self, Color32, RichText, Ui};

use crate::state::{AppState, LogLevel};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            state.log.clear();
        }
        ui.weak(format!("{} entries", state.log.len()));
    });
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(
            ui,
            ui.text_style_height(&egui::TextStyle::Monospace),
            state.log.len(),
            |ui, range| {
                for entry in &state.log[range] {
                    let (tag, color) = match entry.level {
                        LogLevel::Info => ("INFO ", state.shell.shades.muted),
                        LogLevel::Warn => ("WARN ", Color32::from_rgb(220, 180, 60)),
                        LogLevel::Error => ("ERROR", Color32::from_rgb(230, 90, 90)),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tag).monospace().color(color));
                        ui.label(RichText::new(&entry.text).monospace());
                    });
                }
            },
        );
}
