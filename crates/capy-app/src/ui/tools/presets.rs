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
use d2_assets::GamePath;

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
    super::message(ui, &p.msg);
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
