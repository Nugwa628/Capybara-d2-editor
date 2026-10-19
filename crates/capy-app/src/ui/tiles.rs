//! Tiles panel: the recently picked values (the full tile browser comes with
//! P3) and how many assets the background index holds.

use bevy_egui::egui::{self, Ui};

use super::thumbs;
use crate::doc::layer_label;
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

    ui.label(egui::RichText::new("Recent tiles").strong());
    if state.edit.recent.is_empty() {
        ui.weak("Pick tiles with the Eyedropper (I) on the map; they collect here.");
        return;
    }
    let recent = state.edit.recent.clone();
    let mut chosen = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (layer, value) in recent {
                let selected =
                    state.edit.current == value && state.edit.current_layer == Some(layer);
                let resp = thumbs::show(ui, state, layer, value, selected).on_hover_text(format!(
                    "{}: cell {:#010x}, orientation {}",
                    layer_label(layer),
                    value.cell,
                    value.orientation
                ));
                if resp.clicked() {
                    chosen = Some((layer, value));
                }
            }
        });
    });
    if let Some((layer, value)) = chosen {
        state.edit.current = value;
        state.edit.current_layer = Some(layer);
        if let Some(doc) = state.active_doc_mut()
            && layer.exists_in(&doc.ds1)
        {
            doc.active_layer = layer;
        }
        state.edit.tool = crate::edit::tools::Tool::Paint;
    }
}
