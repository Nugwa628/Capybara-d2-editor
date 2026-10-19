//! Layers panel: the active-layer picker, visibility toggles, and the
//! add / remove / upgrade menu of the active map.

use bevy_egui::egui::{self, Ui};
use capy_edit::LayerRef;

use crate::doc::{LayerKey, LayerKind, layer_label};
use crate::edit::ops::EditOp;
use crate::state::AppState;

/// The highest DS1 version this editor writes.
const LATEST_VERSION: i32 = 18;

/// What the layer menu can offer, read off the map before drawing.
#[derive(Clone, Copy, Debug)]
struct MapFacts {
    floors: usize,
    walls: usize,
    has_tag: bool,
    version: i32,
}

/// The right-click menu. `target` is the layer that was clicked, if any.
fn layer_menu(ui: &mut Ui, facts: MapFacts, target: Option<LayerRef>) -> Option<EditOp> {
    let mut op = None;
    if ui
        .add_enabled(facts.floors < 2, egui::Button::new("Add floor layer"))
        .on_disabled_hover_text("A map holds at most 2 floor layers")
        .clicked()
    {
        op = Some(EditOp::AddLayer(LayerRef::Floor(facts.floors as u8)));
    }
    if ui
        .add_enabled(facts.walls < 4, egui::Button::new("Add wall layer"))
        .on_disabled_hover_text("A map holds at most 4 wall layers")
        .clicked()
    {
        op = Some(EditOp::AddLayer(LayerRef::Wall(facts.walls as u8)));
    }
    if ui
        .add_enabled(!facts.has_tag, egui::Button::new("Add tag layer"))
        .clicked()
    {
        op = Some(EditOp::AddLayer(LayerRef::Tag));
    }
    if let Some(layer) = target {
        ui.separator();
        let removable = layer != LayerRef::Shadow;
        if ui
            .add_enabled(
                removable,
                egui::Button::new(format!("Remove {}", layer_label(layer))),
            )
            .on_disabled_hover_text("The shadow layer cannot be removed")
            .clicked()
        {
            op = Some(EditOp::RemoveLayer(layer));
        }
    }
    ui.separator();
    ui.menu_button(
        format!("Upgrade DS1 version (now v{})", facts.version),
        |ui| {
            if facts.version >= LATEST_VERSION {
                ui.label("Already the newest version");
            }
            for v in (facts.version + 1)..=LATEST_VERSION {
                if ui.button(format!("Upgrade to v{v}")).clicked() {
                    op = Some(EditOp::UpgradeVersion(v));
                    ui.close();
                }
            }
        },
    );
    if op.is_some() {
        ui.close();
    }
    op
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let Some(index) = state.active else {
        ui.weak("No map open");
        return;
    };
    let mut op: Option<EditOp> = None;
    {
        let doc = &mut state.docs[index];
        let facts = MapFacts {
            floors: doc.ds1.floors.len(),
            walls: doc.ds1.walls.len(),
            has_tag: doc.ds1.tag.is_some(),
            version: doc.ds1.version,
        };
        ui.label(egui::RichText::new(doc.path.file_name()).strong());
        ui.weak(format!("DS1 v{}", facts.version));
        ui.separator();
        ui.weak("Click a name to edit that layer; right-click for more");

        let mut changed: Vec<(LayerKey, bool)> = Vec::new();
        for key in doc.keys() {
            let layer = key.to_ref();
            let mut visible = doc.is_visible(key);
            let selected_here = doc.selection.iter().filter(|(l, _, _)| *l == layer).count();
            let mut activate = false;
            let row = ui.horizontal(|ui| {
                if ui.checkbox(&mut visible, "").changed() {
                    changed.push((key, visible));
                }
                let swatch = match key.kind() {
                    LayerKind::Floor => egui::Color32::from_rgb(84, 128, 77),
                    LayerKind::Wall => egui::Color32::from_rgb(158, 140, 120),
                    LayerKind::Shadow => egui::Color32::from_rgb(30, 30, 30),
                    LayerKind::Tag => egui::Color32::from_rgb(217, 64, 204),
                };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, swatch);
                let resp = ui.selectable_label(doc.active_layer == layer, key.label());
                if resp.clicked() {
                    activate = true;
                }
                if selected_here > 0 {
                    ui.weak(format!("{selected_here} selected"));
                }
                resp
            });
            if activate {
                doc.active_layer = layer;
            }
            row.inner.context_menu(|ui| {
                if let Some(chosen) = layer_menu(ui, facts, Some(layer)) {
                    op = Some(chosen);
                }
            });
        }
        for (key, visible) in changed {
            doc.set_visible(key, visible);
        }

        ui.menu_button("Layer actions", |ui| {
            if let Some(chosen) = layer_menu(ui, facts, None) {
                op = Some(chosen);
            }
        });

        ui.separator();
        ui.checkbox(&mut doc.roofs_visible, "Roofs")
            .on_hover_text("Roof tiles (orientation 15)");
        ui.checkbox(&mut doc.special_visible, "Special tiles")
            .on_hover_text("Orientation 10 and 11 markers; hidden by default");

        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut doc.objects_visible, "Objects");
            ui.weak(format!("({})", doc.ds1.objects.len()));
        });
        ui.weak("Objects and NPCs draw as sprites; markers show with the Objects tool.");
    }
    if let Some(op) = op {
        state.exec(index, op);
    }
}
