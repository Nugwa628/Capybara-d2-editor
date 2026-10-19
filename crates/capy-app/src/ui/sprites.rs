//! The Sprites mode and the map editor's Sprite preview panel: placeholders
//! until P6 (viewer for DC6/DCC/COF, converter).

use bevy_egui::egui::{self, Ui};

use super::SpritesPanel;

pub fn show(ui: &mut Ui, panel: SpritesPanel) {
    match panel {
        SpritesPanel::Viewer => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Sprites mode coming soon: viewer + converter")
                        .size(18.0)
                        .weak(),
                );
            });
        }
        SpritesPanel::Converter => {
            ui.weak("DC6 / DCC import, export and batch conversion.");
        }
    }
}

/// The dockable preview inside the Map Editor.
pub fn preview_placeholder(ui: &mut Ui) {
    ui.weak("Sprite preview: coming with Sprites mode.");
}
