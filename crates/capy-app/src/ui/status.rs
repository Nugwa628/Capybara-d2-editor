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

/// What the current tool does, for the status bar.
fn tool_hint(state: &AppState) -> &'static str {
    use crate::edit::tools::Tool;
    use crate::objects::ObjectMode;
    if state.edit.paste.is_some() {
        return "Click or Enter places the paste; Esc drops it";
    }
    match state.edit.tool {
        Tool::SelectBox => {
            "Drag to select cells. Shift adds, Ctrl subtracts, Shift+Ctrl intersects; right-click for more"
        }
        Tool::Lasso => "Draw around cells to select them. Shift adds, Ctrl subtracts",
        Tool::SelectById => "Click a cell to select every cell with its tile ID",
        Tool::SelectIdentical => "Click a cell to select every identical cell",
        Tool::Paint => {
            "Click or drag to paint the value; pick one with Pick or from the Tiles column"
        }
        Tool::Erase => "Click or drag to erase cells of the active layer",
        Tool::Eyedropper => "Click a cell to make its tile the paint value",
        Tool::Objects => match state.objects.mode {
            ObjectMode::Select => "Click an object, drag to move it; Delete removes it",
            ObjectMode::Place => {
                "Click on the map to place the object chosen in the Objects column"
            }
            ObjectMode::Path => "Select an NPC, then click to add path points; drag moves one",
        },
    }
}

/// The Map Editor's status bar: tool hint on the left; background work,
/// tool and layer, cell, zoom and map facts on the right.
pub fn show_map(ui: &mut Ui, state: &mut AppState) {
    let muted = state.shell.shades.muted;
    let small = |t: String| egui::RichText::new(t).size(11.5).color(muted);
    let mut cancel = false;
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        if state.active.is_some() {
            ui.add(egui::Label::new(small(tool_hint(state).to_owned())).truncate());
        } else if let Some(loading) = &state.loading {
            ui.spinner();
            ui.label(small(format!(
                "{}: {}...",
                loading.profile_id, loading.stage
            )));
        } else if state.session.is_none() {
            ui.label(small("No profile open: Home > Profile".to_owned()));
        } else {
            ui.label(small("Pick a map in the list on the left".to_owned()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(doc) = state.active_doc() {
                let z = ZOOM_STEPS[doc.view.zoom.min(ZOOM_STEPS.len() - 1)];
                ui.label(small(format!(
                    "{}\u{d7}{} \u{b7} v{} \u{b7} Act {}",
                    doc.ds1.width,
                    doc.ds1.height,
                    doc.ds1.version,
                    doc.ds1.effective_act() + 1
                )));
                ui.label(small(format!("{}%", (100.0 / f64::from(z)).round())));
                ui.label(small(match state.hover_cell {
                    Some((x, y)) => format!("Cell {x}, {y}"),
                    None => "Cell \u{2014}".to_owned(),
                }));
                let layer = crate::doc::LayerKey::from_ref(doc.active_layer).label();
                ui.label(small(format!("{} \u{b7} {layer}", state.edit.tool.label())));
                let n = doc.selection.len();
                if n > 0 || !doc.object_sel.is_empty() {
                    ui.label(small(format!(
                        "{n} cell(s), {} object(s) selected",
                        doc.object_sel.len()
                    )));
                }
            }
            if let Some(at) = &state.history.last_autosave {
                ui.label(small(format!("autosaved {at}")))
                    .on_hover_text("The last autosave of maps with unsaved changes (History)");
            }
            if state.index.state.is_running() {
                let cancelling = state.index.cancelling();
                if ui
                    .add_enabled(!cancelling, egui::Button::new("Cancel").small())
                    .on_hover_text("Stop building the asset index")
                    .clicked()
                {
                    cancel = true;
                }
                let bar = match state.index.state.fraction() {
                    Some(f) => egui::ProgressBar::new(f).show_percentage(),
                    None => egui::ProgressBar::new(0.0).animate(true),
                };
                ui.add(bar.desired_width(110.0).desired_height(12.0));
            }
            if let Some(text) = index_text(&state.index.state) {
                let label = ui.label(small(text));
                if let IndexState::Failed(e) = &state.index.state {
                    label.on_hover_text(e.clone());
                }
            }
            if matches!(state.catalog, CatalogState::Building) {
                ui.label(small("Map catalog...".to_owned()));
            }
        });
    });
    if cancel {
        state.cancel_index();
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
