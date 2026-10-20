//! Presets: reusable blocks of cells. The library lives in the profile's
//! saves (`data\ds1studio\presets\<name>-<id>.json`, ds1-studio's format).
//! The right column's Presets tab lists and places them; the dialogs save the
//! selection as a preset, build one on a 20x20 scratch map, and suggest
//! presets from the structures that recur in the open map.

use bevy_egui::egui::{self, Color32, Sense, Ui};
use capy_edit::presets::{
    Preset, PresetBuilder, SuggestOptions, categories, sort_presets, suggest_presets,
};
use capy_edit::{CellValue, LayerRef};
use capy_mod::preset_package::{
    TileFile, build_preset_package, fill_sources, plan_preset_import, prepare_preset_library,
};
use capy_mod::tiles_review::append_tiles;
use capy_mod::tiles_review::keys_of;
use capy_mod::tiles_review::own_tiles_path;
use d2_assets::GamePath;
use d2_formats::dt1::Dt1;
use std::collections::BTreeSet;

use super::io;
use crate::edit::paste::PasteSession;
use crate::state::AppState;

const LIB_DIR: &str = "data\\ds1studio\\presets";
const BUILDER_CELL: f32 = 22.0;

type TopCell = Option<(LayerRef, CellValue)>;

#[derive(Default)]
pub struct PresetsUi {
    pub library: Vec<Preset>,
    loaded_for: Option<u64>,
    stale: bool,
    pub selected: Option<usize>,
    pub filter: String,
    pub category: String,
    // save dialog
    pub name: String,
    pub save_category: String,
    pub msg: String,
    // builder
    builder: Option<PresetBuilder>,
    pub trim: bool,
    // suggestions
    pub suggestions: Vec<Preset>,
}

fn saves_dir(state: &AppState) -> Option<std::path::PathBuf> {
    Some(io::saves_root(state)?.join(LIB_DIR))
}

/// Reads the library from the profile's saves.
pub fn load_library(state: &AppState) -> Vec<Preset> {
    let mut out = Vec::new();
    if let Some(dir) = saves_dir(state)
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            if e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
                && let Ok(text) = std::fs::read_to_string(e.path())
                && let Some(p) = Preset::from_json(&text)
            {
                out.push(p);
            }
        }
    }
    sort_presets(&mut out);
    out
}

fn refresh(state: &mut AppState) {
    let lib = load_library(state);
    let p = &mut state.tools.presets;
    p.library = lib;
    p.stale = false;
    p.loaded_for = Some(state.session_epoch);
}

/// Saves a preset into the library (saves + CHANGED.txt + snapshot).
pub fn save_preset(state: &mut AppState, preset: &Preset) -> Result<String, String> {
    let mut preset = preset.clone();
    // Record which DT1 each tile comes from (as the open map resolves it), so
    // the preset can be packaged and its tiles copied elsewhere.
    if preset.tile_sources.is_empty()
        && let Some(tiles) = io::doc_tiles(state)
    {
        fill_sources(&mut preset, &|id| {
            let r = tiles.resolve(d2_formats::dt1::TileKey {
                orientation: id.orientation as u8,
                main: id.main as u8,
                sub: id.sub as u8,
            })?;
            tiles
                .dt1_paths()
                .get(r.dt1)
                .map(|p| p.as_str().replace('\\', "/"))
        });
    }
    let path = GamePath::new(&preset.path());
    io::save_game(state, &path, preset.to_json().as_bytes())?;
    state.tools.presets.stale = true;
    Ok(path.as_str().to_owned())
}

/// Saves the active selection as a preset.
pub fn save_selection(state: &mut AppState, name: &str, category: &str) -> Result<String, String> {
    let preset = {
        let doc = state.active_doc().ok_or("No map is open")?;
        Preset::from_selection(&doc.edit, &doc.selection, name.trim(), category.trim())
            .ok_or("Select some cells first")?
    };
    save_preset(state, &preset)
}

/// Starts the floating paste preview with the preset (click to place; it is
/// one undo step like any paste).
pub fn place(state: &mut AppState, preset: &Preset) -> Result<(), String> {
    let doc = state.active_doc().ok_or("Open a map first")?;
    let clip = preset.to_clipboard(doc.edit.ds1());
    let at = state
        .hover_cell
        .unwrap_or_else(|| crate::iso::screen_to_cell(doc.view.center));
    state.edit.paste = Some(PasteSession::new(clip, at, state.edit.paste_mode, false));
    state.edit.paste_cache = None;
    Ok(())
}

/// Places the preset into the open map at once at `at` (undoable).
pub fn place_now(state: &mut AppState, preset: &Preset, at: (i32, i32)) -> Result<(), String> {
    let index = state.active.ok_or("Open a map first")?;
    let clip = preset.to_clipboard(state.docs[index].edit.ds1());
    let mode = state.edit.paste_mode;
    if state.exec(
        index,
        crate::edit::ops::EditOp::Paste {
            clip: Box::new(clip),
            at,
            mode,
        },
    ) {
        Ok(())
    } else {
        Err("The preset could not be placed there".into())
    }
}

/// Which presets an export covers.
pub enum ExportScope<'a> {
    /// Every preset of the library.
    All,
    /// Presets of one category.
    Category(&'a str),
    /// One preset, by name.
    Named(&'a str),
}

/// Writes a preset package (`.zip`) with the mod's own tile libraries the
/// presets use. The game's own DT1s are left out.
pub fn export_run(
    state: &mut AppState,
    scope: ExportScope<'_>,
    out: &str,
) -> Result<String, String> {
    let library = load_library(state);
    let chosen: Vec<Preset> = library
        .into_iter()
        .filter(|p| match scope {
            ExportScope::All => true,
            ExportScope::Category(c) => p.category == c,
            ExportScope::Named(n) => p.name == n,
        })
        .collect();
    if chosen.is_empty() {
        return Err("There is no preset to export".into());
    }
    let session = state.session.as_ref().ok_or("No profile is open yet")?;
    let overlay = session.overlay.clone();
    let locate = |p: &str| -> Option<TileFile> {
        let gp = GamePath::new(p);
        let (layer, bytes) = overlay.read(&gp).ok().flatten()?;
        let base_game =
            crate::state::source_info(&overlay, layer).category == crate::doc::SourceCategory::Base;
        Some(TileFile {
            bytes,
            base_game,
            path: gp.as_str().to_owned(),
        })
    };
    let built = build_preset_package(&chosen, &locate);
    let mut zip = capy_history::zip::ZipWriter::new();
    for (name, bytes) in &built.files {
        zip.add(name, bytes).map_err(|e| e.to_string())?;
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?;
    if let Some(parent) = std::path::Path::new(out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    capy_history::atomic_write(std::path::Path::new(out), &bytes)
        .map_err(|e| format!("cannot write {out}: {e}"))?;
    let mut msg = format!(
        "Exported {} preset(s) and {} tile librar{} to {out}",
        chosen.len(),
        built.dt1s.len(),
        if built.dt1s.len() == 1 { "y" } else { "ies" }
    );
    if !built.missing.is_empty() {
        msg.push_str(&format!("; not found: {}", built.missing.join(", ")));
    }
    Ok(msg)
}

/// Imports a preset package (`.zip`) or a single preset (`.json`): the tile
/// libraries it brings go into the saves (a different file under the same name
/// is kept and the import renamed), then the presets.
pub fn import_run(state: &mut AppState, path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path.trim()).map_err(|e| format!("cannot read {path}: {e}"))?;
    let entries = if bytes.starts_with(b"PK") {
        Some(capy_history::zip::read_zip(&bytes).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let existing = load_library(state);
    let overlay = state
        .session
        .as_ref()
        .ok_or("No profile is open yet")?
        .overlay
        .clone();
    let read = |p: &str| overlay.read_bytes(&GamePath::new(p)).ok().flatten();
    let plan = plan_preset_import(&bytes, entries.as_deref(), &existing, &read)
        .map_err(|e| e.to_string())?;
    for (p, data) in &plan.dt1_writes {
        io::save_game(state, &GamePath::new(p), data)?;
    }
    for p in &plan.presets {
        save_preset(state, p)?;
    }
    let mut msg = format!(
        "Imported {} preset(s), wrote {} tile librar{}",
        plan.presets.len(),
        plan.dt1_writes.len(),
        if plan.dt1_writes.len() == 1 {
            "y"
        } else {
            "ies"
        }
    );
    if !plan.duplicates.is_empty() {
        msg.push_str(&format!(
            "; {} already here unchanged",
            plan.duplicates.len()
        ));
    }
    if !plan.renamed.is_empty() {
        msg.push_str(&format!("; {} library renamed", plan.renamed.len()));
    }
    for p in &plan.problems {
        msg.push_str(&format!("; {p}"));
    }
    Ok(msg)
}

/// Copies the tiles the preset needs and the open map lacks into the map's
/// own DT1 (`<map>_custom.dt1`, appended to when it exists), lists that DT1 in
/// the map, and re-saves the preset pointing at it.
pub fn copy_tiles_run(state: &mut AppState, preset: &Preset) -> Result<String, String> {
    let (tiles, map_path) = {
        let doc = state.active_doc().ok_or("Open a map first")?;
        (
            io::doc_tiles(state).ok_or("The map's tiles could not be loaded")?,
            doc.path.as_str().to_owned(),
        )
    };
    let rel = own_tiles_path(&io::tiles_rel(&map_path), None, &[]);
    let path = io::tiles_game(&rel);
    let existing = io::read_game(state, &path)?.and_then(|b| Dt1::parse(&b).ok());
    let target = |id: capy_mod::tiles_review::TileId| -> Vec<String> {
        tiles
            .candidates(d2_formats::dt1::TileKey {
                orientation: id.orientation as u8,
                main: id.main as u8,
                sub: id.sub as u8,
            })
            .iter()
            .filter_map(|r| tiles.dt1_paths().get(r.dt1))
            .map(|p| p.as_str().replace('\\', "/"))
            .collect()
    };
    let mut taken: BTreeSet<capy_mod::tiles_review::TileId> = tiles
        .keys()
        .map(|(k, _)| {
            capy_mod::tiles_review::TileId::new(
                i32::from(k.orientation),
                i32::from(k.main),
                i32::from(k.sub),
            )
        })
        .collect();
    taken.extend(keys_of(existing.as_ref()));
    let overlay = state
        .session
        .as_ref()
        .ok_or("No profile is open yet")?
        .overlay
        .clone();
    let read = |p: &str| overlay.read_bytes(&GamePath::new(p)).ok().flatten();
    let own = path.as_str().replace('\\', "/");
    let out = prepare_preset_library(preset, &own, &target, &read, &BTreeSet::new(), &taken)
        .map_err(|e| e.to_string())?;
    let Some(new) = out.dt1 else {
        return Ok("The map already has every tile this preset needs".into());
    };
    let (dt1, _) = append_tiles(existing.as_ref(), new.tiles);
    let bytes = dt1
        .to_bytes()
        .map_err(|e| format!("cannot write the DT1: {e}"))?;
    io::save_game(state, &path, &bytes)?;
    // The map has to load the new library for the tiles to show.
    let mut ds1 = state
        .active_doc()
        .ok_or("Open a map first")?
        .edit
        .ds1()
        .clone();
    let add = capy_mod::compat::Edit::AddDt1s {
        paths: vec![rel.clone()],
    };
    if add.apply_ds1(&mut ds1) {
        super::game::replace_map(state, "Add the preset's tile library", ds1)?;
    }
    save_preset(state, &out.preset)?;
    // The game only loads DT1s that LvlTypes lists and the map's Dt1Mask selects.
    let mut msg = format!(
        "Copied {} tile(s) into {rel} ({} renumbered); the preset now uses it",
        out.plan.records.len(),
        out.plan.renumbered.len()
    );
    match io::register_for_open_map(state, &rel) {
        Ok(reg) => {
            for c in &reg.changes {
                msg.push_str(&format!("; {c}"));
            }
            for w in &reg.warnings {
                state.warn(w.clone());
                msg.push_str(&format!("; {w}"));
            }
        }
        Err(e) => {
            let w = format!("The game does not load {rel} yet: {e}");
            state.warn(w.clone());
            msg.push_str(&format!("; {w}"));
        }
    }
    Ok(msg)
}

fn delete_preset(state: &mut AppState, preset: &Preset) -> Result<(), String> {
    let path = GamePath::new(&preset.path());
    let root = io::saves_root(state).ok_or("No profile is open")?;
    let native = path.to_native(&root);
    std::fs::remove_file(&native)
        .map_err(|e| format!("cannot delete {}: {e}", native.display()))?;
    io::forget_changed(state, &[path]);
    state.tools.presets.stale = true;
    Ok(())
}

// ───────────────────────── the right column's tab ─────────────────────────

pub fn show(ui: &mut Ui, state: &mut AppState) {
    if state.tools.presets.loaded_for != Some(state.session_epoch) || state.tools.presets.stale {
        refresh(state);
    }
    let mut p = std::mem::take(&mut state.tools.presets);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut p.filter)
                .hint_text("filter")
                .desired_width(110.0),
        );
        let cats = categories(&p.library);
        egui::ComboBox::from_id_salt("preset_cat")
            .selected_text(if p.category.is_empty() {
                "All"
            } else {
                p.category.as_str()
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.category, String::new(), "All");
                for c in cats {
                    ui.selectable_value(&mut p.category, c.clone(), c);
                }
            });
    });
    ui.horizontal_wrapped(|ui| {
        for (id, label) in [
            ("presets.save", "Save selection..."),
            ("presets.builder", "Builder..."),
            ("presets.suggest", "Suggest..."),
        ] {
            let on = super::enabled(state, id).unwrap_or(false);
            let resp = ui.add_enabled(on, egui::Button::new(label));
            state.shell.ribbon_rects.push((id, resp.rect, on));
            if resp.clicked() {
                state.shell.clicked.push(id);
                super::run(state, id);
            }
        }
    });
    ui.separator();
    let f = p.filter.to_ascii_lowercase();
    let mut place_idx: Option<usize> = None;
    let mut delete_idx: Option<usize> = None;
    egui::ScrollArea::vertical()
        .id_salt("preset_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, pr) in p.library.iter().enumerate() {
                if !f.is_empty() && !pr.name.to_ascii_lowercase().contains(&f) {
                    continue;
                }
                if !p.category.is_empty() && pr.category != p.category {
                    continue;
                }
                let on = p.selected == Some(i);
                let resp = ui
                    .selectable_label(
                        on,
                        format!("{} ({}x{}, {})", pr.name, pr.width, pr.height, pr.category),
                    )
                    .on_hover_text(pr.dt1s.join("\n"));
                if resp.clicked() {
                    p.selected = Some(i);
                }
                if resp.double_clicked() {
                    place_idx = Some(i);
                }
            }
        });
    ui.horizontal(|ui| {
        let sel = p.selected.filter(|i| *i < p.library.len());
        if ui
            .add_enabled(
                sel.is_some() && state.active_doc().is_some(),
                egui::Button::new("Place"),
            )
            .on_hover_text("Shows the preset under the mouse; click the map to place it")
            .clicked()
        {
            place_idx = sel;
        }
        if ui
            .add_enabled(sel.is_some(), egui::Button::new("Delete"))
            .clicked()
        {
            delete_idx = sel;
        }
    });
    let mut export = false;
    let mut import = false;
    let mut copy_tiles = false;
    ui.horizontal(|ui| {
        let sel = p.selected.filter(|i| *i < p.library.len());
        export = ui
            .add_enabled(!p.library.is_empty(), egui::Button::new("Export..."))
            .on_hover_text(
                "Saves a .zip with the selected preset (or the chosen category, or all of them) \
                 and the mod's own tile libraries they use",
            )
            .clicked();
        import = ui
            .button("Import...")
            .on_hover_text(
                "Adds the presets (and their tile libraries) from a package or a preset file",
            )
            .clicked();
        copy_tiles = ui
            .add_enabled(
                sel.is_some() && state.active_doc().is_some(),
                egui::Button::new("Copy tiles to DT1"),
            )
            .on_hover_text(
                "Copies the tiles this preset needs and the open map lacks into the map's own DT1",
            )
            .clicked();
    });
    super::message(ui, &p.msg);
    if export {
        let name = match p.selected.and_then(|i| p.library.get(i)) {
            Some(pr) => pr.name.clone(),
            None if !p.category.is_empty() => p.category.clone(),
            None => "presets".to_owned(),
        };
        if let Some(out) = rfd::FileDialog::new()
            .add_filter("preset package", &["zip"])
            .set_file_name(format!("{name}.zip"))
            .save_file()
        {
            let sel_name = p
                .selected
                .and_then(|i| p.library.get(i))
                .map(|pr| pr.name.clone());
            let category = p.category.clone();
            let scope = match (&sel_name, category.is_empty()) {
                (Some(n), _) => ExportScope::Named(n),
                (None, false) => ExportScope::Category(&category),
                (None, true) => ExportScope::All,
            };
            p.msg = export_run(state, scope, &out.display().to_string()).unwrap_or_else(|e| e);
        }
    }
    if import
        && let Some(file) = rfd::FileDialog::new()
            .add_filter("preset package or preset", &["zip", "json"])
            .pick_file()
    {
        p.msg = import_run(state, &file.display().to_string()).unwrap_or_else(|e| e);
        p.stale = true;
    }
    if copy_tiles && let Some(pr) = p.selected.and_then(|i| p.library.get(i)).cloned() {
        p.msg = copy_tiles_run(state, &pr).unwrap_or_else(|e| e);
        p.stale = true;
    }
    if let Some(i) = place_idx
        && let Some(pr) = p.library.get(i).cloned()
    {
        p.msg = place(state, &pr).err().unwrap_or_default();
    }
    if let Some(i) = delete_idx
        && let Some(pr) = p.library.get(i).cloned()
    {
        p.msg = delete_preset(state, &pr)
            .err()
            .unwrap_or_else(|| format!("Deleted {}", pr.name));
        p.selected = None;
    }
    state.tools.presets = p;
}

// ───────────────────────── save dialog ─────────────────────────

pub fn save_dialog(ui: &mut Ui, state: &mut AppState) {
    let mut p = std::mem::take(&mut state.tools.presets);
    let cells = state.active_doc().map_or(0, |d| d.selection.len());
    ui.label(format!("{cells} selected cell(s) become a preset."));
    egui::Grid::new("preset_save")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(220.0));
            ui.end_row();
            ui.label("Category");
            ui.add(egui::TextEdit::singleline(&mut p.save_category).desired_width(220.0));
            ui.end_row();
        });
    if ui
        .add_enabled(
            cells > 0 && !p.name.trim().is_empty(),
            egui::Button::new("Save preset"),
        )
        .clicked()
    {
        let (n, c) = (p.name.clone(), p.save_category.clone());
        state.tools.presets = std::mem::take(&mut p);
        let r = save_selection(state, &n, &c);
        p = std::mem::take(&mut state.tools.presets);
        p.msg = match r {
            Ok(path) => format!("Saved {path}"),
            Err(e) => e,
        };
    }
    super::message(ui, &p.msg);
    state.tools.presets = p;
}

// ───────────────────────── builder ─────────────────────────

fn top_value(
    ds1: &d2_formats::ds1::Ds1,
) -> impl Fn(u32, u32) -> Option<(LayerRef, CellValue)> + '_ {
    move |x, y| {
        let mut found = None;
        for l in LayerRef::all_in(ds1) {
            if matches!(l, LayerRef::Tag) {
                continue;
            }
            if let Some(v) = l.get(ds1, x, y)
                && v != CellValue::EMPTY
            {
                found = Some((l, v));
            }
        }
        found
    }
}

pub fn builder(ui: &mut Ui, state: &mut AppState) {
    let mut p = std::mem::take(&mut state.tools.presets);
    if p.builder.is_none() {
        let act = state.active_doc().map_or(0, |d| d.edit.ds1().act);
        p.builder = PresetBuilder::new(act).ok();
    }
    let Some(mut b) = p.builder.take() else {
        ui.label("The builder map could not be made.");
        state.tools.presets = p;
        return;
    };
    let brush = state.edit.current_layer.map(|l| (l, state.edit.current));
    ui.label("Paint a 20x20 block with the tile picked in the Tiles tab: left click paints, right click erases.");
    ui.horizontal(|ui| {
        if ui
            .add_enabled(b.document().can_undo(), egui::Button::new("Undo"))
            .clicked()
        {
            b.document_mut().undo();
        }
        if ui
            .add_enabled(b.document().can_redo(), egui::Button::new("Redo"))
            .clicked()
        {
            b.document_mut().redo();
        }
        if ui.button("Clear").clicked() {
            let act = state.active_doc().map_or(0, |d| d.edit.ds1().act);
            if let Ok(nb) = PresetBuilder::new(act) {
                b = nb;
            }
        }
        ui.checkbox(&mut p.trim, "crop to what is drawn");
    });
    let size = BUILDER_CELL * 20.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(24));
    let cells: Vec<(u32, u32, TopCell)> = {
        let top = top_value(b.document().ds1());
        (0..20u32)
            .flat_map(|y| (0..20u32).map(move |x| (x, y)))
            .map(|(x, y)| (x, y, top(x, y)))
            .collect()
    };
    for (x, y, v) in cells {
        let r = egui::Rect::from_min_size(
            rect.min + egui::vec2(x as f32, y as f32) * BUILDER_CELL,
            egui::vec2(BUILDER_CELL, BUILDER_CELL),
        );
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(0.5, Color32::from_gray(50)),
            egui::StrokeKind::Inside,
        );
        if let Some((l, value)) = v {
            match crate::ui::thumbs::thumbnail(ui.ctx(), state, l, value) {
                Some(t) => {
                    painter.image(
                        t.id(),
                        r.shrink(1.0),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    painter.rect_filled(r.shrink(3.0), 2.0, Color32::from_rgb(110, 90, 160));
                }
            }
        }
    }
    let primary = resp.dragged_by(egui::PointerButton::Primary) || resp.clicked();
    let secondary = resp.dragged_by(egui::PointerButton::Secondary) || resp.secondary_clicked();
    if (primary || secondary)
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let c = ((pos - rect.min) / BUILDER_CELL).floor();
        let (x, y) = (c.x as i64, c.y as i64);
        if (0..20).contains(&x) && (0..20).contains(&y) {
            let (x, y) = (x as u32, y as u32);
            if secondary {
                for l in LayerRef::all_in(b.document().ds1()) {
                    if !matches!(l, LayerRef::Tag) {
                        let _ = b.document_mut().paint(l, &[(x, y, CellValue::EMPTY)]);
                    }
                }
            } else if let Some((l, v)) = brush {
                let _ = b.document_mut().paint(l, &[(x, y, v)]);
            }
        }
    }
    egui::Grid::new("preset_build")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(220.0));
            ui.end_row();
            ui.label("Category");
            ui.add(egui::TextEdit::singleline(&mut p.save_category).desired_width(220.0));
            ui.end_row();
        });
    let preset = b.build(p.name.trim(), p.save_category.trim(), p.trim);
    ui.horizontal(|ui| {
        let can = preset.is_some() && !p.name.trim().is_empty();
        if ui
            .add_enabled(can, egui::Button::new("Save to the library"))
            .clicked()
            && let Some(pr) = &preset
        {
            state.tools.presets = std::mem::take(&mut p);
            let r = save_preset(state, pr);
            p = std::mem::take(&mut state.tools.presets);
            p.msg = r.map_or_else(|e| e, |path| format!("Saved {path}"));
        }
        if ui
            .add_enabled(
                preset.is_some() && state.active_doc().is_some(),
                egui::Button::new("Place on the map"),
            )
            .clicked()
            && let Some(pr) = &preset
        {
            p.msg = place(state, pr).err().unwrap_or_default();
        }
    });
    super::message(ui, &p.msg);
    p.builder = Some(b);
    state.tools.presets = p;
}

// ───────────────────────── suggestions ─────────────────────────

/// Presets suggested by the structures that recur in the open map.
pub fn suggestions(state: &AppState) -> Result<Vec<Preset>, String> {
    let doc = state.active_doc().ok_or("Open a map first")?;
    let maps = [(doc.path.as_str().to_owned(), doc.edit.ds1())];
    Ok(suggest_presets(&maps, &SuggestOptions::default()))
}

pub fn suggest(ui: &mut Ui, state: &mut AppState) {
    let mut p = std::mem::take(&mut state.tools.presets);
    ui.label("Wall structures that appear more than once in the open map, offered as presets.");
    if ui.button("Find suggestions").clicked() {
        match suggestions(state) {
            Ok(s) => {
                p.msg = format!("{} suggestion(s)", s.len());
                p.suggestions = s;
            }
            Err(e) => p.msg = e,
        }
    }
    super::message(ui, &p.msg);
    let mut save_idx = None;
    egui::ScrollArea::vertical()
        .id_salt("preset_sug")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, s) in p.suggestions.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} ({}x{}, seen {} time(s))",
                        s.name,
                        s.width,
                        s.height,
                        s.occurrences.unwrap_or(1)
                    ));
                    if ui.small_button("Save").clicked() {
                        save_idx = Some(i);
                    }
                });
            }
        });
    if let Some(i) = save_idx
        && let Some(s) = p.suggestions.get(i).cloned()
    {
        state.tools.presets = std::mem::take(&mut p);
        let r = save_preset(state, &s);
        p = std::mem::take(&mut state.tools.presets);
        p.msg = r.map_or_else(|e| e, |path| format!("Saved {path}"));
    }
    state.tools.presets = p;
}
