//! Properties panel: facts about the active map, how the game loads it (the
//! map context, with an override), how it rendered, and the hovered cell.

use bevy_egui::egui::{self, Ui};
use d2_assets::{MapInfo, MapInfoSource};
use d2_formats::ds1::Ds1;

use crate::doc::RenderState;
use crate::state::{AppState, OverrideEdit};

/// One line per layer describing the cell at `(x, y)`.
pub fn describe_cell(ds1: &Ds1, x: u32, y: u32) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, floor) in ds1.floors.iter().enumerate() {
        if let Some(c) = floor.get(x, y).filter(|c| !c.is_empty()) {
            lines.push(format!(
                "Floor {}: main {} sub {} (prop1 {})",
                i + 1,
                c.main_index(),
                c.sub_index(),
                c.prop1()
            ));
        }
    }
    if let Some(c) = ds1.shadow.get(x, y).filter(|c| !c.is_empty()) {
        lines.push(format!(
            "Shadow: main {} sub {}",
            c.main_index(),
            c.sub_index()
        ));
    }
    for (i, wall) in ds1.walls.iter().enumerate() {
        if let Some(c) = wall.tiles.get(x, y).filter(|c| !c.is_empty()) {
            let orient = wall.orientations.get(x, y).unwrap_or(0);
            lines.push(format!(
                "Wall {}: main {} sub {} orient {orient}",
                i + 1,
                c.main_index(),
                c.sub_index()
            ));
        }
    }
    if let Some(t) = ds1
        .tag
        .as_ref()
        .and_then(|g| g.get(x, y))
        .filter(|&t| t != 0)
    {
        lines.push(format!("Tag: {t}"));
    }
    lines
}

pub fn source_label(source: MapInfoSource) -> &'static str {
    match source {
        MapInfoSource::Override => "override",
        MapInfoSource::LvlPrest => "LvlPrest",
        MapInfoSource::PathGuess => "guess (same folder)",
        MapInfoSource::TileCoverage => "guess (tile coverage)",
        MapInfoSource::Ds1Header => "DS1 header only",
    }
}

/// One-line description of an interpretation, for the combo box.
pub fn interpretation_label(info: &MapInfo) -> String {
    format!(
        "{}: LvlType {}, Def {}{}",
        source_label(info.source),
        info.lvl_type.map_or("-".to_owned(), |v| v.to_string()),
        info.lvlprest_def.map_or("-".to_owned(), |v| v.to_string()),
        info.level_name
            .as_deref()
            .map(|n| format!(", {n}"))
            .unwrap_or_default()
    )
}

/// What the override controls asked for.
enum Action {
    Set(u32, Option<u32>),
    Clear,
}

fn context_rows(ui: &mut Ui, info: &MapInfo) {
    let mut row = |k: &str, v: String| {
        ui.label(k);
        ui.label(v);
        ui.end_row();
    };
    row("Context", source_label(info.source).to_owned());
    row(
        "LvlType",
        info.lvl_type.map_or("none".to_owned(), |v| v.to_string()),
    );
    row(
        "LvlPrest Def",
        info.lvlprest_def
            .map_or("none".to_owned(), |v| match info.lvlprest_slot {
                Some(slot) => format!("{v} (File{})", slot + 1),
                None => v.to_string(),
            }),
    );
    row(
        "Level",
        match (info.level_id, &info.level_name) {
            (Some(id), Some(name)) => format!("{id}: {name}"),
            (Some(id), None) => id.to_string(),
            _ => "none".to_owned(),
        },
    );
    row("Act (context)", (info.act + 1).to_string());
    row("Palette act", (info.palette_act + 1).to_string());
    row(
        "Coverage",
        info.coverage
            .map_or("n/a".to_owned(), |c| format!("{:.1}%", c * 100.0)),
    );
    row("DT1 files", info.dt1_files.len().to_string());
}

/// The LvlType/Def pickers. Returns what to do, if anything.
fn override_controls(
    ui: &mut Ui,
    state_catalog: Option<&d2_assets::MapCatalog>,
    info: &MapInfo,
    edit: &mut OverrideEdit,
) -> Option<Action> {
    let mut action = None;
    let Some(catalog) = state_catalog else {
        ui.weak("The map catalog is still loading.");
        return None;
    };
    let interpretations = catalog.lookup(&info.ds1);
    ui.horizontal(|ui| {
        ui.label("Interpretation");
        egui::ComboBox::from_id_salt("interpretation")
            .selected_text(interpretation_label(info))
            .width(150.0)
            .show_ui(ui, |ui| {
                for candidate in &interpretations {
                    let selected = candidate.source == info.source
                        && candidate.lvl_type == info.lvl_type
                        && candidate.lvlprest_def == info.lvlprest_def;
                    if ui
                        .selectable_label(selected, interpretation_label(candidate))
                        .clicked()
                        && !selected
                        && let Some(lvl_type) = candidate.lvl_type
                    {
                        action = Some(Action::Set(lvl_type, candidate.lvlprest_def));
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("LvlType");
        egui::ComboBox::from_id_salt("lvltype_override")
            .selected_text(edit.lvl_type.to_string())
            .width(70.0)
            .show_ui(ui, |ui| {
                for id in catalog.lvl_type_ids() {
                    ui.selectable_value(&mut edit.lvl_type, id, id.to_string());
                }
            });
        ui.label("Def");
        ui.add(
            egui::TextEdit::singleline(&mut edit.def_text)
                .hint_text("none")
                .desired_width(50.0),
        );
    });
    ui.horizontal(|ui| {
        match edit.def() {
            Ok(def) => {
                if ui.button("Apply").clicked() {
                    action = Some(Action::Set(edit.lvl_type, def));
                }
            }
            Err(e) => {
                ui.add_enabled(false, egui::Button::new("Apply"));
                ui.colored_label(egui::Color32::from_rgb(230, 120, 100), e);
            }
        }
        if ui
            .add_enabled(
                info.source == MapInfoSource::Override,
                egui::Button::new("Clear override"),
            )
            .on_hover_text("Go back to the catalog's own choice")
            .clicked()
        {
            action = Some(Action::Clear);
        }
    });
    action
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    // Keep the pickers in step with the map being shown.
    let doc_marker = state.active_doc().map(|d| (d.id, d.info.clone()));
    match &doc_marker {
        Some((id, Some(info))) if state.override_edit.doc != Some(*id) => {
            state.override_edit = OverrideEdit {
                doc: Some(*id),
                lvl_type: info.lvl_type.unwrap_or(0),
                def_text: info.lvlprest_def.map(|d| d.to_string()).unwrap_or_default(),
            };
        }
        Some((id, None)) if state.override_edit.doc == Some(*id) => {
            state.override_edit.doc = None; // refill once the context arrives
        }
        _ => {}
    }
    let mut edit = state.override_edit.clone();
    let mut action: Option<Action> = None;

    let Some(doc) = state.active_doc() else {
        ui.weak("No map open");
        return;
    };
    let ds1 = &doc.ds1;
    egui::Grid::new("props").num_columns(2).show(ui, |ui| {
        let mut row = |k: &str, v: String| {
            ui.label(k);
            ui.label(v);
            ui.end_row();
        };
        row("File", doc.path.to_string());
        row(
            "From",
            format!(
                "{} ({})",
                doc.source.layer_name,
                doc.source.category.label()
            ),
        );
        row("Profile", doc.profile_id.clone());
        row("DS1 version", ds1.version.to_string());
        row("Size", format!("{} x {} cells", ds1.width, ds1.height));
        row("Act (DS1 header)", (ds1.effective_act() + 1).to_string());
        row(
            "Floors / walls",
            format!("{} / {}", ds1.floors.len(), ds1.walls.len()),
        );
        row(
            "Tag layer",
            if ds1.tag.is_some() { "yes" } else { "no" }.to_owned(),
        );
        row("Objects", ds1.objects.len().to_string());
        row("Groups", ds1.groups.len().to_string());
        row("NPC paths", ds1.npc_paths.len().to_string());
    });

    ui.separator();
    ui.label(egui::RichText::new("Map context").strong());
    match &doc.info {
        Some(info) => {
            egui::Grid::new("context")
                .num_columns(2)
                .show(ui, |ui| context_rows(ui, info));
            if !info.missing_dt1.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 170, 90),
                    format!(
                        "{} DT1 file(s) not found in the profile",
                        info.missing_dt1.len()
                    ),
                )
                .on_hover_text(
                    info.missing_dt1
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            action = override_controls(ui, state.catalog.ready(), info, &mut edit);
        }
        None => {
            ui.weak(match &state.catalog {
                crate::state::CatalogState::Failed(e) => format!("No map catalog: {e}"),
                _ => "Waiting for the map catalog...".to_owned(),
            });
        }
    }

    ui.separator();
    ui.label(egui::RichText::new("Rendering").strong());
    let r = &doc.render;
    egui::Grid::new("render").num_columns(2).show(ui, |ui| {
        ui.label("State");
        match &r.state {
            RenderState::WaitingForContext => ui.label("waiting for map context"),
            RenderState::Loading => ui.label("loading tiles..."),
            RenderState::Ready => ui.label("ready"),
            RenderState::Failed(e) => ui.colored_label(egui::Color32::from_rgb(230, 120, 100), e),
            RenderState::ProfileClosed => ui.label("profile not open"),
        };
        ui.end_row();
        if r.is_ready() {
            ui.label("Tiles drawn");
            ui.label(r.tiles_drawn.to_string());
            ui.end_row();
            ui.label("Missing tiles");
            let text = r.missing.to_string();
            if r.missing > 0 {
                ui.colored_label(egui::Color32::from_rgb(255, 80, 255), text)
                    .on_hover_text(r.missing_top.join("\n"));
            } else {
                ui.label(text);
            }
            ui.end_row();
            if let Some((size, layers)) = r.atlas {
                ui.label("Atlas");
                ui.label(format!("{size} x {size} x {layers}"));
                ui.end_row();
            }
            ui.label("Built in");
            ui.label(format!("{} ms", r.millis));
            ui.end_row();
        }
    });
    for line in r.missing_top.iter().take(5) {
        ui.small(line);
    }
    for w in r.warnings.iter().take(5) {
        ui.small(egui::RichText::new(w).color(egui::Color32::from_rgb(230, 170, 90)));
    }

    ui.separator();
    ui.label(egui::RichText::new("Hovered cell").strong());
    match state.hover_cell {
        Some((x, y)) if x >= 0 && y >= 0 => {
            ui.label(format!("({x}, {y})"));
            let lines = describe_cell(ds1, x as u32, y as u32);
            if lines.is_empty() {
                ui.weak("empty");
            }
            for l in lines {
                ui.label(l);
            }
        }
        _ => {
            ui.weak("\u{2014}");
        }
    }

    ui.separator();
    ui.collapsing(
        format!("Tile files ({})", ds1.tileset_refs().count()),
        |ui| {
            for r in ds1.tileset_refs() {
                ui.monospace(String::from_utf8_lossy(r));
            }
        },
    );

    // Apply what the controls asked for, now that `doc` is no longer borrowed.
    state.override_edit = edit;
    if let (Some(action), Some(index)) = (action, state.active) {
        match action {
            Action::Set(lvl_type, def) => state.set_map_override(index, lvl_type, def),
            Action::Clear => state.clear_map_override(index),
        }
        state.override_edit.doc = None; // refill from the new context
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::ds1::Cell;

    #[test]
    fn describes_only_filled_layers() {
        let mut ds1 = Ds1::new(3, 3, 0);
        assert!(describe_cell(&ds1, 1, 1).is_empty());
        ds1.floors[0].set(1, 1, Cell(1).with_indices(2, 3));
        let lines = describe_cell(&ds1, 1, 1);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("main 2 sub 3"), "{lines:?}");
        assert!(
            describe_cell(&ds1, 99, 99).is_empty(),
            "out of range is empty"
        );
    }

    #[test]
    fn def_text_parses_or_explains() {
        let mut e = OverrideEdit::default();
        assert_eq!(e.def(), Ok(None));
        e.def_text = " 12 ".into();
        assert_eq!(e.def(), Ok(Some(12)));
        e.def_text = "x".into();
        assert!(e.def().is_err());
    }
}
