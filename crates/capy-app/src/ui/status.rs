//! Bottom status bar: background progress, hovered cell, zoom.

use bevy_egui::egui::{self, Ui};

use crate::index_task::{IndexState, phase_label};
use crate::iso::ZOOM_STEPS;
use crate::state::{AppState, CatalogState};

/// Longest "current item" text shown before it is cut (paths get long).
const MAX_CURRENT_CHARS: usize = 48;

fn shorten(text: &str) -> String {
    let count = text.chars().count();
    if count <= MAX_CURRENT_CHARS {
        return text.to_owned();
    }
    let tail: String = text.chars().skip(count - (MAX_CURRENT_CHARS - 1)).collect();
    format!("\u{2026}{tail}")
}

/// The one-line text of an index state, or `None` when there is nothing to say.
pub fn index_text(state: &IndexState) -> Option<String> {
    match state {
        IndexState::Idle => None,
        IndexState::Checking => Some("Index: checking...".to_owned()),
        IndexState::Building(p) => Some(format!(
            "Index: {} {}/{} {}",
            phase_label(p.phase),
            p.done,
            p.total,
            shorten(&p.current)
        )),
        IndexState::Ready(s) if s.rebuilt => Some(format!("Index built: {} assets", s.assets)),
        IndexState::Ready(s) => Some(format!("Index up to date: {} assets", s.assets)),
        IndexState::Cancelled => Some("Index build cancelled".to_owned()),
        IndexState::Failed(_) => Some("Index failed (see the log)".to_owned()),
    }
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let mut cancel = false;
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
            if matches!(state.catalog, CatalogState::Building) {
                ui.separator();
                ui.spinner();
                ui.label("Map catalog...");
            }
        } else {
            ui.label("No profile open");
        }

        if state.index.state.is_running() {
            ui.separator();
            let bar = match state.index.state.fraction() {
                Some(f) => egui::ProgressBar::new(f).show_percentage(),
                None => egui::ProgressBar::new(0.0).animate(true),
            };
            ui.add(bar.desired_width(120.0));
        } else if !state.index.state.is_running() && state.index.state != IndexState::Idle {
            ui.separator();
        }
        if let Some(text) = index_text(&state.index.state) {
            let label = ui.label(text);
            if let IndexState::Failed(e) = &state.index.state {
                label.on_hover_text(e.clone());
            }
        }
        if state.index.state.is_running() {
            let cancelling = state.index.cancelling();
            if ui
                .add_enabled(!cancelling, egui::Button::new("Cancel"))
                .on_hover_text("Stop building the asset index")
                .clicked()
            {
                cancel = true;
            }
            if cancelling {
                ui.label("cancelling...");
            }
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
            if let Some(at) = &state.history.last_autosave {
                ui.separator();
                ui.weak(format!("autosaved {at}")).on_hover_text(
                    "The last autosave of maps with unsaved changes (History panel)",
                );
            }
        });
    });
    if cancel {
        state.cancel_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_task::IndexSummary;
    use capy_index::{BuildPhase, BuildProgress};
    use std::time::Duration;

    #[test]
    fn index_text_covers_every_state() {
        assert_eq!(index_text(&IndexState::Idle), None);
        assert!(index_text(&IndexState::Checking).is_some());
        let building = IndexState::Building(BuildProgress {
            phase: BuildPhase::Tiles,
            done: 3,
            total: 9,
            current: "data\\global\\tiles\\act1\\town\\floor.dt1".into(),
        });
        let text = index_text(&building).expect("text");
        assert!(text.contains("Indexing tiles 3/9"), "{text}");
        let ready = |rebuilt| {
            IndexState::Ready(IndexSummary {
                assets: 42,
                rebuilt,
                warnings: 0,
                elapsed: Duration::ZERO,
            })
        };
        assert_eq!(
            index_text(&ready(false)).as_deref(),
            Some("Index up to date: 42 assets")
        );
        assert_eq!(
            index_text(&ready(true)).as_deref(),
            Some("Index built: 42 assets")
        );
        assert!(index_text(&IndexState::Cancelled).is_some());
    }

    #[test]
    fn long_current_items_keep_their_tail() {
        let long = format!("data\\{}\\file.dt1", "x".repeat(100));
        let s = shorten(&long);
        assert_eq!(s.chars().count(), MAX_CURRENT_CHARS);
        assert!(s.ends_with("file.dt1"));
        assert_eq!(shorten("short"), "short");
    }
}
