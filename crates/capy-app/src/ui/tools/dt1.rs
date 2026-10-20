//! Tile libraries (P11): the DT1 manager (open, duplicate, rename,
//! recolour, pixel painting), tile settings (.ini), sub-tile collision
//! flags and Make act-safe. One library is open at a time, shared by the
//! windows; every change is undoable in the session and written only to
//! `capybara\saves\<profile>\` (+ CHANGED.txt + a history snapshot).

use std::collections::{BTreeSet, HashMap};

use bevy_egui::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use capy_mod::tiles_actsafe::{ActSafeOptions, Safe, count_unsafe_pixels, make_act_safe};
use capy_mod::tiles_dt1::{
    HueOptions, TileChange, duplicate_dt1, hue_remap, plan_rename, recolor_tiles,
    rename_references, set_block_pixels,
};
use capy_mod::tiles_flags::{
    COLLISION_FLAGS, FlagOp, WALKABLE_MASK, collision_hex, describe_collision, edit_subtile_flags,
};
use capy_mod::tiles_settings::{
    TileSettingsPatch, apply_ini, dt1_to_ini, parse_dt1_ini, read_settings, write_settings,
};
use capy_paint::{Brush, BrushShape, FrameRef, Paint, PaintSession};
use d2_assets::GamePath;
use d2_formats::IndexedImage;
use d2_formats::dt1::{BlockFormat, Dt1};
use d2_formats::palette::{Palette, act_palette_path};

use super::io;
use crate::state::AppState;

const MAX_UNDO: usize = 12;

/// The open library.
pub struct Lib {
    pub path: String,
    pub dt1: Dt1,
    pub dirty: bool,
    undo: Vec<Dt1>,
    redo: Vec<Dt1>,
    pub tile: usize,
    pub picked: BTreeSet<usize>,
    /// Bumped on every change (thumbnails are redrawn).
    pub rev: u64,
    tex: HashMap<usize, Option<TextureHandle>>,
    tex_rev: u64,
}

impl Lib {
    pub fn new(path: String, dt1: Dt1) -> Lib {
        Lib {
            path,
            dt1,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            tile: 0,
            picked: BTreeSet::new(),
            rev: 1,
            tex: HashMap::new(),
            tex_rev: 0,
        }
    }

    /// Run an edit that is one undo step. An edit that fails leaves the
    /// library as it was.
    pub fn edit<T>(&mut self, f: impl FnOnce(&mut Dt1) -> Result<T, String>) -> Result<T, String> {
        let before = self.dt1.clone();
        match f(&mut self.dt1) {
            Ok(v) => {
                if self.dt1 != before {
                    self.undo.push(before);
                    if self.undo.len() > MAX_UNDO {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                    self.dirty = true;
                    self.rev += 1;
                }
                Ok(v)
            }
            Err(e) => {
                self.dt1 = before;
                Err(e)
            }
        }
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.dt1, prev));
                self.dirty = true;
                self.rev += 1;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.dt1, next));
                self.dirty = true;
                self.rev += 1;
                true
            }
            None => false,
        }
    }

    /// The selected tiles (the clicked one when none is ticked).
    pub fn targets(&self) -> Vec<usize> {
        if self.picked.is_empty() {
            vec![self.tile]
        } else {
            self.picked.iter().copied().collect()
        }
    }

    fn texture(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        i: usize,
    ) -> Option<TextureHandle> {
        if self.tex_rev != self.rev {
            self.tex.clear();
            self.tex_rev = self.rev;
        }
        if let Some(t) = self.tex.get(&i) {
            return t.clone();
        }
        let made = self
            .dt1
            .tiles
            .get(i)
            .and_then(|t| t.render().ok())
            .and_then(|img| {
                texture(
                    ctx,
                    &img,
                    palette,
                    &format!("dt1-{}-{i}-{}", self.path, self.rev),
                )
            });
        self.tex.insert(i, made.clone());
        made
    }
}

pub fn texture(
    ctx: &egui::Context,
    img: &IndexedImage,
    palette: &Palette,
    name: &str,
) -> Option<TextureHandle> {
    if img.width == 0 || img.height == 0 {
        return None;
    }
    let rgba = img.to_rgba(palette);
    let color =
        ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &rgba);
    Some(ctx.load_texture(name.to_owned(), color, TextureOptions::NEAREST))
}

/// Which act palette the previews use.
pub fn load_palette(state: &AppState, act: u8) -> Option<Palette> {
    let bytes = io::read_game(state, &GamePath::new(act_palette_path(act)))
        .ok()
        .flatten()?;
    Palette::parse_dat(&bytes).ok()
}

#[derive(Default)]
pub struct PixelPaint {
    tile: usize,
    block: usize,
    session: Option<PaintSession>,
    tool: u8,
    color: u8,
    tex: Option<(u64, TextureHandle)>,
    mask: Option<capy_paint::Mask>,
}

pub struct Dt1Ui {
    pub lib: Option<Lib>,
    pub act: u8,
    palette: Option<(u8, Palette)>,
    pub filter: String,
    pub path_input: String,
    candidates: Option<Vec<String>>,
    pub status: String,
    // manager
    pub copy_name: String,
    pub rename_to: String,
    pub hue: f32,
    pub sat: f32,
    pub bright: f32,
    pub tint_on: bool,
    pub tint: [u8; 3],
    pub tint_amount: f32,
    // settings
    pub ini_path: String,
    // flags
    pub flag_bits: u8,
    pub flag_op: usize,
    // act-safe
    pub home_act: u8,
    pub distance: usize,
    pub report: String,
    paint: PixelPaint,
}

impl Default for Dt1Ui {
    fn default() -> Self {
        Dt1Ui {
            lib: None,
            act: 0,
            palette: None,
            filter: String::new(),
            path_input: String::new(),
            candidates: None,
            status: String::new(),
            copy_name: String::new(),
            rename_to: String::new(),
            hue: 0.0,
            sat: 1.0,
            bright: 1.0,
            tint_on: false,
            tint: [200, 60, 60],
            tint_amount: 0.3,
            ini_path: String::new(),
            flag_bits: 0x01,
            flag_op: 0,
            home_act: 0,
            distance: 0,
            report: String::new(),
            paint: PixelPaint::default(),
        }
    }
}

impl Dt1Ui {
    fn palette(&mut self, state: &AppState) -> Option<Palette> {
        if self.palette.as_ref().map(|p| p.0) != Some(self.act) {
            let loaded = load_palette(state, self.act)?;
            self.palette = Some((self.act, loaded));
        }
        self.palette.as_ref().map(|p| p.1.clone())
    }
}

/// Every DT1 the profile can name: the open map's, the saved ones and the
/// ones LvlTypes lists.
fn candidates(state: &AppState) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    if let Some(info) = state.active_doc().and_then(|d| d.info.as_ref()) {
        set.extend(info.dt1_files.iter().map(|p| p.as_str().to_owned()));
    }
    for f in io::saved_files(state) {
        if f.ends_with(".dt1") {
            set.insert(f);
        }
    }
    if let Ok(types) = io::read_txt(state, "lvltypes.txt") {
        for r in 0..types.row_count() {
            for c in 0..types.headers().len() {
                let h = &types.headers()[c];
                if h.to_ascii_lowercase().starts_with("file")
                    && let Some(v) = types.cell(r, c)
                    && let Some(p) = GamePath::from_tile_ref(&v)
                    && p.as_str().ends_with(".dt1")
                {
                    set.insert(p.as_str().to_owned());
                }
            }
        }
    }
    set.into_iter().collect()
}

pub fn open_lib(state: &mut AppState, path: &str) -> Result<(), String> {
    let game = GamePath::new(path);
    let bytes = io::read_game(state, &game)?.ok_or_else(|| format!("{game} was not found"))?;
    let dt1 = Dt1::parse(&bytes).map_err(|e| format!("not a DT1 this editor reads: {e}"))?;
    let act = state
        .active_doc()
        .and_then(|d| d.info.as_ref())
        .map_or(0, |i| i.palette_act);
    let mut ui = std::mem::take(&mut state.tools.dt1);
    ui.act = act;
    ui.copy_name = default_copy_name(game.as_str());
    ui.rename_to.clear();
    ui.paint = PixelPaint::default();
    ui.lib = Some(Lib::new(game.as_str().to_owned(), dt1));
    ui.status = format!("Opened {game}");
    state.tools.dt1 = ui;
    Ok(())
}

fn default_copy_name(path: &str) -> String {
    let stem = path
        .rsplit('\\')
        .next()
        .and_then(|f| f.strip_suffix(".dt1"))
        .unwrap_or("copy");
    format!("{stem}_copy")
}

/// Writes the open library to the saves under its own path.
pub fn save_lib(state: &mut AppState) -> Result<(), String> {
    let mut ui = std::mem::take(&mut state.tools.dt1);
    let result: Result<String, String> = (|| {
        let lib = ui.lib.as_mut().ok_or("No library is open")?;
        let bytes = lib
            .dt1
            .to_bytes()
            .map_err(|e| format!("cannot write the DT1: {e}"))?;
        io::save_game(state, &GamePath::new(&lib.path), &bytes)?;
        lib.dirty = false;
        Ok(format!("Saved {}", lib.path))
    })();
    match &result {
        Ok(m) => ui.status = m.clone(),
        Err(e) => ui.status = e.clone(),
    }
    state.tools.dt1 = ui;
    result.map(|_| ())
}

fn hue_options(ui: &Dt1Ui) -> HueOptions {
    HueOptions {
        hue: ui.hue,
        saturation: ui.sat,
        brightness: ui.bright,
        tint: ui.tint_on.then_some((ui.tint, ui.tint_amount)),
        rgb_metric: false,
        allowed: None,
    }
}

/// Recolour the selected tiles (all with `all`) by the manager's sliders.
pub fn recolor(state: &mut AppState, all: bool) -> Result<u64, String> {
    let mut ui = std::mem::take(&mut state.tools.dt1);
    let result: Result<u64, String> = (|| {
        let palette = ui
            .palette(state)
            .ok_or("the act palette could not be read")?;
        let opts = hue_options(&ui);
        let lib = ui.lib.as_mut().ok_or("No library is open")?;
        let table = hue_remap(&palette, &opts);
        let targets = lib.targets();
        let sel: Option<&[usize]> = if all { None } else { Some(&targets) };
        let changes: Vec<TileChange> =
            lib.edit(|d| recolor_tiles(d, sel, &table).map_err(|e| e.to_string()))?;
        Ok(changes.iter().map(|c| c.pixels).sum::<u64>())
    })();
    match &result {
        Ok(n) => ui.status = format!("Recoloured {n} pixel(s)"),
        Err(e) => ui.status = e.clone(),
    }
    state.tools.dt1 = ui;
    result
}

/// Save the open library as a copy named `name` (same folder); the copy
/// becomes the open library.
pub fn save_copy(state: &mut AppState, name: &str) -> Result<(), String> {
    let (path, bytes) = {
        let lib = state.tools.dt1.lib.as_ref().ok_or("No library is open")?;
        let copy = duplicate_dt1(&lib.dt1);
        let plan = plan_rename(&lib.path, name).map_err(|e| e.to_string())?;
        (
            GamePath::new(&plan.to),
            copy.to_bytes().map_err(|e| e.to_string())?,
        )
    };
    io::save_game(state, &path, &bytes)?;
    open_lib(state, path.as_str())
}

/// Rename: write the library under the new name and point LvlTypes /
/// LvlPrest at it. The old file stays (game files cannot be removed).
pub fn rename(state: &mut AppState, stem: &str) -> Result<usize, String> {
    let (plan, bytes) = {
        let lib = state.tools.dt1.lib.as_ref().ok_or("No library is open")?;
        let plan = plan_rename(&lib.path, stem).map_err(|e| e.to_string())?;
        (plan, lib.dt1.to_bytes().map_err(|e| e.to_string())?)
    };
    let mut tables = io::level_tables(state)?;
    let mut touched = BTreeSet::new();
    let mut count = 0;
    for (doc, which, prefix) in [
        (
            &mut tables.types,
            capy_mod::level_tables::TableName::LvlTypes,
            "file",
        ),
        (
            &mut tables.prest,
            capy_mod::level_tables::TableName::LvlPrest,
            "file",
        ),
    ] {
        let cols: Vec<usize> = doc
            .headers()
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                let h = h.to_ascii_lowercase();
                h.starts_with(prefix) && h[prefix.len()..].chars().all(|c| c.is_ascii_digit())
            })
            .map(|(i, _)| i)
            .collect();
        for r in 0..doc.row_count() {
            let mut cells: Vec<String> = cols
                .iter()
                .map(|&c| doc.cell(r, c).unwrap_or_default())
                .collect();
            let changed = rename_references(&mut cells, &plan.from_rel, &plan.to_rel);
            for i in changed {
                doc.set_cell(r, cols[i], &cells[i])
                    .map_err(|e| e.to_string())?;
                touched.insert(which);
                count += 1;
            }
        }
    }
    io::save_game(state, &GamePath::new(&plan.to), &bytes)?;
    io::save_level_tables(state, &tables, &touched)?;
    open_lib(state, &plan.to)?;
    Ok(count)
}

/// A tile-library window opened: refresh the list of libraries.
pub fn on_open(state: &mut AppState) {
    state.tools.dt1.candidates = None;
}

// ───────────────────────── windows ─────────────────────────

/// The library chooser and the open library's header (shared by the windows).
fn chooser(ui: &mut Ui, state: &mut AppState, d: &mut Dt1Ui) {
    if d.candidates.is_none() {
        d.candidates = Some(candidates(state));
    }
    ui.horizontal(|ui| {
        ui.label("Library:");
        let name = d.lib.as_ref().map_or("(none open)", |l| l.path.as_str());
        egui::ComboBox::from_id_salt("dt1_pick")
            .width(360.0)
            .selected_text(name.trim_start_matches("data\\global\\tiles\\"))
            .show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut d.filter).hint_text("filter"));
                let f = d.filter.to_ascii_lowercase();
                let mut choose = None;
                for p in d
                    .candidates
                    .iter()
                    .flatten()
                    .filter(|p| f.is_empty() || p.contains(&f))
                    .take(300)
                {
                    if ui.selectable_label(false, p.as_str()).clicked() {
                        choose = Some(p.clone());
                    }
                }
                if let Some(p) = choose {
                    // Opened after the combo closes (needs the whole state).
                    d.path_input = p;
                    d.status = "open".into();
                }
            });
        if ui.small_button("Refresh list").clicked() {
            d.candidates = None;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Game path:");
        ui.add(egui::TextEdit::singleline(&mut d.path_input).desired_width(360.0));
        if ui.button("Open").clicked() || d.status == "open" {
            d.status.clear();
            let p = d.path_input.clone();
            let mut taken = std::mem::take(d);
            // `open_lib` takes the UI out of the state itself.
            std::mem::swap(&mut state.tools.dt1, &mut taken);
            let r = open_lib(state, &p);
            let keep = std::mem::take(&mut state.tools.dt1);
            *d = keep;
            if let Err(e) = r {
                d.status = e;
            }
        }
    });
}

fn status_line(ui: &mut Ui, d: &Dt1Ui) {
    if !d.status.is_empty() && d.status != "open" {
        ui.weak(&d.status);
    }
}

fn lib_header(ui: &mut Ui, state: &mut AppState, d: &mut Dt1Ui) {
    let Some(lib) = d.lib.as_mut() else {
        ui.label("Open a library to start.");
        return;
    };
    let mut save = false;
    ui.horizontal(|ui| {
        ui.strong(&lib.path);
        ui.label(format!(
            "{} tiles{}",
            lib.dt1.tiles.len(),
            if lib.dirty { " - unsaved" } else { "" }
        ));
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!lib.undo.is_empty(), egui::Button::new("Undo"))
            .clicked()
        {
            lib.undo();
        }
        if ui
            .add_enabled(!lib.redo.is_empty(), egui::Button::new("Redo"))
            .clicked()
        {
            lib.redo();
        }
        save = ui
            .add_enabled(lib.dirty, egui::Button::new("Save to the saves folder"))
            .clicked();
    });
    if save {
        let mut keep = std::mem::take(d);
        std::mem::swap(&mut state.tools.dt1, &mut keep);
        let _ = save_lib(state);
        *d = std::mem::take(&mut state.tools.dt1);
    }
    ui.horizontal(|ui| {
        ui.label("Palette:");
        let before = d.act;
        egui::ComboBox::from_id_salt("dt1_act")
            .selected_text(format!("Act {}", d.act + 1))
            .show_ui(ui, |ui| {
                for a in 0..5u8 {
                    ui.selectable_value(&mut d.act, a, format!("Act {}", a + 1));
                }
            });
        if before != d.act {
            d.palette = None;
        }
    });
}

/// The thumbnail grid; click selects, Ctrl+click ticks.
fn tile_grid(ui: &mut Ui, d: &mut Dt1Ui, state: &AppState, height: f32) {
    let palette = d.palette(state);
    let Some(lib) = d.lib.as_mut() else { return };
    let Some(palette) = palette else {
        ui.weak("The act palette could not be read.");
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("dt1_grid")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for i in 0..lib.dt1.tiles.len() {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, 3.0, Color32::from_gray(34));
                        if let Some(t) = lib.texture(ui.ctx(), &palette, i) {
                            let ts = t.size_vec2();
                            let scale = (68.0 / ts.x).min(68.0 / ts.y).min(2.0);
                            painter.image(
                                t.id(),
                                egui::Rect::from_center_size(rect.center(), ts * scale),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        }
                        let t = &lib.dt1.tiles[i];
                        painter.text(
                            rect.left_bottom() + egui::vec2(3.0, -2.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{}:{}/{}", t.orientation, t.main_index, t.sub_index),
                            egui::FontId::monospace(9.0),
                            Color32::from_gray(190),
                        );
                        let stroke = if i == lib.tile {
                            egui::Stroke::new(2.0, Color32::from_rgb(255, 220, 90))
                        } else if lib.picked.contains(&i) {
                            egui::Stroke::new(2.0, Color32::from_rgb(90, 170, 255))
                        } else {
                            egui::Stroke::new(1.0, Color32::from_gray(70))
                        };
                        painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        if ui.input(|inp| inp.modifiers.command) {
                            if !lib.picked.remove(&i) {
                                lib.picked.insert(i);
                            }
                        } else {
                            lib.picked.clear();
                        }
                        lib.tile = i;
                    }
                }
            });
        });
}

pub fn manager(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.dt1);
    chooser(ui, state, &mut d);
    status_line(ui, &d);
    ui.separator();
    lib_header(ui, state, &mut d);
    if d.lib.is_some() {
        ui.separator();
        ui.collapsing("Recolour", |ui| {
            egui::Grid::new("dt1_recolour")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Hue");
                    ui.add(egui::Slider::new(&mut d.hue, -180.0..=180.0).suffix(" deg"));
                    ui.end_row();
                    ui.label("Saturation");
                    ui.add(egui::Slider::new(&mut d.sat, 0.0..=2.0));
                    ui.end_row();
                    ui.label("Brightness");
                    ui.add(egui::Slider::new(&mut d.bright, 0.2..=2.0));
                    ui.end_row();
                    ui.label("Tint");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut d.tint_on, "");
                        ui.color_edit_button_srgb(&mut d.tint);
                        ui.add(egui::Slider::new(&mut d.tint_amount, 0.0..=1.0));
                    });
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                let sel = d.lib.as_ref().map_or(0, |l| l.targets().len());
                if ui
                    .button(format!("Recolour {sel} selected tile(s)"))
                    .clicked()
                {
                    state.tools.dt1 = std::mem::take(&mut d);
                    let _ = recolor(state, false);
                    d = std::mem::take(&mut state.tools.dt1);
                }
                if ui.button("Recolour every tile").clicked() {
                    state.tools.dt1 = std::mem::take(&mut d);
                    let _ = recolor(state, true);
                    d = std::mem::take(&mut state.tools.dt1);
                }
            });
            ui.weak("Pixels keep palette indices 1..255; index 0 stays transparent.");
        });
        ui.collapsing("Duplicate / rename", |ui| {
            ui.horizontal(|ui| {
                ui.label("Save a copy as");
                ui.add(egui::TextEdit::singleline(&mut d.copy_name).desired_width(160.0));
                if ui.button("Duplicate").clicked() {
                    let name = d.copy_name.clone();
                    state.tools.dt1 = std::mem::take(&mut d);
                    let r = save_copy(state, &name);
                    d = std::mem::take(&mut state.tools.dt1);
                    d.status = r.err().unwrap_or_else(|| format!("Saved the copy {name}"));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Rename to");
                ui.add(egui::TextEdit::singleline(&mut d.rename_to).desired_width(160.0));
                if ui.button("Rename").clicked() {
                    let name = d.rename_to.clone();
                    state.tools.dt1 = std::mem::take(&mut d);
                    let r = rename(state, &name);
                    d = std::mem::take(&mut state.tools.dt1);
                    d.status = match r {
                        Ok(n) => format!("Renamed to {name}; {n} table reference(s) updated"),
                        Err(e) => e,
                    };
                }
            });
            ui.weak("Rename writes the new file and points LvlTypes / LvlPrest at it; the old file is left alone.");
        });
        ui.separator();
        let h = ui.available_height().max(120.0);
        tile_grid(ui, &mut d, state, h);
    }
    state.tools.dt1 = d;
}

pub fn settings(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.dt1);
    chooser(ui, state, &mut d);
    lib_header(ui, state, &mut d);
    status_line(ui, &d);
    if let Some(lib) = d.lib.as_mut() {
        ui.separator();
        let tile = lib.tile.min(lib.dt1.tiles.len().saturating_sub(1));
        if let Some(t) = lib.dt1.tiles.get(tile) {
            let s = read_settings(t);
            let mut v = (
                s.direction,
                i32::from(s.roof_height),
                i32::from(s.sound),
                s.animated,
                s.orientation,
                s.main_index,
                s.sub_index,
            );
            ui.label(format!("Tile {tile} of {}", lib.dt1.tiles.len()));
            let before = v;
            egui::Grid::new("dt1_settings")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Direction");
                    ui.add(egui::DragValue::new(&mut v.0));
                    ui.end_row();
                    ui.label("Roof height");
                    ui.add(egui::DragValue::new(&mut v.1).range(-32768..=32767));
                    ui.end_row();
                    ui.label("Sound");
                    ui.add(egui::DragValue::new(&mut v.2).range(0..=255));
                    ui.end_row();
                    ui.label("Animated");
                    ui.checkbox(&mut v.3, "");
                    ui.end_row();
                    ui.label("Orientation");
                    ui.add(egui::DragValue::new(&mut v.4));
                    ui.end_row();
                    ui.label("Main index");
                    ui.add(egui::DragValue::new(&mut v.5));
                    ui.end_row();
                    ui.label("Sub index");
                    ui.add(egui::DragValue::new(&mut v.6));
                    ui.end_row();
                });
            if v != before {
                let patch = TileSettingsPatch {
                    direction: Some(v.0),
                    roof_height: i16::try_from(v.1).ok(),
                    sound: u8::try_from(v.2).ok(),
                    animated: Some(v.3),
                    orientation: Some(v.4),
                    main_index: Some(v.5),
                    sub_index: Some(v.6),
                    frame: None,
                    unknown: None,
                    flags: None,
                };
                let r = lib.edit(|dt| {
                    write_settings(dt, &[(tile, patch)]).map_err(|i| format!("no tile {i}"))
                });
                if let Err(e) = r {
                    d.status = e;
                }
            }
        }
        ui.separator();
        ui.label("DT1 Tools .ini (the same text DT1 Tools writes):");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut d.ini_path)
                    .hint_text("path of the .ini file")
                    .desired_width(300.0),
            );
            if ui.button("Browse...").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .add_filter("ini", &["ini"])
                    .pick_file()
            {
                d.ini_path = p.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Export .ini").clicked() {
                let text = dt1_to_ini(&lib.dt1);
                d.status = match std::fs::write(&d.ini_path, text) {
                    Ok(()) => format!("Wrote {}", d.ini_path),
                    Err(e) => format!("cannot write {}: {e}", d.ini_path),
                };
            }
            if ui.button("Import .ini").clicked() {
                d.status = match std::fs::read_to_string(&d.ini_path) {
                    Ok(text) => {
                        let (blocks, _count) = parse_dt1_ini(&text);
                        let r = lib.edit(|dt| Ok(apply_ini(dt, &blocks)));
                        match r {
                            Ok((changed, missing)) => format!(
                                "Imported: {} tile(s) changed, {} not in this library",
                                changed.len(),
                                missing.len()
                            ),
                            Err(e) => e,
                        }
                    }
                    Err(e) => format!("cannot read {}: {e}", d.ini_path),
                };
            }
        });
    }
    state.tools.dt1 = d;
}

/// The 5x5 sub-tile grid as a diamond: cell (x, y) is at screen
/// `(x - y, x + y)`; returns the cell under `pos`.
fn diamond_cell(rect: egui::Rect, pos: egui::Pos2) -> Option<usize> {
    let cw = rect.width() / 10.0;
    let ch = rect.height() / 10.0;
    let dx = (pos.x - rect.center().x) / cw; // = x - y
    let dy = (pos.y - rect.top()) / ch; // = x + y + 1 for centres
    let x = ((dy + dx) / 2.0).floor();
    let y = ((dy - dx) / 2.0).floor();
    ((0.0..5.0).contains(&x) && (0.0..5.0).contains(&y)).then(|| y as usize * 5 + x as usize)
}

fn diamond_poly(rect: egui::Rect, x: usize, y: usize) -> Vec<egui::Pos2> {
    let cw = rect.width() / 10.0;
    let ch = rect.height() / 10.0;
    let cx = rect.center().x + (x as f32 - y as f32) * cw;
    let top = rect.top() + (x as f32 + y as f32) * ch;
    vec![
        egui::pos2(cx, top),
        egui::pos2(cx + cw, top + ch),
        egui::pos2(cx, top + 2.0 * ch),
        egui::pos2(cx - cw, top + ch),
    ]
}

pub fn collision(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.dt1);
    chooser(ui, state, &mut d);
    lib_header(ui, state, &mut d);
    status_line(ui, &d);
    let palette = d.palette(state);
    let (op_i, bits) = (d.flag_op, d.flag_bits);
    if d.lib.is_some() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Brush:");
            for (i, name) in ["Add", "Remove", "Set", "Make walkable"].iter().enumerate() {
                ui.selectable_value(&mut d.flag_op, i, *name);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for f in COLLISION_FLAGS {
                let mut on = d.flag_bits & f.bit != 0;
                if ui
                    .checkbox(&mut on, format!("{:02X} {}", f.bit, f.name))
                    .on_hover_text(f.help)
                    .changed()
                {
                    d.flag_bits ^= f.bit;
                }
            }
        });
    }
    let _ = (op_i, bits);
    let (op_i, bits) = (d.flag_op, d.flag_bits);
    let mut paint_cells: Vec<usize> = Vec::new();
    let mut whole = false;
    if let Some(lib) = d.lib.as_mut() {
        let tile = lib.tile.min(lib.dt1.tiles.len().saturating_sub(1));
        ui.horizontal_top(|ui| {
            if let (Some(p), Some(_)) = (palette.as_ref(), lib.dt1.tiles.get(tile))
                && let Some(t) = lib.texture(ui.ctx(), p, tile)
            {
                let ts = t.size_vec2();
                let scale = (260.0 / ts.x).min(260.0 / ts.y).min(3.0);
                ui.image((t.id(), ts * scale));
            }
            ui.vertical(|ui| {
                ui.label(format!(
                    "Tile {tile}: click or drag across the 5x5 sub-tiles"
                ));
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(300.0, 150.0), egui::Sense::click_and_drag());
                if let Some(t) = lib.dt1.tiles.get(tile) {
                    let painter = ui.painter_at(rect);
                    for y in 0..5 {
                        for x in 0..5 {
                            let f = t.subtile_flags[y * 5 + x];
                            let fill = if f == 0 {
                                Color32::from_rgb(40, 90, 50)
                            } else if f & WALKABLE_MASK != 0 {
                                Color32::from_rgb(150, 60, 50)
                            } else {
                                Color32::from_rgb(150, 120, 40)
                            };
                            let poly = diamond_poly(rect, x, y);
                            let c =
                                poly.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.0;
                            painter.add(egui::Shape::convex_polygon(
                                poly,
                                fill,
                                egui::Stroke::new(1.0, Color32::from_gray(15)),
                            ));
                            painter.text(
                                c.to_pos2(),
                                egui::Align2::CENTER_CENTER,
                                collision_hex(f),
                                egui::FontId::monospace(10.0),
                                Color32::WHITE,
                            );
                        }
                    }
                }
                if (resp.clicked() || resp.dragged())
                    && let Some(p) = resp.interact_pointer_pos()
                    && let Some(cell) = diamond_cell(rect, p)
                {
                    paint_cells.push(cell);
                }
                if let Some(p) = resp.hover_pos()
                    && let Some(cell) = diamond_cell(rect, p)
                    && let Some(t) = lib.dt1.tiles.get(tile)
                {
                    let f = t.subtile_flags[cell];
                    resp.on_hover_text(format!(
                        "({}, {}) {}",
                        cell % 5,
                        cell / 5,
                        describe_collision(f)
                    ));
                }
                ui.horizontal(|ui| {
                    if ui.button("Whole tile").clicked() {
                        whole = true;
                    }
                    ui.weak(format!("{} tile(s) selected", lib.targets().len()));
                });
            });
        });
        if !paint_cells.is_empty() || whole {
            let cells: Vec<usize> = if whole {
                (0..25).collect()
            } else {
                paint_cells
            };
            let op = match op_i {
                0 => FlagOp::Add(bits),
                1 => FlagOp::Remove(bits),
                2 => FlagOp::Set(bits),
                _ => FlagOp::MakeWalkable,
            };
            let tiles = lib.targets();
            let r = lib
                .edit(|dt| edit_subtile_flags(dt, &tiles, &cells, op).map_err(|e| e.to_string()));
            if let Err(e) = r {
                d.status = e;
            }
        }
    }
    state.tools.dt1 = d;
}

/// Pixel painting on one block of the selected tile, with `capy-paint`.
pub fn paint(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.dt1);
    chooser(ui, state, &mut d);
    lib_header(ui, state, &mut d);
    status_line(ui, &d);
    let palette = d.palette(state);
    let (Some(lib), Some(palette)) = (d.lib.as_mut(), palette) else {
        state.tools.dt1 = d;
        return;
    };
    let pp = &mut d.paint;
    let tile = lib.tile.min(lib.dt1.tiles.len().saturating_sub(1));
    ui.separator();
    let blocks = lib.dt1.tiles.get(tile).map_or(0, |t| t.blocks.len());
    if blocks == 0 {
        ui.label("This tile has no blocks to paint.");
        state.tools.dt1 = d;
        return;
    }
    // Load the block into a paint session when the choice changed.
    if pp.session.is_none() || pp.tile != tile || pp.block >= blocks {
        pp.tile = tile;
        pp.block = pp.block.min(blocks - 1);
        pp.session = None;
    }
    ui.horizontal(|ui| {
        ui.label(format!("Tile {tile}, block"));
        let before = pp.block;
        egui::ComboBox::from_id_salt("paint_block")
            .selected_text(format!("{}", pp.block))
            .show_ui(ui, |ui| {
                for b in 0..blocks {
                    ui.selectable_value(&mut pp.block, b, format!("{b}"));
                }
            });
        if before != pp.block {
            pp.session = None;
        }
        for (i, n) in ["Pencil", "Fill", "Pick"].iter().enumerate() {
            ui.selectable_value(&mut pp.tool, i as u8, *n);
        }
    });
    if pp.session.is_none() {
        let block = &lib.dt1.tiles[tile].blocks[pp.block];
        match block.render() {
            Ok(img) => {
                pp.mask = (block.format == BlockFormat::Isometric).then(|| {
                    let mut m = capy_paint::Mask::none(img.width, img.height);
                    for (i, a) in img.alpha.iter().enumerate() {
                        m.bits[i] = *a != 0;
                    }
                    m
                });
                pp.session = PaintSession::new(vec![vec![img]], palette.clone()).ok();
                pp.tex = None;
            }
            Err(e) => {
                ui.label(format!("cannot decode the block: {e}"));
            }
        }
    }
    let Some(session) = pp.session.as_mut() else {
        state.tools.dt1 = d;
        return;
    };
    let fr = FrameRef { dir: 0, frame: 0 };
    ui.horizontal(|ui| {
        if ui
            .add_enabled(session.can_undo(), egui::Button::new("Undo stroke"))
            .clicked()
        {
            session.undo();
        }
        if ui
            .add_enabled(session.can_redo(), egui::Button::new("Redo stroke"))
            .clicked()
        {
            session.redo();
        }
        if ui
            .add_enabled(session.is_dirty(), egui::Button::new("Apply to the tile"))
            .clicked()
            && let Some(img) = session.frame(fr).cloned()
        {
            let (t, b) = (tile, pp.block);
            match lib.edit(|dt| set_block_pixels(dt, t, b, &img).map_err(|e| e.to_string())) {
                Ok(c) => {
                    d.status = format!("Painted {} pixel(s)", c.pixels);
                    session.mark_saved();
                }
                Err(e) => d.status = e,
            }
        }
    });
    // The canvas, 10x.
    if let Some(img) = session.frame(fr).cloned() {
        if pp.tex.as_ref().map(|t| t.0) != Some(session.revision())
            && let Some(tex) = texture(ui.ctx(), &img, session.palette(), "dt1-paint")
        {
            pp.tex = Some((session.revision(), tex));
        }
        let zoom = (360.0 / img.width.max(img.height).max(1) as f32)
            .clamp(2.0, 12.0)
            .floor();
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(img.width as f32 * zoom, img.height as f32 * zoom),
            egui::Sense::click_and_drag(),
        );
        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(28));
        if let Some((_, tex)) = &pp.tex {
            ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if (resp.clicked() || resp.dragged())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let px = ((p.x - rect.left()) / zoom).floor() as i32;
            let py = ((p.y - rect.top()) / zoom).floor() as i32;
            let brush = Brush {
                size: 1,
                shape: BrushShape::Square,
            };
            let mask = pp.mask.as_ref();
            let r = match pp.tool {
                0 => session.pencil(fr, &[(px, py)], brush, Paint::Index(pp.color.max(1)), mask),
                1 => session.fill(
                    fr,
                    (px, py),
                    Paint::Index(pp.color.max(1)),
                    capy_paint::FillMode::Contiguous,
                    mask,
                ),
                _ => {
                    if let Some(Paint::Index(i)) = session.pick(fr, (px, py)) {
                        pp.color = i;
                    }
                    Ok(())
                }
            };
            if let Err(e) = r {
                d.status = e.to_string();
            }
        }
    }
    ui.label(format!("Colour: index {}", pp.color));
    let pal = session.palette().clone();
    for row in 0..16 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(1.0, 1.0);
            for col in 0..16 {
                let i = (row * 16 + col) as u8;
                let [r, g, b] = pal.colors[usize::from(i)];
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
                if i == pp.color {
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(2.0, Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
                if resp.clicked() {
                    pp.color = i;
                }
            }
        });
    }
    state.tools.dt1 = d;
}

/// Make act-safe: no pixel left in the act-specific colours 225..=254.
pub fn act_safe_apply(state: &mut AppState, all: bool) -> Result<String, String> {
    let mut d = std::mem::take(&mut state.tools.dt1);
    let result: Result<String, String> = (|| {
        let home = load_palette(state, d.home_act).ok_or("the palette could not be read")?;
        let lib = d.lib.as_mut().ok_or("No library is open")?;
        let opts = ActSafeOptions {
            safe: Safe::default(),
            distance: match d.distance {
                1 => capy_paint::remap::ColorDistance::Redmean,
                2 => capy_paint::remap::ColorDistance::Rgb,
                _ => capy_paint::remap::ColorDistance::OkLab,
            },
        };
        let targets = lib.targets();
        let sel: Option<&[usize]> = if all { None } else { Some(&targets) };
        let report =
            lib.edit(|dt| make_act_safe(dt, &home, sel, &opts).map_err(|e| e.to_string()))?;
        Ok(format!(
            "{} unsafe pixel(s) found, {} changed in {} tile(s)",
            report.unsafe_pixels,
            report.pixels_changed,
            report.tiles.len()
        ))
    })();
    d.report = match &result {
        Ok(m) | Err(m) => m.clone(),
    };
    state.tools.dt1 = d;
    result
}

pub fn act_safe(ui: &mut Ui, state: &mut AppState) {
    let mut d = std::mem::take(&mut state.tools.dt1);
    chooser(ui, state, &mut d);
    lib_header(ui, state, &mut d);
    status_line(ui, &d);
    if let Some((n, sel)) = d.lib.as_ref().map(|l| {
        (
            count_unsafe_pixels(&l.dt1, None, &Safe::default()),
            l.targets().len(),
        )
    }) {
        ui.separator();
        ui.label(format!(
            "{n} pixel(s) use the act-specific colours (palette indices 225-254), which change colour in other acts."
        ));
        ui.horizontal(|ui| {
            ui.label("Take the replacement colours from");
            egui::ComboBox::from_id_salt("safe_home")
                .selected_text(format!("Act {}", d.home_act + 1))
                .show_ui(ui, |ui| {
                    for a in 0..5u8 {
                        ui.selectable_value(&mut d.home_act, a, format!("Act {}", a + 1));
                    }
                });
            egui::ComboBox::from_id_salt("safe_dist")
                .selected_text(["OkLab", "Redmean", "RGB"][d.distance.min(2)])
                .show_ui(ui, |ui| {
                    for (i, n) in ["OkLab", "Redmean", "RGB"].iter().enumerate() {
                        ui.selectable_value(&mut d.distance, i, *n);
                    }
                });
        });
        ui.horizontal(|ui| {
            if ui
                .button(format!("Make {sel} selected tile(s) act-safe"))
                .clicked()
            {
                state.tools.dt1 = std::mem::take(&mut d);
                let _ = act_safe_apply(state, false);
                d = std::mem::take(&mut state.tools.dt1);
            }
            if ui.button("Make the whole library act-safe").clicked() {
                state.tools.dt1 = std::mem::take(&mut d);
                let _ = act_safe_apply(state, true);
                d = std::mem::take(&mut state.tools.dt1);
            }
        });
        if !d.report.is_empty() {
            ui.label(&d.report);
        }
        ui.separator();
        let h = ui.available_height().max(120.0);
        tile_grid(ui, &mut d, state, h);
    }
    state.tools.dt1 = d;
}
