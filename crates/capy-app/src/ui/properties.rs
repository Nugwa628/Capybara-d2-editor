//! Properties panel: facts about the active map and the hovered cell.

use bevy_egui::egui::{self, Ui};
use d2_formats::ds1::Ds1;

use crate::state::AppState;

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

pub fn show(ui: &mut Ui, state: &mut AppState) {
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
        row("Act", (ds1.effective_act() + 1).to_string());
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
}
