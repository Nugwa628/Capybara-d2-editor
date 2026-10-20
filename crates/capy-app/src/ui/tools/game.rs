//! The Game tab's level and data tools: Add to game, Change level type,
//! Type packages, Entering text, Cube recipe and Custom object. Each plans
//! its change with `capy-mod` (pure edits of the tables), shows the change
//! list, and on Apply writes the edited tables to the profile's saves.

use std::collections::BTreeSet;

use bevy_egui::egui::{self, Ui};
use capy_edit::Patch;
use capy_mod::add_map::{AddMapInput, AddMode, add_map_to_game};
use capy_mod::change_type::{
    ChangeTypeInput, LevelTypeChoice, change_level_type, level_type_choices,
};
use capy_mod::cube::{CubeItemInput, CubeTables, MapTarget, plan_cube_item};
use capy_mod::custom_object::{
    CustomObjectOptions, custom_object_files, custom_object_row, free_token,
};
use capy_mod::entry_text::{
    ENTRY_IMAGE_DIR, Font, PATCH_STRINGS, entry_dc6, levels_using_entry_file, render_entry_text,
    set_area_name, set_entry_file, suggest_entry_name,
};
use capy_mod::level_tables::{LevelTables, TableName};
use capy_mod::type_package::{TypePackage, export_type_package, import_type_package};
use d2_assets::GamePath;
use d2_formats::dc6::Dc6;
use d2_formats::txt::TxtDoc;

use super::io;
use crate::state::AppState;

#[derive(Default)]
pub struct GameUi {
    // add to game
    pub add_new: bool,
    pub add_level: u32,
    pub add_name: String,
    pub add_rel: String,
    pub add_new_type: bool,
    pub add_lines: Vec<String>,
    pub add_msg: String,
    // change type
    pub ct_choices: Vec<LevelTypeChoice>,
    pub ct_target: u32,
    pub ct_copy: bool,
    pub ct_name: String,
    pub ct_lines: Vec<String>,
    pub ct_msg: String,
    // type packages
    pub tp_type: u32,
    pub tp_path: String,
    pub tp_msg: String,
    // entering text
    pub et_level: u32,
    pub et_text: String,
    pub et_key: String,
    pub et_entry: String,
    pub et_font: String,
    pub et_image: bool,
    pub et_msg: String,
    // cube
    pub cu_template: String,
    pub cu_name: String,
    pub cu_code: String,
    pub cu_inputs: String,
    pub cu_num: u32,
    pub cu_map_name: String,
    pub cu_level: String,
    pub cu_accept: bool,
    pub cu_lines: Vec<String>,
    pub cu_msg: String,
    // custom object
    pub co_row: usize,
    pub co_name: String,
    pub co_png: String,
    pub co_frames: u32,
    pub co_act: u8,
    pub co_blocks: bool,
    pub co_msg: String,
}

type Facts = (String, u32, u32, Vec<String>, Option<u32>);

fn map_facts(state: &AppState) -> Option<Facts> {
    let doc = state.active_doc()?;
    let ds1 = doc.edit.ds1();
    let dt1s = doc
        .info
        .as_ref()
        .map(|i| {
            i.dt1_files
                .iter()
                .map(|p| io::tiles_rel(p.as_str()))
                .collect()
        })
        .unwrap_or_default();
    Some((
        io::tiles_rel(doc.path.as_str()),
        ds1.width,
        ds1.height,
        dt1s,
        doc.info.as_ref().and_then(|i| i.level_id),
    ))
}

pub fn on_open(state: &mut AppState, id: &str) {
    let facts = map_facts(state);
    let mut g = std::mem::take(&mut state.tools.game);
    match id {
        "game.addToGame" => {
            if let Some((rel, _, _, _, level)) = facts {
                if g.add_rel.is_empty() {
                    g.add_rel = rel.clone();
                }
                if g.add_name.is_empty() {
                    g.add_name = rel
                        .rsplit('/')
                        .next()
                        .and_then(|f| f.strip_suffix(".ds1"))
                        .unwrap_or("custom")
                        .to_owned();
                }
                g.add_level = level.unwrap_or(1);
            }
            g.add_lines.clear();
        }
        "game.changeType" => {
            g.ct_lines.clear();
            g.ct_msg.clear();
            g.ct_choices.clear();
            if let Some((rel, _, _, dt1s, level)) = facts {
                match io::level_tables(state).and_then(|t| {
                    level_type_choices(&t, &rel, &dt1s, level).map_err(|e| e.to_string())
                }) {
                    Ok(c) => {
                        g.ct_target = c.first().map_or(0, |c| c.id);
                        g.ct_choices = c;
                    }
                    Err(e) => g.ct_msg = e,
                }
            }
        }
        "game.entryText" => {
            if g.et_font.is_empty() {
                g.et_font = "font42".into();
            }
            if let Some(level) = state
                .active_doc()
                .and_then(|d| d.info.as_ref())
                .and_then(|i| i.level_id)
            {
                g.et_level = level;
            }
        }
        _ => {}
    }
    state.tools.game = g;
}

fn changed_tables(before: &LevelTables, after: &LevelTables) -> BTreeSet<TableName> {
    let mut set = BTreeSet::new();
    if before.levels != after.levels {
        set.insert(TableName::Levels);
    }
    if before.prest != after.prest {
        set.insert(TableName::LvlPrest);
    }
    if before.types != after.types {
        set.insert(TableName::LvlTypes);
    }
    set
}

// ───────────────────────── add to game ─────────────────────────

pub struct AddForm {
    pub new_level: bool,
    pub level_id: u32,
    pub name: String,
    pub map_rel: String,
    pub new_type: bool,
}

/// Plans (and with `apply` saves) registering the open map in the level
/// tables. The map is written to the saves under its new path when that
/// differs from where it was opened. Returns the change lines.
pub fn add_to_game_run(
    state: &mut AppState,
    form: &AddForm,
    apply: bool,
) -> Result<Vec<String>, String> {
    let (cur_rel, w, h, dt1s, _) = map_facts(state).ok_or("No map is open")?;
    let rel = form.map_rel.trim().replace('\\', "/");
    if !rel.to_ascii_lowercase().ends_with(".ds1") {
        return Err("The map path must end in .ds1".into());
    }
    let tables = io::level_tables(state)?;
    let input = AddMapInput {
        mode: if form.new_level {
            AddMode::New
        } else {
            AddMode::Existing
        },
        level_id: form.level_id,
        name: form.name.trim().to_owned(),
        map_rel: rel.clone(),
        width: w,
        height: h,
        used_dt1s: dt1s,
        pop_count: 0,
        pal_act: None,
        new_type: form.new_type,
    };
    let out = add_map_to_game(&tables, &input).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = out.changes.iter().map(|c| c.summary()).collect();
    lines.extend(out.warnings.iter().map(|w| format!("Warning: {w}")));
    if lines.is_empty() {
        lines.push("The tables already have this map; nothing to change.".into());
    }
    if apply {
        if !rel.eq_ignore_ascii_case(&cur_rel) {
            let bytes = state
                .active_doc()
                .ok_or("No map is open")?
                .edit
                .ds1()
                .to_bytes()
                .map_err(|e| format!("cannot write the map: {e}"))?;
            let path = io::tiles_game(&rel);
            io::save_game(state, &path, &bytes)?;
            state.register_map(&path);
        }
        io::save_level_tables(state, &out.tables, &out.touched)?;
        lines.push(format!(
            "Saved {} table(s); level {} loads {}",
            out.touched.len(),
            out.level_id,
            rel
        ));
    }
    Ok(lines)
}

pub fn add_to_game(ui: &mut Ui, state: &mut AppState) {
    let mut g = std::mem::take(&mut state.tools.game);
    ui.label("Registers the open map in Levels.txt, LvlPrest.txt and LvlTypes.txt so the game loads it. The map is saved at the path below.");
    ui.separator();
    egui::Grid::new("add_game").num_columns(2).show(ui, |ui| {
        ui.label("Map path (under data\\global\\tiles)");
        ui.add(egui::TextEdit::singleline(&mut g.add_rel).desired_width(340.0));
        ui.end_row();
        ui.label("Level name");
        ui.add(egui::TextEdit::singleline(&mut g.add_name).desired_width(200.0));
        ui.end_row();
        ui.label("What it becomes");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut g.add_new, false, "A preset level's map");
            ui.selectable_value(&mut g.add_new, true, "A new level");
        });
        ui.end_row();
        ui.label(if g.add_new {
            "Copy the settings of level"
        } else {
            "Preset level (Levels.txt Id)"
        });
        ui.add(egui::DragValue::new(&mut g.add_level).range(0..=4000));
        ui.end_row();
        ui.label("Level type of its own");
        ui.checkbox(
            &mut g.add_new_type,
            "even if the template's type lists every library",
        );
        ui.end_row();
    });
    let form = AddForm {
        new_level: g.add_new,
        level_id: g.add_level,
        name: g.add_name.clone(),
        map_rel: g.add_rel.clone(),
        new_type: g.add_new_type,
    };
    ui.horizontal(|ui| {
        if ui.button("Preview the changes").clicked() {
            match add_to_game_run(state, &form, false) {
                Ok(l) => {
                    g.add_lines = l;
                    g.add_msg.clear();
                }
                Err(e) => g.add_msg = e,
            }
        }
        if ui.button("Add to game").clicked() {
            match add_to_game_run(state, &form, true) {
                Ok(l) => {
                    g.add_lines = l;
                    g.add_msg = "Done. The Maps list now shows the map.".into();
                }
                Err(e) => g.add_msg = e,
            }
        }
    });
    super::message(ui, &g.add_msg);
    for l in &g.add_lines {
        ui.add(egui::Label::new(l.as_str()).wrap());
    }
    state.tools.game = g;
}

// ───────────────────────── change level type ─────────────────────────

pub fn change_type_run(
    state: &mut AppState,
    target: u32,
    copy: bool,
    copy_name: Option<String>,
    apply: bool,
) -> Result<Vec<String>, String> {
    let (rel, _, _, dt1s, level) = map_facts(state).ok_or("No map is open")?;
    let tables = io::level_tables(state)?;
    let out = change_level_type(
        &tables,
        &ChangeTypeInput {
            map_rel: rel,
            level_id: level,
            target_type: target,
            copy,
            used_dt1s: dt1s,
            copy_name,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = out.changes.iter().map(|c| c.summary()).collect();
    lines.extend(out.warnings.iter().map(|w| format!("Warning: {w}")));
    if apply {
        let touched = changed_tables(&tables, &out.tables);
        io::save_level_tables(state, &out.tables, &touched)?;
        lines.push(format!(
            "Level {} now uses level type {}{}",
            out.level_id,
            out.type_id,
            if out.copied { " (a copy)" } else { "" }
        ));
    }
    Ok(lines)
}

pub fn change_type(ui: &mut Ui, state: &mut AppState) {
    let mut g = std::mem::take(&mut state.tools.game);
    ui.label("Points the open map's level at another tile set (level type) and fixes Dt1Mask.");
    if g.ct_choices.is_empty() {
        super::message(ui, &g.ct_msg);
        if ui.button("Refresh").clicked() {
            state.tools.game = g;
            on_open(state, "game.changeType");
            return;
        }
        state.tools.game = g;
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("ct_list")
        .max_height(220.0)
        .show(ui, |ui| {
            for c in &g.ct_choices {
                let text = format!(
                    "{} {} (act {}, {} libraries, {} other level(s)){}",
                    c.id,
                    c.name,
                    c.act + 1,
                    c.files,
                    c.users,
                    c.copy_only
                        .as_deref()
                        .map(|w| format!(" - copy only: {w}"))
                        .unwrap_or_default()
                );
                if ui.selectable_label(g.ct_target == c.id, text).clicked() {
                    g.ct_target = c.id;
                    g.ct_copy = c.copy_only.is_some();
                }
            }
        });
    ui.checkbox(&mut g.ct_copy, "Make a level type of its own (a copy)");
    ui.horizontal(|ui| {
        ui.label("Name of the copy");
        ui.text_edit_singleline(&mut g.ct_name);
    });
    let (t, c, n) = (g.ct_target, g.ct_copy, g.ct_name.trim().to_owned());
    let name = (!n.is_empty()).then_some(n);
    ui.horizontal(|ui| {
        if ui.button("Preview").clicked() {
            match change_type_run(state, t, c, name.clone(), false) {
                Ok(l) => {
                    g.ct_lines = l;
                    g.ct_msg.clear();
                }
                Err(e) => g.ct_msg = e,
            }
        }
        if ui.button("Change the level type").clicked() {
            match change_type_run(state, t, c, name.clone(), true) {
                Ok(l) => {
                    g.ct_lines = l;
                    g.ct_msg = "Saved.".into();
                }
                Err(e) => g.ct_msg = e,
            }
        }
    });
    super::message(ui, &g.ct_msg);
    for l in &g.ct_lines {
        ui.add(egui::Label::new(l.as_str()).wrap());
    }
    state.tools.game = g;
}

// ───────────────────────── type packages ─────────────────────────

pub fn export_type(state: &mut AppState, type_id: u32, path: &str) -> Result<String, String> {
    let tables = io::level_tables(state)?;
    let pkg = export_type_package(&tables, type_id).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&pkg).map_err(|e| e.to_string())?;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, json).map_err(|e| format!("cannot write {path}: {e}"))?;
    Ok(format!(
        "Exported level type {} \"{}\" ({} DT1s, {} maps) to {path}",
        pkg.type_id,
        pkg.type_name,
        pkg.dt1s.len(),
        pkg.maps.len()
    ))
}

pub fn import_type(state: &mut AppState, path: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let pkg: TypePackage =
        serde_json::from_str(&text).map_err(|e| format!("not a type package: {e}"))?;
    merge_type_package(state, &pkg)
}

/// Merges a type package into the profile's tables and saves them.
pub fn merge_type_package(state: &mut AppState, pkg: &TypePackage) -> Result<String, String> {
    let tables = io::level_tables(state)?;
    let out = import_type_package(&tables, pkg).map_err(|e| e.to_string())?;
    let touched = changed_tables(&tables, &out.tables);
    io::save_level_tables(state, &out.tables, &touched)?;
    Ok(format!(
        "Imported \"{}\": level type {}, {} change(s), {} warning(s)",
        pkg.type_name,
        out.type_id,
        out.changes.len(),
        out.warnings.len()
    ))
}

pub fn type_packages(ui: &mut Ui, state: &mut AppState) {
    let mut g = std::mem::take(&mut state.tools.game);
    if g.tp_path.is_empty() {
        g.tp_path = state
            .paths
            .root
            .join("exports")
            .join("type-package.json")
            .display()
            .to_string();
    }
    ui.label("A type package carries a level type with its DT1 list, levels and presets as one JSON file, to move it between mods.");
    ui.horizontal(|ui| {
        ui.label("Level type Id");
        ui.add(egui::DragValue::new(&mut g.tp_type).range(0..=500));
    });
    ui.horizontal(|ui| {
        ui.label("File");
        ui.add(egui::TextEdit::singleline(&mut g.tp_path).desired_width(380.0));
        if ui.button("Browse...").clicked()
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("json", &["json"])
                .pick_file()
        {
            g.tp_path = p.display().to_string();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Export").clicked() {
            g.tp_msg = export_type(state, g.tp_type, &g.tp_path).unwrap_or_else(|e| e);
        }
        if ui.button("Import (merge by name)").clicked() {
            g.tp_msg = import_type(state, &g.tp_path).unwrap_or_else(|e| e);
        }
    });
    super::message(ui, &g.tp_msg);
    state.tools.game = g;
}

// ───────────────────────── entering text ─────────────────────────

pub fn entry_text_run(
    state: &mut AppState,
    level: u32,
    text: &str,
    key: Option<&str>,
    entry: Option<(&str, &str)>,
) -> Result<String, String> {
    let mut levels = io::read_txt(state, "levels.txt")?;
    let mut strings = io::read_tbl(state, PATCH_STRINGS)?;
    let mut msg = Vec::new();
    if !text.trim().is_empty() {
        let out = set_area_name(&levels, &strings, level, text, key).map_err(|e| e.to_string())?;
        levels = out.levels;
        strings = out.strings;
        msg.push(format!(
            "Level {level} is named \"{}\" (key {})",
            text.trim(),
            out.key
        ));
        io::save_tbl(state, PATCH_STRINGS, &strings)?;
    }
    if let Some((name, font)) = entry {
        let path = set_entry_file(&mut levels, level, name).map_err(|e| e.to_string())?;
        if !font.is_empty() {
            let dc6_bytes = io::read_game(
                state,
                &GamePath::new(&format!("data/local/font/latin/{font}.dc6")),
            )?
            .ok_or_else(|| format!("the font {font}.dc6 is not in this profile"))?;
            let tbl_bytes = io::read_game(
                state,
                &GamePath::new(&format!("data/local/font/latin/{font}.tbl")),
            )?
            .ok_or_else(|| format!("the font {font}.tbl is not in this profile"))?;
            let dc6 = Dc6::parse(&dc6_bytes).map_err(|e| e.to_string())?;
            let f = Font::load(&dc6, &tbl_bytes).map_err(|e| e.to_string())?;
            let line = if text.trim().is_empty() {
                name
            } else {
                text.trim()
            };
            let img = render_entry_text(f.height, |c| f.glyph(c), line);
            let bytes = entry_dc6(&img)
                .and_then(|d| d.to_bytes().map_err(Into::into))
                .map_err(|e| e.to_string())?;
            io::save_game(state, &GamePath::new(&path), &bytes)?;
            msg.push(format!(
                "Wrote the loading image {path} ({} px wide)",
                img.width
            ));
        }
        msg.push(format!("EntryFile of level {level} is {name}"));
    }
    if msg.is_empty() {
        return Err("Give the level a name or an entry file".into());
    }
    io::save_txt(state, "levels.txt", &levels)?;
    state.rebuild_catalog();
    Ok(msg.join("; "))
}

pub fn entry_text(ui: &mut Ui, state: &mut AppState) {
    let mut g = std::mem::take(&mut state.tools.game);
    ui.label("The area name shown when the level is entered (patchstring.tbl) and the loading-screen text image (EntryFile).");
    egui::Grid::new("entry_text").num_columns(2).show(ui, |ui| {
        ui.label("Level (Levels.txt Id)");
        ui.add(egui::DragValue::new(&mut g.et_level).range(0..=4000));
        ui.end_row();
        ui.label("Name shown in game");
        ui.add(egui::TextEdit::singleline(&mut g.et_text).desired_width(280.0));
        ui.end_row();
        ui.label("String key (optional)");
        ui.add(egui::TextEdit::singleline(&mut g.et_key).desired_width(200.0));
        ui.end_row();
        ui.label("Loading image name");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut g.et_entry).desired_width(140.0));
            if ui.small_button("Suggest").clicked() {
                let levels = io::read_txt(state, "levels.txt").ok();
                let taken = |n: &str| {
                    levels
                        .as_ref()
                        .is_some_and(|l| !levels_using_entry_file(l, n).is_empty())
                };
                g.et_entry = suggest_entry_name(&g.et_text, taken);
            }
        });
        ui.end_row();
        ui.label("Draw the image");
        ui.horizontal(|ui| {
            ui.checkbox(&mut g.et_image, "with font");
            ui.add(egui::TextEdit::singleline(&mut g.et_font).desired_width(80.0));
        });
        ui.end_row();
    });
    ui.weak(format!(
        "The image is saved as {ENTRY_IMAGE_DIR}<name>.dc6."
    ));
    if ui.button("Apply").clicked() {
        let key = g.et_key.trim().to_owned();
        let name = g.et_entry.trim().to_owned();
        let font = if g.et_image {
            g.et_font.trim().to_owned()
        } else {
            String::new()
        };
        let entry = (!name.is_empty()).then_some((name.as_str(), font.as_str()));
        g.et_msg = entry_text_run(
            state,
            g.et_level,
            &g.et_text.clone(),
            (!key.is_empty()).then_some(key.as_str()),
            entry,
        )
        .unwrap_or_else(|e| e);
    }
    super::message(ui, &g.et_msg);
    state.tools.game = g;
}

// ───────────────────────── cube recipe ─────────────────────────

pub struct CubeForm {
    pub template: String,
    pub name: String,
    pub code: String,
    pub inputs: Vec<String>,
    pub numinputs: u32,
    pub map_name: String,
    pub level: Option<u32>,
    pub accept: bool,
}

/// Row of the Misc.txt item with this `code` or `name`.
fn misc_row(misc: &TxtDoc, key: &str) -> Option<usize> {
    let key = key.trim().to_ascii_lowercase();
    ["code", "name"].iter().find_map(|col| {
        let c = misc.column(col)?;
        (1..misc.row_count()).find(|&r| {
            misc.cell(r, c)
                .is_some_and(|v| v.trim().to_ascii_lowercase() == key)
        })
    })
}

pub fn cube_run(state: &mut AppState, form: &CubeForm, apply: bool) -> Result<Vec<String>, String> {
    let misc = io::read_txt(state, "misc.txt")?;
    let cube = io::read_txt(state, "cubemain.txt")?;
    let levels = io::read_txt(state, "levels.txt").ok();
    let strings = io::read_tbl(state, PATCH_STRINGS).ok();
    let template_row = misc_row(&misc, &form.template)
        .ok_or_else(|| format!("Misc.txt has no item \"{}\"", form.template))?;
    let tables = CubeTables {
        misc: misc.clone(),
        cube: cube.clone(),
        levels: levels.clone(),
        strings: strings.clone(),
    };
    let input = CubeItemInput {
        template_row,
        name: form.name.clone(),
        code: form.code.clone(),
        inputs: form.inputs.clone(),
        numinputs: form.numinputs,
        map_name: form.map_name.clone(),
        accept_mod_type: form.accept,
        map: form.level.map(|level_id| MapTarget {
            level_id,
            level_title: None,
        }),
    };
    let out = plan_cube_item(&tables, &input).map_err(|e| e.to_string())?;
    let mut lines = out.changes.clone();
    if apply {
        if out.tables.misc != misc {
            io::save_txt(state, "misc.txt", &out.tables.misc)?;
        }
        if out.tables.cube != cube {
            io::save_txt(state, "cubemain.txt", &out.tables.cube)?;
        }
        if let (Some(a), Some(b)) = (&out.tables.levels, &levels)
            && a != b
        {
            io::save_txt(state, "levels.txt", a)?;
        }
        if let (Some(a), Some(b)) = (&out.tables.strings, &strings)
            && a.is_modified()
            && a != b
        {
            io::save_tbl(state, PATCH_STRINGS, a)?;
        }
        lines.push("Saved Misc.txt, CubeMain.txt and the strings that changed.".into());
    }
    Ok(lines)
}

pub fn cube(ui: &mut Ui, state: &mut AppState) {
    let mut g = std::mem::take(&mut state.tools.game);
    ui.label("Adds an item (a copy of a template from Misc.txt) and the Horadric Cube recipe that makes it.");
    egui::Grid::new("cube_form").num_columns(2).show(ui, |ui| {
        ui.label("Template item (code or name)");
        ui.add(egui::TextEdit::singleline(&mut g.cu_template).desired_width(200.0));
        ui.end_row();
        ui.label("New item name");
        ui.add(egui::TextEdit::singleline(&mut g.cu_name).desired_width(240.0));
        ui.end_row();
        ui.label("New item code");
        ui.add(egui::TextEdit::singleline(&mut g.cu_code).desired_width(80.0));
        ui.end_row();
        ui.label("Ingredients (comma separated)");
        ui.add(egui::TextEdit::singleline(&mut g.cu_inputs).desired_width(280.0));
        ui.end_row();
        ui.label("Items in the cube");
        ui.add(egui::DragValue::new(&mut g.cu_num).range(1..=7));
        ui.end_row();
        ui.label("Map the recipe is for");
        ui.add(egui::TextEdit::singleline(&mut g.cu_map_name).desired_width(240.0));
        ui.end_row();
        ui.label("Level it opens (map items)");
        ui.add(egui::TextEdit::singleline(&mut g.cu_level).desired_width(80.0));
        ui.end_row();
        ui.label("Mod item types");
        ui.checkbox(&mut g.cu_accept, "I know the template's type is fine");
        ui.end_row();
    });
    let form = CubeForm {
        template: g.cu_template.clone(),
        name: g.cu_name.clone(),
        code: g.cu_code.clone(),
        inputs: g
            .cu_inputs
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        numinputs: g.cu_num.max(1),
        map_name: g.cu_map_name.clone(),
        level: g.cu_level.trim().parse().ok(),
        accept: g.cu_accept,
    };
    ui.horizontal(|ui| {
        if ui.button("Preview").clicked() {
            match cube_run(state, &form, false) {
                Ok(l) => {
                    g.cu_lines = l;
                    g.cu_msg.clear();
                }
                Err(e) => g.cu_msg = e,
            }
        }
        if ui.button("Add the item and recipe").clicked() {
            match cube_run(state, &form, true) {
                Ok(l) => {
                    g.cu_lines = l;
                    g.cu_msg = "Saved.".into();
                }
                Err(e) => g.cu_msg = e,
            }
        }
    });
    super::message(ui, &g.cu_msg);
    for l in &g.cu_lines {
        ui.add(egui::Label::new(l.as_str()).wrap());
    }
    state.tools.game = g;
}

// ───────────────────────── custom object ─────────────────────────

/// Nearest palette index of an RGBA pixel (0 = transparent).
fn to_indices(rgba: &[u8], palette: &d2_formats::palette::Palette) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            if p[3] < 128 {
                return 0;
            }
            let mut best = (u32::MAX, 1u8);
            for (i, c) in palette.colors.iter().enumerate().skip(1) {
                let d = (0..3)
                    .map(|k| {
                        let x = i32::from(p[k]) - i32::from(c[k]);
                        (x * x) as u32
                    })
                    .sum::<u32>();
                if d < best.0 {
                    best = (d, i as u8);
                }
            }
            best.1
        })
        .collect()
}

pub fn custom_object_run(
    state: &mut AppState,
    row: usize,
    name: &str,
    png: &str,
    frames: u32,
    act: u8,
    blocks: bool,
) -> Result<String, String> {
    let objects = io::read_txt(state, "objects.txt")?;
    let used: BTreeSet<String> = BTreeSet::new();
    let token = free_token(&objects, &used.into_iter().collect()).map_err(|e| e.to_string())?;
    let opts = CustomObjectOptions {
        name: name.to_owned(),
        token: token.clone(),
        frames: frames.max(1),
        speed: 256,
        light: 0,
        light_colour: [255, 255, 255],
        flicker: false,
        blocks,
        size: 1,
        draw_under: false,
        act,
    };
    let out = custom_object_row(&objects, row, &opts).map_err(|e| e.to_string())?;
    let mut msg = vec![format!(
        "objects.txt row {row} becomes \"{name}\" (token {token})"
    )];
    if !png.trim().is_empty() {
        let img = image::open(png.trim())
            .map_err(|e| format!("cannot read {png}: {e}"))?
            .to_rgba8();
        let palette =
            super::dt1::load_palette(state, act).ok_or("the act palette could not be read")?;
        let (w, h) = (img.width(), img.height());
        let pixels = to_indices(img.as_raw(), &palette);
        let strip = capy_mod::custom_object::split_strip(w, h, &pixels, frames.max(1))
            .map_err(|e| e.to_string())?;
        let plan = custom_object_files(&token, &strip, 0, None).map_err(|e| e.to_string())?;
        io::save_game(state, &GamePath::new(&plan.dc6_path), &plan.dc6)?;
        io::save_game(state, &GamePath::new(&plan.cof.path), &plan.cof_bytes)?;
        msg.push(format!("wrote {} and {}", plan.dc6_path, plan.cof.path));
    }
    io::save_txt(state, "objects.txt", &out.objects)?;
    Ok(msg.join("; "))
}

pub fn custom_object(ui: &mut Ui, state: &mut AppState) {
    let mut g = std::mem::take(&mut state.tools.game);
    ui.label("Turns a free objects.txt row into a decoration drawn from your picture (frames side by side).");
    egui::Grid::new("custom_obj").num_columns(2).show(ui, |ui| {
        ui.label("objects.txt row");
        ui.add(egui::DragValue::new(&mut g.co_row).range(0..=2000));
        ui.end_row();
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut g.co_name).desired_width(240.0));
        ui.end_row();
        ui.label("Picture (PNG)");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut g.co_png).desired_width(240.0));
            if ui.button("Browse...").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .add_filter("png", &["png"])
                    .pick_file()
            {
                g.co_png = p.display().to_string();
            }
        });
        ui.end_row();
        ui.label("Frames");
        ui.add(egui::DragValue::new(&mut g.co_frames).range(1..=255));
        ui.end_row();
        ui.label("Act (palette)");
        ui.add(egui::DragValue::new(&mut g.co_act).range(0..=4));
        ui.end_row();
        ui.label("Blocks walking");
        ui.checkbox(&mut g.co_blocks, "");
        ui.end_row();
    });
    if ui.button("Make the object").clicked() {
        g.co_msg = custom_object_run(
            state,
            g.co_row,
            &g.co_name.clone(),
            &g.co_png.clone(),
            g.co_frames,
            g.co_act,
            g.co_blocks,
        )
        .unwrap_or_else(|e| e);
    }
    super::message(ui, &g.co_msg);
    state.tools.game = g;
}

/// Replaces the open map's model with `ds1` as one undoable step.
pub fn replace_map(
    state: &mut AppState,
    label: &str,
    ds1: d2_formats::ds1::Ds1,
) -> Result<(), String> {
    let doc = state.active_doc_mut().ok_or("No map is open")?;
    let before = doc.edit.ds1().clone();
    doc.edit
        .apply_patch(Patch::replace(label, before, ds1))
        .map_err(|e| e.to_string())?;
    doc.sync();
    Ok(())
}
