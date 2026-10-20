//! Upper and lower walls as Show-bar groups, and the DT1 wall-categories row
//! under the Show bar (ds1-studio `wallCategories.ts`, `MapLayerBar.tsx`):
//! a wall tile is a lower wall when its orientation is 16 or more and an
//! upper wall for 1-9, 12 and 14, unless its DT1 is set to one group. It is
//! an editor preference only; the game files stay as they are.

use bevy_egui::egui::{self, RichText, Ui};

use super::super::shell::chip;
use crate::doc::LayerKey;
use crate::render::job::wall_group;
use crate::state::AppState;

/// The two group chips and the row's toggle, for the Show bar.
pub fn show_bar_chips(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    ui.add_space(6.0);
    for (id, label, on) in [
        ("layer.upperWalls", "Upper walls", state.studio.upper_walls),
        ("layer.lowerWalls", "Lower walls", state.studio.lower_walls),
    ] {
        let key = state.shell.key_of(id);
        let tip = if key.is_empty() {
            format!("Show or hide {} on every wall layer", label.to_lowercase())
        } else {
            format!(
                "Show or hide {} on every wall layer ({key})",
                label.to_lowercase()
            )
        };
        if chip(ui, &s, label, on, true).on_hover_text(tip).clicked() {
            super::super::commands::run(state, id);
        }
    }
    let open = state.config.ui.studio.wall_row;
    if chip(ui, &s, "Wall categories", open, true)
        .on_hover_text("Which group each DT1's wall tiles belong to")
        .clicked()
    {
        super::super::commands::run(state, "view.wallCategories");
    }
}

/// One DT1 of the map with wall tiles: (path, lower-case key, upper, lower).
fn wall_dt1s(state: &AppState) -> Vec<(String, String, usize, usize)> {
    let Some(tiles) = state.active_doc().and_then(|d| d.tiles.clone()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (d, path) in tiles.dt1_paths().iter().enumerate() {
        let (mut upper, mut lower) = (0, 0);
        let mut t = 0;
        while let Some(tile) = tiles.tile(&capy_render::TileRef { dt1: d, tile: t }) {
            match wall_group(tile.orientation) {
                Some(true) => lower += 1,
                Some(false) => upper += 1,
                None => {}
            }
            t += 1;
        }
        if upper + lower > 0 {
            out.push((
                path.as_str()
                    .trim_start_matches("data\\global\\tiles\\")
                    .replace('\\', "/"),
                path.as_str().to_ascii_lowercase(),
                upper,
                lower,
            ));
        }
    }
    out
}

/// The collapsible row of DT1s with their wall group.
pub fn row(ui: &mut Ui, state: &mut AppState) {
    if !state.config.ui.studio.wall_row || state.active.is_none() {
        return;
    }
    let s = state.shell.shades;
    let list = wall_dt1s(state);
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Wall categories").color(s.muted).size(11.5))
            .on_hover_text(
                "Upper/Lower are visibility groups, separate from Wall 1-4. Automatic uses \
                 the DT1 tile type; set a DT1 here when its artwork belongs in the other \
                 group. Editor preference only.",
            );
        if list.is_empty() {
            ui.label(
                RichText::new("no wall tiles (or the map is not drawn yet)")
                    .color(s.muted)
                    .size(11.5),
            );
        }
        for (label, key, upper, lower) in list {
            let current = state.config.ui.studio.wall_categories.get(&key).cloned();
            let shown = match current.as_deref() {
                Some("upper") => "Upper",
                Some("lower") => "Lower",
                _ => "Auto",
            };
            let name = label.rsplit('/').next().unwrap_or(&label).to_owned();
            egui::ComboBox::from_id_salt(("wallcat", key.clone()))
                .selected_text(format!("{name}: {shown}"))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (value, text) in [
                        (None, "Automatic (DT1 type)"),
                        (Some("upper"), "Upper walls"),
                        (Some("lower"), "Lower walls"),
                    ] {
                        if ui
                            .selectable_label(current.as_deref() == value, text)
                            .clicked()
                        {
                            let map = &mut state.config.ui.studio.wall_categories;
                            match value {
                                Some(v) => {
                                    map.insert(key.clone(), v.to_owned());
                                }
                                None => {
                                    map.remove(&key);
                                }
                            }
                            state.config_dirty = true;
                        }
                    }
                })
                .response
                .on_hover_text(format!("{label}: {upper} upper / {lower} lower wall tiles"));
        }
    });
}

/// Shift+9: show only the lower walls; again: everything back.
pub fn solo_lower(state: &mut AppState) {
    let (upper, lower) = (state.studio.upper_walls, state.studio.lower_walls);
    let Some(doc) = state.active_doc_mut() else {
        return;
    };
    let soloed = !upper
        && lower
        && !doc.roofs_visible
        && doc.keys().iter().all(|k| match k {
            LayerKey::Wall(_) => doc.is_visible(*k),
            _ => !doc.is_visible(*k),
        });
    if soloed {
        doc.hidden.clear();
        doc.roofs_visible = true;
        doc.special_visible = true;
        state.studio.upper_walls = true;
        state.studio.lower_walls = true;
        return;
    }
    for k in doc.keys() {
        doc.set_visible(k, matches!(k, LayerKey::Wall(_)));
    }
    doc.roofs_visible = false;
    doc.special_visible = false;
    state.studio.upper_walls = false;
    state.studio.lower_walls = true;
}
