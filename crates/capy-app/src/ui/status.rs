//! Bottom status bar: background progress, hovered cell, zoom.

use bevy_egui::egui::{self, Ui};

use crate::iso::ZOOM_STEPS;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &AppState) {
    ui.horizontal(|ui| {
        if let Some(loading) = &state.loading {
            ui.spinner();
            ui.add(
                egui::ProgressBar::new(0.0)
                    .animate(true)
                    .desired_width(140.0)
                    .show_percentage(),
            );
            ui.label(format!("{}: {}...", loading.profile_id, loading.stage));
        } else if let Some(session) = &state.session {
            ui.label(format!(
                "{} \u{2022} {} maps \u{2022} {}",
                session.profile.name,
                session.maps.len(),
                session.summary
            ));
        } else {
            ui.label("No profile open");
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(doc) = state.active_doc() {
                let z = ZOOM_STEPS[doc.view.zoom.min(ZOOM_STEPS.len() - 1)];
                ui.label(format!("1:{z}"));
                ui.separator();
                match state.hover_cell {
                    Some((x, y)) => ui.label(format!("Cell ({x}, {y})")),
                    None => ui.label("Cell \u{2014}"),
                };
            }
        });
    });
}
