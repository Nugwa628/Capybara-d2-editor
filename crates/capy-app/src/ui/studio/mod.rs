//! ds1-studio parity in the Map Editor: map editing (P9) and views and
//! overlays (P10).
//!
//! * [`paint`]: Rectangle (U) and Fill area (L) paint/erase, random-mix
//!   painting, dragging the map edge to resize, Shift+wheel through stacked
//!   tiles and double-click to select every copy of a tile.
//! * [`dialogs`]: Replace (Ctrl+H), Re-roll floors, Reassign tiles, New map.
//! * [`views`]: the view modes (Tiles, Objects, Walkability, Automap, Level
//!   light, Roof hiding) with their overlays, the 8x8 rooms grid, game-view
//!   framing, warps, spawn regions and the walkable-area badge.
//! * [`sections`]: the right column's Cell and Tags & groups sections.
//! * [`export`]: PNG of the map or the selection, the view to the clipboard.
//! * [`wallcats`]: the DT1 wall-categories row under the Show bar.
//!
//! Everything is reached through the command registry: [`run`], [`enabled`]
//! and [`active`] answer for the ids this module owns.

pub mod checks;
pub mod dialogs;
pub mod export;
pub mod levels;
pub mod libs;
pub mod modes;
pub mod paint;
pub mod script;
pub mod sections;
pub mod stack;
pub mod views;
pub mod walkplan;
pub mod wallcats;

use std::collections::BTreeMap;

use capy_edit::{CellValue, LayerRef};
use serde::{Deserialize, Serialize};

use crate::edit::tools::Tool;
use crate::state::AppState;

/// Lucide glyphs used here (the font holds the whole set).
pub mod icons {
    pub const SQUARE: char = '\u{e167}';
    pub const PAINT_BUCKET: char = '\u{e2e6}';
    pub const REPLACE: char = '\u{e3db}';
    pub const FOOTPRINTS: char = '\u{e3b9}';
    pub const MAP: char = '\u{e110}';
    pub const FILE_PLUS: char = '\u{e0c9}';
    pub const DICES: char = '\u{e2c5}';
    pub const SHUFFLE: char = '\u{e15e}';
    pub const IMAGE_DOWN: char = '\u{e53c}';
    pub const CAMERA: char = '\u{e064}';
    pub const SUN: char = '\u{e178}';
    pub const DOOR_OPEN: char = '\u{e3d6}';
    pub const FRAME: char = '\u{e291}';
    pub const MONITOR: char = '\u{e11d}';
    pub const HOME: char = '\u{e0f5}';
    pub const LAYOUT_GRID: char = '\u{e0ff}';
    pub const WAYPOINTS: char = '\u{e542}';
}

/// How Paint and Erase apply (ds1-studio's Paint/erase group).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintMode {
    /// Follows the mouse (B / E).
    #[default]
    Freehand,
    /// Drag a rectangle (U).
    Rect,
    /// The connected area of the same tile (L).
    Fill,
}

/// The view modes (Tab / Shift+Tab, F1-F6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Tiles,
    Objects,
    Walk,
    Automap,
    Light,
    Roofs,
}

impl ViewMode {
    pub const ALL: [ViewMode; 6] = [
        ViewMode::Tiles,
        ViewMode::Objects,
        ViewMode::Walk,
        ViewMode::Automap,
        ViewMode::Light,
        ViewMode::Roofs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Tiles => "Tiles",
            ViewMode::Objects => "Objects",
            ViewMode::Walk => "Walkability",
            ViewMode::Automap => "Automap",
            ViewMode::Light => "Level light",
            ViewMode::Roofs => "Roof hiding",
        }
    }
}

/// Saved with the config (`[ui.studio]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StudioPrefs {
    /// The screen size Game view frames.
    pub game_width: u32,
    pub game_height: u32,
    /// The DT1 wall-categories row under the Show bar is open.
    pub wall_row: bool,
    /// DT1 path (lower case) -> "upper" / "lower": wall tiles of that DT1
    /// belong to that group whatever their orientation.
    pub wall_categories: BTreeMap<String, String>,
    pub spawn_regions: bool,
    pub warps: bool,
}

impl Default for StudioPrefs {
    fn default() -> Self {
        StudioPrefs {
            game_width: 800,
            game_height: 600,
            wall_row: false,
            wall_categories: BTreeMap::new(),
            spawn_regions: false,
            warps: true,
        }
    }
}

/// Everything about P9/P10 that is not saved.
pub struct StudioState {
    pub paint_mode: PaintMode,
    /// Ctrl+clicked tiles: Paint picks one of them at random per cell.
    pub mix: Vec<(LayerRef, CellValue)>,
    /// The view mode other than Objects (that one is the Objects tool).
    pub view: ViewMode,
    pub rooms: bool,
    pub game_view: bool,
    /// Roof hiding: show the hiding tiles as hidden ("as if inside").
    pub pops_inside: bool,
    pub upper_walls: bool,
    pub lower_walls: bool,
    /// Seed of the next random-mix edit.
    pub seed: u64,
    /// The cell the Cell section shows while the pointer is off the map.
    pub pinned_cell: Option<(u32, u32)>,
    pub drag: Option<paint::Drag>,
    /// Shift+wheel: the stacked tile last stepped to at a cell.
    pub stack: Option<((u32, u32), usize)>,
    pub stack_accum: f32,
    /// Alt-painting: the layer each cell of the stroke went to.
    pub stack_placed: Option<stack::Placed>,
    /// Alt was held when a Rectangle drag started.
    pub stack_rect: bool,
    pub modes: modes::Modes,
    pub dialogs: dialogs::Dialogs,
    pub sections: sections::SectionsState,
    pub analysis: views::Analysis,
    pub levels: levels::Levels,
    pub export: export::ExportState,
    pub minimap: views::MinimapRender,
}

impl Default for StudioState {
    fn default() -> Self {
        StudioState {
            paint_mode: PaintMode::Freehand,
            mix: Vec::new(),
            view: ViewMode::Tiles,
            rooms: false,
            game_view: false,
            pops_inside: false,
            upper_walls: true,
            lower_walls: true,
            seed: 1,
            pinned_cell: None,
            drag: None,
            stack: None,
            stack_accum: 0.0,
            stack_placed: None,
            stack_rect: false,
            modes: modes::Modes::default(),
            dialogs: dialogs::Dialogs::default(),
            sections: sections::SectionsState::default(),
            analysis: views::Analysis::default(),
            levels: levels::Levels::default(),
            export: export::ExportState::default(),
            minimap: views::MinimapRender::default(),
        }
    }
}

impl StudioState {
    pub fn next_seed(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(1);
        self.seed
    }
}

/// The view mode on screen.
pub fn current_view(state: &AppState) -> ViewMode {
    if state.edit.tool == Tool::Objects {
        ViewMode::Objects
    } else if state.studio.view == ViewMode::Objects {
        ViewMode::Tiles
    } else {
        state.studio.view
    }
}

/// Switch to `mode` (Objects is the Objects tool; leaving it returns to the
/// tool used before).
pub fn set_view(state: &mut AppState, mode: ViewMode) {
    let objects = state.edit.tool == Tool::Objects;
    if mode == ViewMode::Objects {
        if !objects {
            super::commands::run(state, "tool.object");
        }
    } else if objects {
        super::commands::run(state, "mode.tiles");
    }
    state.studio.view = mode;
}

fn cycle_view(state: &mut AppState, forward: bool) {
    let all = ViewMode::ALL;
    let now = all
        .iter()
        .position(|m| *m == current_view(state))
        .unwrap_or(0);
    let n = all.len();
    let next = if forward {
        (now + 1) % n
    } else {
        (now + n - 1) % n
    };
    set_view(state, all[next]);
}

fn toggle_view(state: &mut AppState, mode: ViewMode) {
    if current_view(state) == mode {
        set_view(state, ViewMode::Tiles);
    } else {
        set_view(state, mode);
    }
}

fn set_paint_mode(state: &mut AppState, mode: PaintMode) {
    super::map_tools::cancel_gesture(state);
    state.studio.drag = None;
    state.studio.paint_mode = mode;
    if !state.edit.tool.brushes() {
        super::commands::run(state, "tool.paint");
    }
}

fn prefs(state: &mut AppState) -> &mut StudioPrefs {
    state.config_dirty = true;
    &mut state.config.ui.studio
}

/// The ids this module runs. `None`: not one of ours.
pub fn run(state: &mut AppState, id: &str) -> Option<bool> {
    match id {
        "tool.rect" => set_paint_mode(state, PaintMode::Rect),
        "tool.fill" => set_paint_mode(state, PaintMode::Fill),
        "tool.freehand" => set_paint_mode(state, PaintMode::Freehand),
        "mix.clear" => state.studio.mix.clear(),
        "edit.replace" => dialogs::open_replace(state),
        "map.new" => dialogs::open_new_map(state),
        "map.reroll" => dialogs::open_reroll(state),
        "map.reassign" => dialogs::open_reassign(state),
        "export.png" => export::export_png(state, false, None),
        "export.selectionPng" => export::export_png(state, true, None),
        "view.snapshot" => state.studio.export.copy_view = true,
        "view.next" => cycle_view(state, true),
        "view.prev" => cycle_view(state, false),
        "mode.walk" => set_view(state, ViewMode::Walk),
        "mode.automap" => set_view(state, ViewMode::Automap),
        "mode.light" => set_view(state, ViewMode::Light),
        "mode.roofs" => set_view(state, ViewMode::Roofs),
        "view.walkable" => toggle_view(state, ViewMode::Walk),
        "view.automap" => toggle_view(state, ViewMode::Automap),
        "view.light" => toggle_view(state, ViewMode::Light),
        "view.pops" => toggle_view(state, ViewMode::Roofs),
        "view.popsInside" => state.studio.pops_inside = !state.studio.pops_inside,
        "view.rooms" => state.studio.rooms = !state.studio.rooms,
        "view.game" => views::toggle_game_view(state),
        "view.spawn" => {
            let p = prefs(state);
            p.spawn_regions = !p.spawn_regions;
        }
        "view.warps" => {
            let p = prefs(state);
            p.warps = !p.warps;
        }
        "view.wallCategories" => {
            let p = prefs(state);
            p.wall_row = !p.wall_row;
        }
        "layer.lowerWalls" => state.studio.lower_walls = !state.studio.lower_walls,
        "layer.upperWalls" => state.studio.upper_walls = !state.studio.upper_walls,
        "solo.lowerWalls" => wallcats::solo_lower(state),
        _ => return None,
    }
    Some(true)
}

/// Whether one of our ids can run now. `None`: not one of ours.
pub fn enabled(state: &AppState, id: &str) -> Option<bool> {
    let doc = state.active_doc();
    let has_doc = doc.is_some();
    Some(match id {
        "mix.clear" => !state.studio.mix.is_empty(),
        "map.new" => state.session.is_some(),
        "export.selectionPng" => doc.is_some_and(|d| !d.selection.is_empty()),
        "export.png" => doc.is_some_and(|d| d.tiles.is_some()),
        "map.reroll" => doc.is_some_and(|d| d.tiles.is_some()),
        "view.snapshot" => has_doc && state.map_rect.is_some(),
        "tool.rect"
        | "tool.fill"
        | "tool.freehand"
        | "edit.replace"
        | "map.reassign"
        | "view.next"
        | "view.prev"
        | "mode.walk"
        | "mode.automap"
        | "mode.light"
        | "mode.roofs"
        | "view.walkable"
        | "view.automap"
        | "view.light"
        | "view.pops"
        | "view.popsInside"
        | "view.rooms"
        | "view.game"
        | "view.spawn"
        | "view.warps"
        | "view.wallCategories"
        | "layer.lowerWalls"
        | "layer.upperWalls"
        | "solo.lowerWalls" => has_doc,
        _ => return None,
    })
}

/// Whether one of our toggles is on. `None`: not one of ours.
pub fn active(state: &AppState, id: &str) -> Option<bool> {
    let s = &state.studio;
    let p = &state.config.ui.studio;
    let brush = state.edit.tool.brushes();
    let view = current_view(state);
    Some(match id {
        "tool.rect" => brush && s.paint_mode == PaintMode::Rect,
        "tool.fill" => brush && s.paint_mode == PaintMode::Fill,
        "tool.freehand" => brush && s.paint_mode == PaintMode::Freehand,
        "mode.walk" | "view.walkable" => view == ViewMode::Walk,
        "mode.automap" | "view.automap" => view == ViewMode::Automap,
        "mode.light" | "view.light" => view == ViewMode::Light,
        "mode.roofs" | "view.pops" => view == ViewMode::Roofs,
        "view.popsInside" => s.pops_inside,
        "view.rooms" => s.rooms,
        "view.game" => s.game_view,
        "view.spawn" => p.spawn_regions,
        "view.warps" => p.warps,
        "view.wallCategories" => p.wall_row,
        "layer.lowerWalls" => s.lower_walls,
        "layer.upperWalls" => s.upper_walls,
        _ => return None,
    })
}

/// The status bar's hint for this module's modes, when one applies.
pub fn status_hint(state: &AppState) -> Option<&'static str> {
    let brush = state.edit.tool.brushes();
    let erase = state.edit.tool == Tool::Erase;
    Some(match (brush, state.studio.paint_mode, erase) {
        (true, PaintMode::Rect, false) => {
            "Drag a rectangle to paint it (Ctrl+click tiles to paint a random mix)"
        }
        (true, PaintMode::Rect, true) => "Drag a rectangle to erase it on the active layer",
        (true, PaintMode::Fill, false) => {
            "Click to fill the connected area of the same tile (inside the selection when you click in it)"
        }
        (true, PaintMode::Fill, true) => "Click to erase the connected area of the same tile",
        _ => match current_view(state) {
            ViewMode::Walk => {
                "Walkability: green walkable, red blocked, yellow monsters only; paint with the Cell and Sub-tile tools"
            }
            ViewMode::Automap => {
                "Automap: pick Paint, Pick or Clear in the panel, then click cells"
            }
            ViewMode::Light => "Level light from Levels.txt: edit it in the panel",
            ViewMode::Roofs => "Roof hiding areas; Shift+H shows the roofs as from inside",
            _ if state.edit.tool == Tool::SelectBox => {
                "Drag to select; double-click selects every copy of a tile; drag a map edge to resize"
            }
            _ => return None,
        },
    })
}

/// Actions of this module that change the map (refused on a snapshot).
pub fn edits_the_map(id: &str) -> bool {
    matches!(id, "edit.replace" | "map.reroll" | "map.reassign")
}

/// Once per frame: keep each map's render filter in step with the wall
/// groups and roof hiding.
pub fn sync(state: &mut AppState) {
    let roofs = current_view(state) == ViewMode::Roofs && state.studio.pops_inside;
    let overrides: Vec<(String, bool)> = state
        .config
        .ui
        .studio
        .wall_categories
        .iter()
        .map(|(k, v)| (k.clone(), v == "lower"))
        .collect();
    let (upper, lower) = (state.studio.upper_walls, state.studio.lower_walls);
    let Some(i) = state.active else { return };
    let hide_cells = if roofs {
        views::pop_hidden_cells(state, i)
    } else {
        std::sync::Arc::new(Vec::new())
    };
    let doc = &mut state.docs[i];
    let want = crate::render::job::TileFilter {
        hide_upper: !upper,
        hide_lower: !lower,
        overrides,
        hide_cells,
    };
    if doc.filter != want {
        doc.filter = want;
    }
}

/// The windows of this module (dialogs).
pub fn windows(ctx: &bevy_egui::egui::Context, state: &mut AppState) {
    sync(state);
    dialogs::show(ctx, state);
    export::poll(state);
}

/// Close every dialog (scripts, between steps).
pub fn close_dialogs(state: &mut AppState) {
    state.studio.dialogs = dialogs::Dialogs::default();
}
