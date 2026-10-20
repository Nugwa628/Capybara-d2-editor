//! The editable view modes (ds1-studio `ModePanels.tsx`, `WalkPanel.tsx`):
//!
//! * **Automap**: click a cell to give its tile an automap picture (cel) in
//!   AutoMap.txt, the same edit the Automap editor window makes
//!   (`capy_mod::automap`), previewed live and saved with the table.
//! * **Level light**: the map's Levels.txt Intensity / Red / Green / Blue,
//!   previewed by a colour multiply in the tile shader and saved through
//!   `save_game_file`.
//! * **Walkability**: paint whole cells (DS1 flag bit 17) or single sub-tiles
//!   (blockers and tile copies in `<map>_walk.dt1`), and the no-spawn area
//!   planner. Every map change is one undo step.

use std::collections::BTreeMap;

use bevy_egui::egui::{self, Align2, Color32, Response, RichText, Shape, Stroke, Ui};
use capy_edit::{CellValue, LayerRef};
use capy_mod::automap::{AutomapTable, Piece, TileId, pieces};
use d2_formats::ds1::{Cell, Ds1};
use d2_formats::txt::TxtDoc;

use super::walkplan::{self, Edit, WalkMode, WalkPaint};
use super::{ViewMode, current_view, levels, libs, views};
use crate::edit::ops::EditOp;
use crate::state::AppState;
use crate::ui::map_tools::{ViewFrame, subtile_at};
use crate::ui::tools::{automap_ed, io};

// ───────────────────────── state ─────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoTool {
    #[default]
    Select,
    Paint,
    Pick,
    Clear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WalkTool {
    #[default]
    Select,
    /// Whole cells (DS1 flag).
    Cell,
    /// Single sub-tiles (`<map>_walk.dt1`).
    Sub,
}

pub struct AutomapPaint {
    pub tool: AutoTool,
    pub cel: i32,
    /// Apply to every sequence of the tile's style.
    pub wide: bool,
    last: Option<(u32, u32)>,
    pub msg: String,
}

impl Default for AutomapPaint {
    fn default() -> Self {
        AutomapPaint {
            tool: AutoTool::Select,
            cel: 0,
            wide: false,
            last: None,
            msg: String::new(),
        }
    }
}

#[derive(Default)]
pub struct LightEdit {
    /// `(doc id, level id)` the fields were loaded for.
    loaded: Option<(u64, i32)>,
    pub intensity: i32,
    pub rgb: [i32; 3],
    pub msg: String,
}

struct WalkDrag {
    doc: u64,
    cells: BTreeMap<(u32, u32), u32>,
}

pub struct WalkTools {
    pub tool: WalkTool,
    pub mode: WalkMode,
    /// Collision bits a sub-tile stroke sets or clears (1 walk, 2 light, 4 jump, 8 player).
    pub bits: u8,
    /// Brush edge in sub-tiles.
    pub size: u32,
    pub whole_rooms: bool,
    drag: Option<WalkDrag>,
    pub msg: String,
}

impl Default for WalkTools {
    fn default() -> Self {
        WalkTools {
            tool: WalkTool::Select,
            mode: WalkMode::Block,
            bits: 1,
            size: 1,
            whole_rooms: false,
            drag: None,
            msg: String::new(),
        }
    }
}

#[derive(Default)]
pub struct Modes {
    pub automap: AutomapPaint,
    pub light: LightEdit,
    pub walk: WalkTools,
    /// The document the Automap editor table was opened for.
    automap_doc: Option<u64>,
}

// ───────────────────────── shared edit plumbing ─────────────────────────

fn set_in(ds1: &mut Ds1, (layer, x, y, v): Edit) {
    match layer {
        LayerRef::Floor(i) => {
            if let Some(f) = ds1.floors.get_mut(usize::from(i)) {
                f.set(x, y, Cell(v.cell));
            }
        }
        LayerRef::Wall(i) => {
            if let Some(w) = ds1.walls.get_mut(usize::from(i)) {
                w.tiles.set(x, y, Cell(v.cell));
                w.orientations.set(x, y, v.orientation);
            }
        }
        _ => {}
    }
}

/// Cells of one layer.
type LayerCells = (LayerRef, Vec<(u32, u32, CellValue)>);

/// Applies planned cell edits as one undo step (adding floor 2 first when an
/// edit needs it).
pub fn exec_edits(state: &mut AppState, index: usize, label: &str, edits: &[Edit]) -> bool {
    if edits.is_empty() {
        return true;
    }
    state.docs[index].edit.begin_group(label);
    let mut ok = true;
    if edits.iter().any(|e| e.0 == LayerRef::Floor(1))
        && state.docs[index].ds1.floors.len() < 2
        && !state.exec(index, EditOp::AddLayer(LayerRef::Floor(1)))
    {
        ok = false;
    }
    let mut by: Vec<LayerCells> = Vec::new();
    for &(l, x, y, v) in edits {
        match by.iter_mut().find(|(k, _)| *k == l) {
            Some((_, cells)) => cells.push((x, y, v)),
            None => by.push((l, vec![(x, y, v)])),
        }
    }
    for (layer, cells) in by {
        if ok {
            ok = state.exec(index, EditOp::Paint { layer, cells });
        }
    }
    state.docs[index].edit.end_group();
    ok
}

fn selection_cells(state: &AppState, index: usize) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = state.docs[index]
        .selection
        .iter()
        .map(|(_, x, y)| (x, y))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn all_cells(ds1: &Ds1) -> Vec<(u32, u32)> {
    (0..ds1.height)
        .flat_map(|y| (0..ds1.width).map(move |x| (x, y)))
        .collect()
}

// ───────────────────────── automap ─────────────────────────

/// Opens AutoMap.txt for the active map (once per map; unsaved edits stay).
pub fn ensure_automap(state: &mut AppState) {
    let Some(doc) = state.active_doc().map(|d| d.id) else {
        return;
    };
    let fresh = state.tools.automap.doc.is_none()
        || (state.studio.modes.automap_doc != Some(doc) && !state.tools.automap.dirty);
    if fresh {
        automap_ed::on_open(state);
    }
    state.studio.modes.automap_doc = Some(doc);
}

/// The pieces of the open map under the Automap editor's level.
pub fn automap_pieces(state: &AppState, index: usize) -> Option<Vec<Piece>> {
    let a = &state.tools.automap;
    let table = AutomapTable::parse(a.doc.as_ref()?);
    Some(pieces(&state.docs[index].ds1, &table, &a.level))
}

fn piece_at(state: &AppState, index: usize, cell: (u32, u32)) -> Option<Piece> {
    automap_pieces(state, index)?
        .into_iter()
        .rfind(|p| (p.x, p.y) == cell)
}

fn tile_name(t: TileId) -> String {
    format!(
        "{} {}/{}",
        capy_mod::automap::AUTOMAP_CODES
            .get(t.orientation)
            .copied()
            .unwrap_or("?"),
        t.main,
        t.sub
    )
}

/// One automap click on `cell`. Returns what happened.
pub fn automap_apply(
    state: &mut AppState,
    index: usize,
    cell: (u32, u32),
    tool: AutoTool,
) -> Result<String, String> {
    ensure_automap(state);
    if state.tools.automap.doc.is_none() {
        return Err(state.tools.automap.msg.clone());
    }
    let piece = piece_at(state, index, cell)
        .ok_or_else(|| format!("({}, {}) has no floor or wall tile", cell.0, cell.1))?;
    let name = tile_name(piece.tile);
    match tool {
        AutoTool::Select => Ok(String::new()),
        AutoTool::Pick => {
            let cel = piece
                .cel
                .ok_or_else(|| format!("{name} has no automap picture"))?;
            state.studio.modes.automap.cel = cel;
            Ok(format!("{name}: picture {cel}"))
        }
        AutoTool::Paint | AutoTool::Clear => {
            let cel = if tool == AutoTool::Clear {
                -1
            } else {
                state.studio.modes.automap.cel
            };
            let wide = state.studio.modes.automap.wide;
            let summary = automap_ed::set_tile_cel(state, piece.tile, cel, wide)?;
            views::invalidate_automap(state);
            Ok(format!("{name}: {summary}"))
        }
    }
}

pub fn automap_save(state: &mut AppState) -> Result<(), String> {
    automap_ed::save(state)?;
    views::invalidate_automap(state);
    Ok(())
}

// ───────────────────────── level light ─────────────────────────

/// Writes the light of `level` into a Levels.txt document.
pub fn set_light(
    doc: &mut TxtDoc,
    level: i32,
    intensity: i32,
    rgb: [i32; 3],
) -> Result<(), String> {
    let id_col = doc.column("Id").ok_or("Levels.txt has no Id column")?;
    let row = (0..doc.row_count())
        .find(|&r| {
            doc.cell(r, id_col)
                .and_then(|c| c.trim().parse::<i32>().ok())
                == Some(level)
        })
        .ok_or_else(|| format!("Levels.txt has no level {level}"))?;
    for (name, value) in [
        ("Intensity", intensity),
        ("Red", rgb[0]),
        ("Green", rgb[1]),
        ("Blue", rgb[2]),
    ] {
        let col = doc
            .column(name)
            .ok_or_else(|| format!("Levels.txt has no {name} column"))?;
        doc.set_cell(row, col, &value.clamp(0, 255).to_string())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn save_light(
    state: &mut AppState,
    level: i32,
    intensity: i32,
    rgb: [i32; 3],
) -> Result<(), String> {
    let mut doc = io::read_txt(state, "levels.txt")?;
    set_light(&mut doc, level, intensity, rgb)?;
    io::save_txt(state, "levels.txt", &doc)?;
    levels::invalidate(state);
    Ok(())
}

/// The colour the tile shader multiplies by: the Levels.txt light of the
/// active map (or the fields being edited) in Level light mode, else white.
pub fn light_tint(state: &mut AppState) -> [f32; 3] {
    if current_view(state) != ViewMode::Light {
        return [1.0; 3];
    }
    let Some(index) = state.active else {
        return [1.0; 3];
    };
    let Some(row) = levels::row_of(state, index) else {
        return [1.0; 3];
    };
    let m = &state.studio.modes.light;
    let (intensity, rgb) = if m.loaded == Some((state.docs[index].id, row.id)) {
        (m.intensity, m.rgb)
    } else {
        (row.intensity, row.rgb)
    };
    capy_render::links::LevelLight { intensity, rgb }
        .multiplier()
        .unwrap_or([1.0; 3])
}

fn light_panel(ui: &mut Ui, state: &mut AppState, index: usize) {
    let Some(row) = levels::row_of(state, index) else {
        ui.weak("This map has no Levels.txt row (it is not placed in a level).");
        return;
    };
    let doc_id = state.docs[index].id;
    if state.studio.modes.light.loaded != Some((doc_id, row.id)) {
        let m = &mut state.studio.modes.light;
        m.loaded = Some((doc_id, row.id));
        m.intensity = row.intensity;
        m.rgb = row.rgb;
        m.msg.clear();
    }
    ui.label(RichText::new(format!("{} (level {})", row.name, row.id)).strong());
    let m = &mut state.studio.modes.light;
    let mut changed = false;
    changed |= ui
        .add(egui::Slider::new(&mut m.intensity, 0..=255).text("Intensity"))
        .on_hover_text("0 = the act's daylight; the light is intensity x colour / 255 x 255")
        .changed();
    for (v, name) in m.rgb.iter_mut().zip(["Red", "Green", "Blue"]) {
        changed |= ui.add(egui::Slider::new(v, 0..=255).text(name)).changed();
    }
    if changed {
        m.msg.clear();
    }
    let (intensity, rgb) = (m.intensity, m.rgb);
    let dirty = intensity != row.intensity || rgb != row.rgb;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(dirty, egui::Button::new("Save to Levels.txt"))
            .on_hover_text("Writes the row to the profile's saves (CHANGED.txt lists it)")
            .clicked()
        {
            let msg = match save_light(state, row.id, intensity, rgb) {
                Ok(()) => format!("Saved the light of level {} to Levels.txt", row.id),
                Err(e) => e,
            };
            state.studio.modes.light.msg = msg;
            state.studio.modes.light.loaded = None;
        }
        if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
            state.studio.modes.light.loaded = None;
        }
    });
    let msg = &state.studio.modes.light.msg;
    if !msg.is_empty() {
        ui.weak(msg);
    }
}

// ───────────────────────── walkability ─────────────────────────

/// Whole-cell paint: `on` blocks, otherwise clears the DS1 flag.
pub fn walk_cells(
    state: &mut AppState,
    index: usize,
    cells: &[(u32, u32)],
    on: bool,
) -> Result<String, String> {
    let plan = walkplan::cell_unwalkable_plan(&state.docs[index].ds1, cells, on);
    if plan.edits.is_empty() {
        return Ok(format!(
            "Nothing to change{}",
            if plan.skipped > 0 {
                format!(" ({} cell(s) have no floor to copy)", plan.skipped)
            } else {
                String::new()
            }
        ));
    }
    let label = if on { "Block cells" } else { "Unblock cells" };
    if !exec_edits(state, index, label, &plan.edits) {
        return Err("the edit failed".to_owned());
    }
    Ok(format!(
        "{} {} cell(s){}",
        if on { "Blocked" } else { "Cleared" },
        plan.cells,
        if plan.filled > 0 {
            format!(" ({} empty: hidden floor copy)", plan.filled)
        } else {
            String::new()
        }
    ))
}

/// Sub-tile paint: blockers and tile copies go to `<map>_walk.dt1`.
pub fn walk_subtiles(
    state: &mut AppState,
    index: usize,
    paint: &WalkPaint,
) -> Result<String, String> {
    let tiles = state.docs[index]
        .tiles
        .clone()
        .ok_or("the map's tiles are not loaded yet")?;
    let map = state.docs[index].path.clone();
    let wpath = libs::walk_path(&map);
    let existing = libs::read_dt1(state, &wpath)?;
    let ds1 = state.docs[index].ds1.clone();
    let plan = walkplan::plan_walk(&ds1, &tiles, existing.as_ref(), paint, None);
    let note = if plan.skipped.is_empty() {
        String::new()
    } else {
        format!(
            "; skipped {}: {}",
            plan.skipped.len(),
            plan.skipped.first().map_or("", String::as_str)
        )
    };
    if plan.edits.is_empty() && plan.dt1.is_none() {
        return Ok(format!("Nothing to change{note}"));
    }
    if let Some(d) = &plan.dt1 {
        libs::save_dt1(state, &wpath, d)?;
    }
    let mut after = libs::with_file(&ds1, &wpath);
    while after.floors.len() < plan.floors {
        after
            .add_floor_layer()
            .map_err(|e| format!("cannot add a floor layer: {e}"))?;
    }
    for e in &plan.edits {
        set_in(&mut after, *e);
    }
    let label = match paint.mode {
        WalkMode::Block => "Block sub-tiles",
        WalkMode::Clear => "Clear sub-tiles",
    };
    if !state.exec(
        index,
        EditOp::ReplaceModel {
            label: label.to_owned(),
            after: Box::new(after),
        },
    ) {
        return Err("the edit failed".to_owned());
    }
    libs::attach_to_info(state, index, &wpath);
    Ok(format!(
        "{label}: {} sub-tile(s) changed{note}",
        plan.changed
    ))
}

/// The no-spawn area planner over `cells` (the whole map when empty).
pub fn no_spawn(
    state: &mut AppState,
    index: usize,
    cells: &[(u32, u32)],
    whole_rooms: bool,
) -> Result<String, String> {
    let ds1 = state.docs[index].ds1.clone();
    let all;
    let cells = if cells.is_empty() {
        all = all_cells(&ds1);
        &all
    } else {
        cells
    };
    let plan = walkplan::no_spawn_plan(&ds1, cells, whole_rooms);
    if plan.edits.is_empty() {
        return Ok(format!(
            "No-spawn: nothing to change in {} room(s){}",
            plan.rooms,
            plan.skipped
                .first()
                .map_or(String::new(), |s| format!(" ({s})"))
        ));
    }
    if !exec_edits(state, index, "No-spawn area", &plan.edits) {
        return Err("the edit failed (a second floor layer may need a newer map version)".into());
    }
    Ok(format!(
        "No-spawn: {} room(s), {} floor(s) hidden over a copy, {} empty seed(s) marked{}",
        plan.rooms,
        plan.copied,
        plan.voids,
        if plan.skipped.is_empty() {
            String::new()
        } else {
            format!("; {} skipped", plan.skipped.len())
        }
    ))
}

fn walk_paint_of(w: &WalkTools, cells: BTreeMap<(u32, u32), u32>) -> WalkPaint {
    WalkPaint {
        mode: w.mode,
        bits: w.bits,
        cells,
    }
}

fn finish_walk_drag(state: &mut AppState, index: usize) {
    let Some(drag) = state.studio.modes.walk.drag.take() else {
        return;
    };
    if drag.cells.is_empty() || state.docs[index].id != drag.doc {
        return;
    }
    let w = &state.studio.modes.walk;
    let (tool, block) = (w.tool, w.mode == WalkMode::Block);
    let result = match tool {
        WalkTool::Cell => {
            let cells: Vec<(u32, u32)> = drag.cells.keys().copied().collect();
            walk_cells(state, index, &cells, block)
        }
        WalkTool::Sub => {
            let paint = walk_paint_of(&state.studio.modes.walk, drag.cells);
            walk_subtiles(state, index, &paint)
        }
        WalkTool::Select => return,
    };
    state.studio.modes.walk.msg = result.unwrap_or_else(|e| e);
}

// ───────────────────────── input ─────────────────────────

/// Pointer handling of the paint tools of the Automap and Walkability
/// modes. True when the press or drag was taken.
pub fn input(
    ui: &Ui,
    state: &mut AppState,
    f: &ViewFrame,
    response: &Response,
    space: bool,
) -> bool {
    let Some(index) = state.active else {
        return false;
    };
    let view = current_view(state);
    let (pressed, down, pos) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.primary_down(),
            i.pointer.latest_pos(),
        )
    });
    let Some(p) = pos else { return false };
    let (cx, cy) = f.cell_at(p);
    let cell = u32::try_from(cx)
        .ok()
        .zip(u32::try_from(cy).ok())
        .filter(|&(x, y)| x < state.docs[index].ds1.width && y < state.docs[index].ds1.height);
    match view {
        ViewMode::Automap if state.studio.modes.automap.tool != AutoTool::Select => {
            let tool = state.studio.modes.automap.tool;
            if !down {
                state.studio.modes.automap.last = None;
                return false;
            }
            if !(pressed && response.hovered() && !space)
                && state.studio.modes.automap.last.is_none()
            {
                return false;
            }
            if let Some(c) = cell
                && state.studio.modes.automap.last != Some(c)
                && !(tool == AutoTool::Pick && !pressed)
            {
                state.studio.modes.automap.last = Some(c);
                state.studio.modes.automap.msg =
                    automap_apply(state, index, c, tool).unwrap_or_else(|e| e);
            } else if state.studio.modes.automap.last.is_none() {
                state.studio.modes.automap.last = Some((u32::MAX, u32::MAX));
            }
            true
        }
        ViewMode::Walk if state.studio.modes.walk.tool != WalkTool::Select => {
            let doc = state.docs[index].id;
            if state.studio.modes.walk.drag.is_none() {
                if !(pressed && response.hovered() && !space) {
                    return false;
                }
                state.studio.modes.walk.drag = Some(WalkDrag {
                    doc,
                    cells: BTreeMap::new(),
                });
            }
            if !down {
                finish_walk_drag(state, index);
                return true;
            }
            let (tool, size) = (
                state.studio.modes.walk.tool,
                state.studio.modes.walk.size as i32,
            );
            let (w, h) = (state.docs[index].ds1.width, state.docs[index].ds1.height);
            let (sx, sy) = subtile_at(f, p);
            let Some(drag) = state.studio.modes.walk.drag.as_mut() else {
                return true;
            };
            match tool {
                WalkTool::Cell => {
                    if let Some(c) = cell {
                        drag.cells.insert(c, 0x1ff_ffff);
                    }
                }
                _ => {
                    for dy in 0..size {
                        for dx in 0..size {
                            let (gx, gy) = (sx - size / 2 + dx, sy - size / 2 + dy);
                            let (Ok(x), Ok(y)) = (
                                u32::try_from(gx.div_euclid(5)),
                                u32::try_from(gy.div_euclid(5)),
                            ) else {
                                continue;
                            };
                            if x < w && y < h {
                                let bit = (gy.rem_euclid(5) * 5 + gx.rem_euclid(5)) as u32;
                                *drag.cells.entry((x, y)).or_insert(0) |= 1 << bit;
                            }
                        }
                    }
                }
            }
            true
        }
        _ => {
            if state.studio.modes.walk.drag.is_some() {
                state.studio.modes.walk.drag = None;
            }
            false
        }
    }
}

// ───────────────────────── panel ─────────────────────────

fn automap_panel(ui: &mut Ui, state: &mut AppState, index: usize) {
    ensure_automap(state);
    if state.tools.automap.doc.is_none() {
        ui.colored_label(Color32::LIGHT_RED, state.tools.automap.msg.clone());
        return;
    }
    let level = state.tools.automap.level.clone();
    ui.label(RichText::new(format!("AutoMap.txt level: {level}")).strong());
    let a = &mut state.studio.modes.automap;
    ui.horizontal(|ui| {
        for (t, label, tip) in [
            (AutoTool::Select, "Select", "Normal selection"),
            (
                AutoTool::Paint,
                "Paint",
                "Click a cell: its tile gets the picture",
            ),
            (AutoTool::Pick, "Pick", "Click a cell: take its picture"),
            (
                AutoTool::Clear,
                "Clear",
                "Click a cell: its tile leaves the automap",
            ),
        ] {
            ui.selectable_value(&mut a.tool, t, label)
                .on_hover_text(tip);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Picture (cel)");
        ui.add(egui::DragValue::new(&mut a.cel).range(0..=999));
    });
    ui.checkbox(&mut a.wide, "Whole style (every sequence of the tile)");
    let dirty = state.tools.automap.dirty;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(dirty, egui::Button::new("Save AutoMap.txt"))
            .clicked()
        {
            state.studio.modes.automap.msg = match automap_save(state) {
                Ok(()) => "Saved AutoMap.txt".to_owned(),
                Err(e) => e,
            };
        }
        if ui.small_button("Open the Automap editor").clicked() {
            crate::ui::commands::run(state, "game.automap");
        }
    });
    if dirty {
        ui.colored_label(Color32::from_rgb(230, 190, 90), "Unsaved AutoMap.txt edits");
    }
    let n =
        automap_pieces(state, index).map_or(0, |p| p.iter().filter(|p| p.cel.is_none()).count());
    ui.weak(format!(
        "Red cells ({n}) are tiles without a picture. Light = floor / wall pictures."
    ));
    let msg = &state.studio.modes.automap.msg;
    if !msg.is_empty() {
        ui.weak(msg);
    }
}

fn walk_panel(ui: &mut Ui, state: &mut AppState, index: usize) {
    let w = &mut state.studio.modes.walk;
    ui.horizontal(|ui| {
        for (t, label, tip) in [
            (WalkTool::Select, "Select", "Normal selection"),
            (
                WalkTool::Cell,
                "Cell",
                "Paint whole cells with the DS1 'unwalkable' flag",
            ),
            (
                WalkTool::Sub,
                "Sub-tile",
                "Paint single sub-tiles; blockers go to <map>_walk.dt1",
            ),
        ] {
            ui.selectable_value(&mut w.tool, t, label)
                .on_hover_text(tip);
        }
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut w.mode, WalkMode::Block, "Block");
        ui.selectable_value(&mut w.mode, WalkMode::Clear, "Clear");
    });
    if w.tool == WalkTool::Sub {
        ui.horizontal(|ui| {
            ui.label("Flags");
            for (bit, name) in [(1u8, "walk"), (2, "light"), (4, "jump"), (8, "player")] {
                let mut on = w.bits & bit != 0;
                if ui.checkbox(&mut on, name).changed() {
                    w.bits = if on { w.bits | bit } else { w.bits & !bit };
                    if w.bits == 0 {
                        w.bits = 1;
                    }
                }
            }
        });
        ui.add(egui::Slider::new(&mut w.size, 1..=5).text("brush (sub-tiles)"));
    }
    ui.separator();
    ui.label(RichText::new("No-spawn area").strong());
    ui.weak(
        "Makes the rooms under the selection (the whole map with none) spawn no \
         random monsters while staying walkable and looking the same.",
    );
    ui.checkbox(&mut w.whole_rooms, "Whole rooms, not only region seeds");
    if ui.button("Apply no-spawn area").clicked() {
        let cells = selection_cells(state, index);
        let whole = state.studio.modes.walk.whole_rooms;
        state.studio.modes.walk.msg = no_spawn(state, index, &cells, whole).unwrap_or_else(|e| e);
    }
    let w = &state.studio.modes.walk;
    if !w.msg.is_empty() {
        ui.weak(&w.msg);
    }
}

/// The floating tool panel of the editable modes and the stroke feedback.
pub fn panel(ui: &mut Ui, state: &mut AppState, f: &ViewFrame) {
    let Some(index) = state.active else { return };
    let view = current_view(state);
    if !matches!(view, ViewMode::Automap | ViewMode::Light | ViewMode::Walk) {
        return;
    }
    if view == ViewMode::Walk
        && let Some(drag) = &state.studio.modes.walk.drag
        && state.studio.modes.walk.tool == WalkTool::Cell
    {
        let painter = ui.painter_at(f.rect);
        for &(x, y) in drag.cells.keys() {
            let d = f.diamond(x as i32, y as i32);
            painter.add(Shape::convex_polygon(
                d.to_vec(),
                Color32::from_rgba_unmultiplied(240, 80, 60, 90),
                Stroke::NONE,
            ));
        }
    } else if view == ViewMode::Walk
        && let Some(drag) = &state.studio.modes.walk.drag
    {
        let painter = ui.painter_at(f.rect);
        for (&(x, y), &mask) in &drag.cells {
            let a = f.tile_to_screen(f64::from(x), f64::from(y));
            let _ = a;
            for k in 0..25u32 {
                if mask & (1 << k) == 0 {
                    continue;
                }
                let (sx, sy) = (f64::from(k % 5), f64::from(k / 5));
                let (u0, v0) = (f64::from(x) + sx / 5.0, f64::from(y) + sy / 5.0);
                let pts = vec![
                    f.tile_to_screen(u0, v0),
                    f.tile_to_screen(u0 + 0.2, v0),
                    f.tile_to_screen(u0 + 0.2, v0 + 0.2),
                    f.tile_to_screen(u0, v0 + 0.2),
                ];
                painter.add(Shape::convex_polygon(
                    pts,
                    Color32::from_rgba_unmultiplied(240, 80, 60, 140),
                    Stroke::NONE,
                ));
            }
        }
    }
    let ctx = ui.ctx().clone();
    let title = match view {
        ViewMode::Automap => "Automap",
        ViewMode::Light => "Level light",
        _ => "Walkability",
    };
    egui::Area::new(egui::Id::new("studio_mode_panel"))
        .order(egui::Order::Foreground)
        .anchor(Align2::LEFT_TOP, egui::vec2(10.0, 46.0))
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(250.0);
                ui.label(RichText::new(title).strong());
                match view {
                    ViewMode::Automap => automap_panel(ui, state, index),
                    ViewMode::Light => light_panel(ui, state, index),
                    _ => walk_panel(ui, state, index),
                }
            });
        });
}

// ───────────────────────── scripts ─────────────────────────

fn parse_u32(s: &str) -> Result<u32, String> {
    s.parse().map_err(|_| format!("{s:?} is not a number"))
}

/// The `CAPY_SCRIPT` words of the editable modes. `Ok(false)`: not done yet.
pub fn script(
    state: &mut AppState,
    index: usize,
    word: &str,
    args: &[String],
) -> Result<bool, String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let report = |state: &mut AppState, r: Result<String, String>| -> Result<bool, String> {
        let msg = r?;
        state.info(msg);
        Ok(true)
    };
    match (word, a.as_slice()) {
        ("stack-paint", [layer, rest @ ..]) if rest.len() >= 2 && rest.len() % 2 == 0 => {
            let layer = crate::edit::script::parse_layer(layer)?;
            let cells: Vec<(u32, u32)> = rest
                .chunks(2)
                .map(|c| Ok((parse_u32(c[0])?, parse_u32(c[1])?)))
                .collect::<Result<_, String>>()?;
            state.docs[index].edit.begin_group("Paint (stacked)");
            let ok =
                super::stack::paint_cells(state, index, layer, &cells, &mut Default::default());
            state.docs[index].edit.end_group();
            if ok {
                Ok(true)
            } else {
                Err("stack-paint failed".into())
            }
        }
        ("automap-cel", [x, y, cel, rest @ ..]) => {
            let cell = (parse_u32(x)?, parse_u32(y)?);
            let (tool, n) = if *cel == "clear" {
                (AutoTool::Clear, 0)
            } else {
                (
                    AutoTool::Paint,
                    cel.parse::<i32>().map_err(|e| e.to_string())?,
                )
            };
            state.studio.modes.automap.cel = n;
            state.studio.modes.automap.wide = rest.first() == Some(&"style");
            let r = automap_apply(state, index, cell, tool);
            report(state, r)
        }
        ("automap-open", []) => {
            ensure_automap(state);
            if state.tools.automap.doc.is_some() {
                Ok(true)
            } else {
                Err(state.tools.automap.msg.clone())
            }
        }
        ("assert-light", [i, r, g, b]) => {
            let row = levels::row_of(state, index).ok_or("the map has no Levels.txt row")?;
            let n = |s: &str| s.parse::<i32>().map_err(|e| e.to_string());
            if (row.intensity, row.rgb) == (n(i)?, [n(r)?, n(g)?, n(b)?]) {
                Ok(true)
            } else {
                Err(format!(
                    "assert-light: Levels.txt has {} {:?}",
                    row.intensity, row.rgb
                ))
            }
        }
        ("automap-save", []) => {
            automap_save(state)?;
            Ok(true)
        }
        ("light-set", [i, r, g, b]) => {
            let level = levels::row_of(state, index)
                .ok_or("the map has no Levels.txt row")?
                .id;
            let n = |s: &str| s.parse::<i32>().map_err(|e| e.to_string());
            save_light(state, level, n(i)?, [n(r)?, n(g)?, n(b)?])?;
            Ok(true)
        }
        ("walk-cell", [x, y, rest @ ..]) => {
            let cell = (parse_u32(x)?, parse_u32(y)?);
            let r = walk_cells(state, index, &[cell], rest.first() != Some(&"clear"));
            report(state, r)
        }
        ("walk-sub", [x, y, sx, sy, rest @ ..]) => {
            let (x, y) = (parse_u32(x)?, parse_u32(y)?);
            let (sx, sy) = (parse_u32(sx)?, parse_u32(sy)?);
            if sx > 4 || sy > 4 {
                return Err("walk-sub: sub-tile x and y are 0..4".into());
            }
            let paint = WalkPaint {
                mode: if rest.first() == Some(&"clear") {
                    WalkMode::Clear
                } else {
                    WalkMode::Block
                },
                bits: 1,
                cells: BTreeMap::from([((x, y), 1u32 << (sy * 5 + sx))]),
            };
            let r = walk_subtiles(state, index, &paint);
            report(state, r)
        }
        ("no-spawn", rest) => {
            let r = no_spawn(state, index, &[], rest.first() == Some(&"whole"));
            report(state, r)
        }
        ("reroll-lib", [layer, lib, rest @ ..]) => {
            let tiles: Vec<usize> = rest
                .iter()
                .map(|t| t.parse().map_err(|_| format!("{t:?} is not a tile index")))
                .collect::<Result<_, String>>()?;
            let layer = parse_u32(layer)? as u8;
            super::dialogs::reroll_from_library(state, index, lib, &tiles, layer, 1)?;
            Ok(true)
        }
        ("assert-flag", [layer, x, y, flag, on]) => {
            let layer = crate::edit::script::parse_layer(layer)?;
            let (x, y) = (parse_u32(x)?, parse_u32(y)?);
            let v = layer
                .get(&state.docs[index].ds1, x, y)
                .ok_or("no such cell")?;
            let bit = match *flag {
                "unwalkable" => walkplan::UNWALKABLE,
                "hidden" => 1 << 31,
                other => return Err(format!("no flag {other:?}")),
            };
            let want = *on == "on";
            if (v.cell & bit != 0) == want {
                Ok(true)
            } else {
                Err(format!("assert-flag: {flag} at ({x}, {y}) is not {on}"))
            }
        }
        ("assert-layers", [floors, walls]) => {
            let d = &state.docs[index].ds1;
            let (f, w) = (parse_u32(floors)? as usize, parse_u32(walls)? as usize);
            if (d.floors.len(), d.walls.len()) == (f, w) {
                Ok(true)
            } else {
                Err(format!(
                    "assert-layers: the map has {} floor / {} wall layer(s), wanted {f} / {w}",
                    d.floors.len(),
                    d.walls.len()
                ))
            }
        }
        ("assert-subtile", [x, y, sx, sy, want]) => {
            let Some(tiles) = state.docs[index].tiles.clone() else {
                return Ok(false);
            };
            let (x, y) = (parse_u32(x)?, parse_u32(y)?);
            let (sx, sy) = (parse_u32(sx)?, parse_u32(sy)?);
            let grid = capy_render::walk::sub_tile_flags(
                &state.docs[index].ds1,
                &tiles,
                capy_render::walk::Collision::Editor,
            );
            let blocked = grid.flags[grid.index(x, y, sx, sy)] & capy_render::walk::BLOCK_WALK != 0;
            if blocked == (*want == "blocked") {
                Ok(true)
            } else {
                // The tile set reloads in the background after a new library is listed.
                Ok(false)
            }
        }
        _ => Err(format!("{word}: wrong arguments")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const LEVELS: &str =
        "Name\tId\tIntensity\tRed\tGreen\tBlue\nA\t1\t0\t0\t0\t0\nB\t2\t100\t200\t100\t50\n";

    #[test]
    fn light_is_written_to_the_matching_row_only() {
        let mut doc = TxtDoc::parse(LEVELS.as_bytes());
        set_light(&mut doc, 1, 128, [255, 200, 100]).unwrap();
        let text = String::from_utf8(doc.to_bytes()).unwrap();
        assert!(text.contains("A\t1\t128\t255\t200\t100"), "{text}");
        assert!(text.contains("B\t2\t100\t200\t100\t50"), "{text}");
        assert!(set_light(&mut doc, 9, 1, [0; 3]).is_err());
    }

    #[test]
    fn light_is_clamped_to_a_byte() {
        let mut doc = TxtDoc::parse(LEVELS.as_bytes());
        set_light(&mut doc, 2, 999, [-5, 0, 300]).unwrap();
        let text = String::from_utf8(doc.to_bytes()).unwrap();
        assert!(text.contains("B\t2\t255\t0\t0\t255"), "{text}");
    }
}
