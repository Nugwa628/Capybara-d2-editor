//! The automap editor: which automap picture (cel) each tile of the open map
//! gets from AutoMap.txt, with suggestions, import from another AutoMap.txt
//! and the safety check that stops a broken table from being saved.

use std::collections::{BTreeMap, HashMap};

use bevy_egui::egui::{self, Ui};
use capy_mod::automap::import::{merge_rows, read_rows};
use capy_mod::automap::safety::{invalid_rows, remove_invalid_rows, validate_save};
use capy_mod::automap::{
    AutomapTable, Scope, TileId, apply_suggestions, clear_tiles, pieces, set_cel, suggest,
};
use d2_formats::txt::TxtDoc;

use super::io;
use crate::state::AppState;

const MAX_UNDO: usize = 30;

#[derive(Default)]
pub struct AutomapUi {
    pub doc: Option<TxtDoc>,
    undo: Vec<TxtDoc>,
    pub dirty: bool,
    pub level: String,
    pub floors: bool,
    import_path: String,
    pub msg: String,
}

impl AutomapUi {
    fn push_undo(&mut self) {
        if let Some(d) = &self.doc {
            self.undo.push(d.clone());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
    }
}

pub fn on_open(state: &mut AppState) {
    let mut a = std::mem::take(&mut state.tools.automap);
    match io::read_txt(state, "automap.txt") {
        Ok(doc) => {
            let table = AutomapTable::parse(&doc);
            if a.level.is_empty() || !table.levels.contains(&a.level) {
                let (act, lvl_type) = state
                    .active_doc()
                    .and_then(|d| d.info.as_ref())
                    .map_or((None, None), |i| {
                        (Some(i32::from(i.act)), i.lvl_type.map(|t| t as i32))
                    });
                a.level = table
                    .level_for(None, act, lvl_type)
                    .or_else(|| table.levels.first().cloned())
                    .unwrap_or_default();
            }
            a.doc = Some(doc);
            a.undo.clear();
            a.dirty = false;
            a.msg.clear();
        }
        Err(e) => a.msg = e,
    }
    state.tools.automap = a;
}

/// Sets the cel of one tile of the open map (scripts and the table).
pub fn set_tile_cel(
    state: &mut AppState,
    tile: TileId,
    cel: i32,
    style_wide: bool,
) -> Result<String, String> {
    let a = &mut state.tools.automap;
    a.push_undo();
    let level = a.level.clone();
    let doc = a.doc.as_mut().ok_or("AutoMap.txt is not open")?;
    let r = set_cel(
        doc,
        &level,
        tile.orientation,
        tile.main,
        tile.sub,
        cel,
        if style_wide {
            Scope::Style
        } else {
            Scope::Sequence
        },
    )
    .map_err(|e| e.to_string());
    match r {
        Ok(s) => {
            a.dirty |= !s.changes.is_empty();
            if s.changes.is_empty() {
                a.undo.pop();
            }
            Ok(s.summary)
        }
        Err(e) => {
            a.undo.pop();
            Err(e)
        }
    }
}

pub fn save(state: &mut AppState) -> Result<(), String> {
    let doc = state
        .tools
        .automap
        .doc
        .clone()
        .ok_or("AutoMap.txt is not open")?;
    let bytes = doc.to_bytes();
    validate_save("data/global/excel/automap.txt", &bytes).map_err(|e| e.to_string())?;
    io::save_txt(state, "automap.txt", &doc)?;
    state.tools.automap.dirty = false;
    Ok(())
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let mut a = std::mem::take(&mut state.tools.automap);
    if a.doc.is_none() {
        super::message(ui, &a.msg);
        if ui.button("Open AutoMap.txt").clicked() {
            state.tools.automap = a;
            on_open(state);
            return;
        }
        state.tools.automap = a;
        return;
    }
    let table = AutomapTable::parse(a.doc.as_ref().expect("doc"));
    ui.horizontal(|ui| {
        ui.label("Level:");
        egui::ComboBox::from_id_salt("am_level")
            .width(260.0)
            .selected_text(a.level.as_str())
            .show_ui(ui, |ui| {
                for l in &table.levels {
                    ui.selectable_value(&mut a.level, l.clone(), l.as_str());
                }
            });
        ui.checkbox(&mut a.floors, "floors too");
        if ui
            .add_enabled(!a.undo.is_empty(), egui::Button::new("Undo"))
            .clicked()
            && let Some(prev) = a.undo.pop()
        {
            a.doc = Some(prev);
            a.dirty = true;
        }
        if ui.add_enabled(a.dirty, egui::Button::new("Save")).clicked() {
            state.tools.automap = std::mem::take(&mut a);
            let r = save(state);
            a = std::mem::take(&mut state.tools.automap);
            a.msg = r.map_or_else(|e| e, |()| "Saved AutoMap.txt".into());
        }
    });
    let all = state
        .active_doc()
        .map(|d| pieces(d.edit.ds1(), &table, &a.level))
        .unwrap_or_default();
    // One row per distinct tile.
    let mut groups: BTreeMap<TileId, (usize, Option<usize>, Option<i32>)> = BTreeMap::new();
    for p in &all {
        let e = groups.entry(p.tile).or_insert((0, p.rule_row, p.cel));
        e.0 += 1;
    }
    let unmapped = groups.values().filter(|g| g.2.is_none()).count();
    ui.label(format!(
        "{} distinct tile(s) in the open map, {unmapped} without an automap piece. {} rule(s) for this level.",
        groups.len(),
        table.rules.iter().filter(|r| r.level == a.level).count()
    ));
    ui.horizontal(|ui| {
        let sug = suggest(&table, &a.level, &all, a.floors);
        if ui
            .add_enabled(
                !sug.is_empty(),
                egui::Button::new(format!("Apply {} suggestion(s)", sug.len())),
            )
            .on_hover_text("Give the unmapped tiles the cel that similar tiles of this level use")
            .clicked()
        {
            a.push_undo();
            let level = a.level.clone();
            if let Some(doc) = a.doc.as_mut() {
                match apply_suggestions(doc, &level, &sug) {
                    Ok(s) => {
                        a.dirty |= !s.changes.is_empty();
                        a.msg = s.summary;
                    }
                    Err(e) => {
                        a.msg = e.to_string();
                        a.undo.pop();
                    }
                }
            }
        }
        let bad = a
            .doc
            .as_ref()
            .and_then(|d| invalid_rows(d).ok())
            .map_or(0, |b| b.len());
        if ui
            .add_enabled(
                bad > 0,
                egui::Button::new(format!("Remove {bad} invalid row(s)")),
            )
            .on_hover_text("Rows whose Cel1 is -1 crash the game")
            .clicked()
        {
            a.push_undo();
            if let Some(doc) = a.doc.as_mut() {
                match remove_invalid_rows(doc) {
                    Ok(n) => {
                        a.dirty |= n > 0;
                        a.msg = format!("Removed {n} invalid row(s)");
                    }
                    Err(e) => a.msg = e.to_string(),
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Import rows from");
        ui.add(egui::TextEdit::singleline(&mut a.import_path).desired_width(300.0));
        if ui.button("Browse...").clicked()
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("txt", &["txt"])
                .pick_file()
        {
            a.import_path = p.display().to_string();
        }
        if ui.button("Import").clicked() {
            a.msg = match std::fs::read(&a.import_path) {
                Ok(bytes) => match read_rows(&bytes) {
                    Some(src) => {
                        let mapping: HashMap<String, Option<String>> = src
                            .levels
                            .iter()
                            .map(|l| (l.name.clone(), Some(l.name.clone())))
                            .collect();
                        a.push_undo();
                        match a.doc.as_mut().map(|d| merge_rows(d, &src, &mapping)) {
                            Some(Ok(r)) => {
                                a.dirty = true;
                                format!("Imported: {r:?}")
                            }
                            Some(Err(e)) => {
                                a.undo.pop();
                                e.to_string()
                            }
                            None => String::new(),
                        }
                    }
                    None => "Cannot read: not an AutoMap table".into(),
                },
                Err(e) => format!("Cannot read {}: {e}", a.import_path),
            };
        }
    });
    super::message(ui, &a.msg);
    ui.separator();
    let mut edit: Option<(TileId, i32)> = None;
    let mut clear: Option<TileId> = None;
    egui::ScrollArea::vertical()
        .id_salt("am_pieces")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("am_grid")
                .striped(true)
                .num_columns(6)
                .show(ui, |ui| {
                    for h in ["Orientation", "Main", "Sub", "Placed", "Rule row", "Cel"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for (t, (n, rule, cel)) in &groups {
                        ui.label(capy_mod::automap::code_name(
                            capy_mod::automap::AUTOMAP_CODES
                                .get(t.orientation)
                                .copied()
                                .unwrap_or(""),
                        ));
                        ui.label(format!("{}", t.main));
                        ui.label(format!("{}", t.sub));
                        ui.label(format!("{n}"));
                        ui.label(rule.map_or("-".to_owned(), |r| format!("{r}")));
                        ui.horizontal(|ui| {
                            let mut v = cel.unwrap_or(-1);
                            if ui
                                .add(egui::DragValue::new(&mut v).range(-1..=1000))
                                .changed()
                            {
                                edit = Some((*t, v));
                            }
                            if cel.is_some() && ui.small_button("Clear").clicked() {
                                clear = Some(*t);
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    if let Some((t, v)) = edit {
        state.tools.automap = std::mem::take(&mut a);
        let r = set_tile_cel(state, t, v, false);
        a = std::mem::take(&mut state.tools.automap);
        a.msg = r.unwrap_or_else(|e| e);
    }
    if let Some(t) = clear {
        a.push_undo();
        let level = a.level.clone();
        if let Some(doc) = a.doc.as_mut() {
            match clear_tiles(doc, &level, &[t]) {
                Ok(s) => {
                    a.dirty |= !s.changes.is_empty();
                    a.msg = s.summary;
                }
                Err(e) => {
                    a.msg = e.to_string();
                    a.undo.pop();
                }
            }
        }
    }
    state.tools.automap = a;
}
