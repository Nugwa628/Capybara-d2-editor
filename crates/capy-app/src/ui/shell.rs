//! The Map Editor's fixed layout, after ds1-studio: the ribbon on top, the
//! map list on the left (260 px), the Tiles column on the right (320 px),
//! the map in the centre under the Show bar, and a 26 px status bar. Both
//! side columns can be resized and folded to a 30 px strip (Shift+F folds
//! both). The Log, Objects and Sprite preview panels are floating windows;
//! Objects also fills the right column while the Objects tool is active.

use std::collections::{BTreeSet, HashSet, VecDeque};

use bevy_egui::egui::{
    self, Align, CornerRadius, Frame, Id, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2,
};

use super::commands::{self, Bindings, LayerName};
use super::theme::{self, Shades, Theme, icon};
use super::{finder, history, layers, log, map_view, maps, objects, properties, ribbon};
use super::{shortcuts_dialog, sprites, status, tiles};
use crate::config::{Config, SIDE_WIDTH};
use crate::doc::LayerKey;
use crate::edit::tools::Tool;
use crate::state::AppState;

/// Width of a folded side column.
pub const FOLDED: f32 = 30.0;
/// Height of the status bar.
pub const STATUS_HEIGHT: f32 = 26.0;

/// What "show only this layer" replaced, to put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Solo {
    pub doc: u64,
    pub layer: LayerName,
    pub hidden: BTreeSet<LayerKey>,
    pub roofs: bool,
    pub specials: bool,
}

/// The Ctrl+K command finder.
#[derive(Default)]
pub struct Finder {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub focus: bool,
}

impl Finder {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.selected = 0;
            self.focus = true;
        }
    }
}

/// The Shortcuts dialog.
#[derive(Default)]
pub struct ShortcutsUi {
    pub open: bool,
    /// The command waiting for its new key.
    pub capturing: Option<&'static str>,
    pub filter: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Fonts {
    #[default]
    Unset,
    /// Set this frame; egui uses them from the next one.
    Pending,
    Ready,
}

/// Everything about the Map Editor's shell that is not saved.
pub struct ShellState {
    pub bindings: Bindings,
    fonts: Fonts,
    /// The theme on screen (`(id, custom theme count)`), to notice changes.
    applied: Option<(String, usize)>,
    pub shades: Shades,
    pub finder: Finder,
    pub shortcuts: ShortcutsUi,
    pub about_open: bool,
    pub log_window: bool,
    pub objects_window: bool,
    pub preview_window: bool,
    /// The tool the Objects toggle returns to.
    pub prev_tool: Tool,
    pub solo: Option<Solo>,
    /// Folders of the map list that are open (`act1/town`).
    pub open_folders: HashSet<String>,
    /// Ribbon buttons drawn this frame: (id, rect, enabled), for scripts.
    pub ribbon_rects: Vec<(&'static str, egui::Rect, bool)>,
    /// Input events scripts feed in, one batch per frame.
    pub inject: VecDeque<Vec<egui::Event>>,
    /// Ribbon buttons clicked and commands run from keys (scripts check them).
    pub clicked: Vec<&'static str>,
    pub ran: Vec<&'static str>,
    /// The Preferences' share-code box.
    pub theme_code: String,
    /// A scripted "click every ribbon button" in progress.
    pub click_all: Option<super::ribbon::ClickAll>,
    /// Close every open popup menu next frame (scripts).
    pub close_popups: bool,
    pub minimap: Option<super::minimap::Cache>,
    pub map_tree: Option<super::maps::Tree>,
    /// The map whose folder the list last opened.
    pub revealed_doc: Option<u64>,
    /// A scripted click or key press in progress.
    pub probe: Option<crate::script_run::Probe>,
    /// Height the right column's sections needed last frame.
    pub sections_height: f32,
}

/// The ribbon's tab ids.
pub const RIBBON_TABS: [&str; 6] = ["home", "view", "map", "game", "diag", "help"];

pub use super::ribbon::{click_all_step, click_events};

pub fn new_click_all() -> super::ribbon::ClickAll {
    super::ribbon::ClickAll::new()
}

impl ShellState {
    pub fn new(config: &Config) -> ShellState {
        ShellState {
            bindings: Bindings::new(&config.ui.keys),
            fonts: Fonts::Unset,
            applied: None,
            shades: Shades::of(&current_theme(config)),
            finder: Finder::default(),
            shortcuts: ShortcutsUi::default(),
            about_open: false,
            log_window: false,
            objects_window: false,
            preview_window: false,
            prev_tool: Tool::SelectBox,
            solo: None,
            open_folders: HashSet::new(),
            ribbon_rects: Vec::new(),
            inject: VecDeque::new(),
            clicked: Vec::new(),
            ran: Vec::new(),
            theme_code: String::new(),
            click_all: None,
            close_popups: false,
            minimap: None,
            map_tree: None,
            revealed_doc: None,
            probe: None,
            sections_height: 200.0,
        }
    }

    /// The command finder or the Shortcuts dialog's key capture holds the
    /// keyboard.
    pub fn blocks_keys(&self) -> bool {
        self.finder.open || self.shortcuts.capturing.is_some()
    }

    pub fn fonts_ready(&self) -> bool {
        self.fonts == Fonts::Ready
    }

    /// The key text of `id` for tooltips (`""` when it has none).
    pub fn key_of(&self, id: &str) -> String {
        self.bindings.label(id)
    }

    /// Close every window and popup the shell opened (scripts).
    pub fn close_windows(&mut self) {
        self.finder.open = false;
        self.shortcuts.open = false;
        self.shortcuts.capturing = None;
        self.about_open = false;
        self.log_window = false;
        self.objects_window = false;
        self.preview_window = false;
    }
}

/// Close every dialog, window and popup and drop a paste in progress
/// (scripts, between steps).
pub fn close_dialogs(state: &mut AppState) {
    state.shell.close_windows();
    state.tools.close_all();
    state.shell.close_popups = true;
    state.settings.open = false;
    state.settings.add = None;
    state.history.prompt = None;
    state.edit.move_dialog = None;
    state.edit.resize_dialog = None;
    state.edit.confirm_packed = false;
    state.edit.close_prompt = None;
    state.browser.palette.open = false;
    state.cancel_paste();
}

pub fn current_theme(config: &Config) -> Theme {
    theme::find_theme(&config.ui.theme, &config.ui.custom_themes)
}

/// Once per frame before anything is drawn: fonts, theme, and the requests
/// other parts of the app left for the shell.
pub fn prepare_frame(ctx: &egui::Context, state: &mut AppState) {
    match state.shell.fonts {
        Fonts::Unset => {
            if !theme::install_fonts(ctx) {
                state.info("Segoe UI not found; using egui's font");
            }
            state.shell.fonts = Fonts::Pending;
        }
        Fonts::Pending => state.shell.fonts = Fonts::Ready,
        Fonts::Ready => {}
    }
    let key = (
        state.config.ui.theme.clone(),
        state.config.ui.custom_themes.len(),
    );
    if state.shell.applied.as_ref() != Some(&key) {
        let t = current_theme(&state.config);
        theme::apply(ctx, &t);
        state.shell.shades = Shades::of(&t);
        state.shell.applied = Some(key);
    }
    state.shell.ribbon_rects.clear();
    if std::mem::take(&mut state.shell.close_popups) {
        egui::Popup::close_all(ctx);
    }

    let ui = &mut state.config.ui;
    if std::mem::take(&mut state.history.focus) {
        ui.sections.history = true;
        ui.right_folded = false;
        state.config_dirty = true;
    }
    if std::mem::take(&mut state.browser.focus_tiles) && ui.right_folded {
        ui.right_folded = false;
        state.config_dirty = true;
    }
    if std::mem::take(&mut state.objects.focus) && state.config.ui.right_folded {
        state.config.ui.right_folded = false;
        state.config_dirty = true;
    }
    if std::mem::take(&mut state.sprites.show_preview_panel) {
        state.shell.preview_window = true;
    }
}

/// Brings a panel forward by its old dock title (`CAPY_TAB`). Returns false
/// for an unknown title.
pub fn focus_panel(state: &mut AppState, title: &str) -> bool {
    let ui = &mut state.config.ui;
    match title.to_ascii_lowercase().as_str() {
        "map" | "maps" => ui.left_folded = false,
        "tiles" => ui.right_folded = false,
        "layers" => {
            ui.right_folded = false;
            ui.sections.layers = true;
        }
        "properties" => {
            ui.right_folded = false;
            ui.sections.map = true;
        }
        "history" => {
            ui.right_folded = false;
            ui.sections.history = true;
        }
        "log" => state.shell.log_window = true,
        "objects" => state.shell.objects_window = true,
        "sprite preview" => state.shell.preview_window = true,
        _ => return false,
    }
    true
}

fn column_frame(s: &Shades) -> Frame {
    Frame::NONE.fill(s.panel).stroke(Stroke::new(1.0, s.border))
}

/// Draw the Map Editor.
pub fn show(root: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    egui::Panel::top("ribbon")
        .frame(Frame::NONE.fill(s.panel).stroke(Stroke::new(1.0, s.border)))
        .resizable(false)
        .show_separator_line(false)
        .show(root, |ui| ribbon::show(ui, state));
    egui::Panel::bottom("status_bar")
        .exact_size(STATUS_HEIGHT)
        .frame(
            Frame::NONE
                .fill(s.panel)
                .stroke(Stroke::new(1.0, s.border))
                .inner_margin(Margin::symmetric(12, 0)),
        )
        .show_separator_line(false)
        .show(root, |ui| status::show_map(ui, state));
    left_column(root, state);
    right_column(root, state);
    egui::CentralPanel::no_frame().show(root, |ui| centre(ui, state));
}

/// The 30 px strip of a folded column: one button that unfolds it.
fn folded_strip(ui: &mut Ui, state: &mut AppState, glyph: char, tip: &str) -> bool {
    ui.add_space(6.0);
    ui.vertical_centered(|ui| icon_button(ui, state, glyph, tip).clicked())
        .inner
}

/// A small square icon button.
pub fn icon_button(ui: &mut Ui, state: &AppState, glyph: char, tip: &str) -> egui::Response {
    let s = state.shell.shades;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, s.hover);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        theme::icon_font(14.0, state.shell.fonts_ready()),
        if resp.hovered() { s.text } else { s.muted },
    );
    resp.on_hover_text(tip)
}

/// Keep a column's width in the config once the user dragged it.
fn remember_width(state: &mut AppState, left: bool, width: f32) {
    let width = width.clamp(*SIDE_WIDTH.start(), *SIDE_WIDTH.end());
    let ui = &mut state.config.ui;
    let slot = if left {
        &mut ui.left_width
    } else {
        &mut ui.right_width
    };
    if (*slot - width).abs() > 0.5 {
        *slot = width;
        state.config_dirty = true;
    }
}

fn left_column(root: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    if state.config.ui.left_folded {
        egui::Panel::left("left_folded")
            .exact_size(FOLDED)
            .resizable(false)
            .frame(column_frame(&s))
            .show(root, |ui| {
                if folded_strip(ui, state, icon::CHEVRON_RIGHT, "Show the map list") {
                    state.config.ui.left_folded = false;
                    state.config_dirty = true;
                }
            });
        return;
    }
    let width = state.config.ui.left_width;
    let resp = egui::Panel::left("left_column")
        .resizable(true)
        .default_size(width)
        .size_range(SIDE_WIDTH)
        .frame(column_frame(&s).inner_margin(Margin::symmetric(0, 6)))
        .show(root, |ui| maps::show(ui, state));
    remember_width(state, true, resp.response.rect.width());
}

fn right_column(root: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    if state.config.ui.right_folded {
        egui::Panel::right("right_folded")
            .exact_size(FOLDED)
            .resizable(false)
            .frame(column_frame(&s))
            .show(root, |ui| {
                if folded_strip(ui, state, icon::CHEVRON_LEFT, "Show the tiles column") {
                    state.config.ui.right_folded = false;
                    state.config_dirty = true;
                }
            });
        return;
    }
    let width = state.config.ui.right_width;
    let resp = egui::Panel::right("right_column")
        .resizable(true)
        .default_size(width)
        .size_range(SIDE_WIDTH)
        .frame(column_frame(&s))
        .show(root, |ui| right_contents(ui, state));
    remember_width(state, false, resp.response.rect.width());
}

/// A tab header of the right column (TILES / PRESETS / OBJECTS).
fn column_tab(
    ui: &mut Ui,
    state: &AppState,
    label: &str,
    on: bool,
    enabled: bool,
) -> egui::Response {
    let s = state.shell.shades;
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(w, 34.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let color = match (on, enabled) {
        (true, _) => s.accent_text,
        (false, true) if resp.hovered() => s.text,
        (false, true) => s.muted,
        (false, false) => s.muted.gamma_multiply(0.5),
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        color,
    );
    if on {
        let line = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 2.0),
            rect.right_bottom(),
        );
        ui.painter().rect_filled(line, 0.0, s.accent);
    }
    resp
}

fn right_contents(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    let objects_tool = state.edit.tool == Tool::Objects;
    egui::Panel::top("right_tabs")
        .frame(Frame::NONE.stroke(Stroke::new(1.0, s.border)))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.columns(2, |cols| {
                if objects_tool {
                    column_tab(&mut cols[0], state, "OBJECTS", true, true);
                    if column_tab(&mut cols[1], state, "TILES", false, true)
                        .on_hover_text("Back to the tiles (leaves the Objects tool)")
                        .clicked()
                    {
                        commands::run(state, "mode.tiles");
                    }
                    return;
                }
                let presets = state.tools.presets_tab;
                if column_tab(&mut cols[0], state, "TILES", !presets, true).clicked() {
                    state.tools.presets_tab = false;
                }
                if column_tab(&mut cols[1], state, "PRESETS", presets, true)
                    .on_hover_text("Saved blocks of tiles to place on the map")
                    .clicked()
                {
                    state.tools.presets_tab = true;
                }
            });
        });
    // The sections take what they needed last frame, up to 60% of the column.
    let max_sections = (ui.available_height() * 0.6).max(120.0);
    let height = state.shell.sections_height.clamp(34.0, max_sections);
    egui::Panel::bottom("right_sections")
        .frame(Frame::NONE.stroke(Stroke::new(1.0, s.border)))
        .resizable(false)
        .exact_size(height)
        .show_separator_line(false)
        .show(ui, |ui| {
            let out = egui::ScrollArea::vertical()
                .id_salt("right_sections_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| sections(ui, state));
            state.shell.sections_height = out.content_size.y;
        });
    egui::CentralPanel::default()
        .frame(Frame::NONE.inner_margin(Margin::symmetric(10, 8)))
        .show(ui, |ui| {
            if objects_tool {
                objects::show(ui, state);
            } else if state.tools.presets_tab {
                super::tools::presets::show(ui, state);
            } else {
                tiles::show(ui, state);
            }
        });
}

/// A collapsible section header: chevron, CAPS title, summary on the right.
/// Returns whether the section is open after the click.
fn section_header(ui: &mut Ui, state: &AppState, title: &str, summary: &str, open: bool) -> bool {
    let s = state.shell.shades;
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, s.hover);
    }
    let painter = ui.painter();
    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, s.border),
    );
    let glyph = if open {
        icon::CHEVRON_DOWN
    } else {
        icon::CHEVRON_RIGHT
    };
    painter.text(
        rect.left_center() + Vec2::new(16.0, 0.0),
        egui::Align2::CENTER_CENTER,
        glyph,
        theme::icon_font(11.0, state.shell.fonts_ready()),
        s.muted,
    );
    painter.text(
        rect.left_center() + Vec2::new(28.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(12.0),
        s.heading,
    );
    painter.text(
        rect.right_center() - Vec2::new(14.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        summary,
        egui::FontId::proportional(11.5),
        s.muted,
    );
    if resp.clicked() { !open } else { open }
}

fn section(
    ui: &mut Ui,
    state: &mut AppState,
    title: &str,
    summary: String,
    get: fn(&mut crate::config::Sections) -> &mut bool,
    body: impl FnOnce(&mut Ui, &mut AppState),
) {
    let open = *get(&mut state.config.ui.sections);
    let now = section_header(ui, state, title, &summary, open);
    if now != open {
        *get(&mut state.config.ui.sections) = now;
        state.config_dirty = true;
    }
    if now {
        Frame::NONE
            .inner_margin(Margin {
                left: 14,
                right: 12,
                top: 2,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.push_id(title, |ui| body(ui, state));
            });
    }
}

fn sections(ui: &mut Ui, state: &mut AppState) {
    let cell = state
        .hover_cell
        .map_or_else(String::new, |(x, y)| format!("{x}, {y}"));
    section(ui, state, "CELL", cell, |s| &mut s.cell, cell_section);
    let history = state
        .active_doc()
        .and_then(|d| d.edit.undo_label().map(|l| format!("undo: {l}")))
        .unwrap_or_else(|| "no edits".to_owned());
    section(
        ui,
        state,
        "HISTORY",
        history,
        |s| &mut s.history,
        |ui, state| {
            history::show(ui, state);
        },
    );
    let tags = match state.active_doc() {
        Some(d) if d.ds1.tag.is_some() || !d.ds1.groups.is_empty() => format!(
            "{}{} group(s)",
            if d.ds1.tag.is_some() {
                "tag layer, "
            } else {
                ""
            },
            d.ds1.groups.len()
        ),
        _ => "none".to_owned(),
    };
    section(
        ui,
        state,
        "TAGS & GROUPS",
        tags,
        |s| &mut s.tags,
        tags_section,
    );
    let layers = state
        .active_doc()
        .map_or_else(String::new, |d| d.keys().len().to_string());
    section(
        ui,
        state,
        "LAYERS",
        layers,
        |s| &mut s.layers,
        |ui, state| {
            layers::show(ui, state);
        },
    );
    let map = state
        .active_doc()
        .map_or_else(String::new, |d| format!("v{}", d.ds1.version));
    section(
        ui,
        state,
        "MAP",
        map,
        |s| &mut s.map,
        |ui, state| {
            properties::show(ui, state);
        },
    );
}

fn cell_section(ui: &mut Ui, state: &mut AppState) {
    let Some(doc) = state.active_doc() else {
        ui.weak("No map open");
        return;
    };
    match state.hover_cell {
        Some((x, y)) if x >= 0 && y >= 0 => {
            let lines = properties::describe_cell(&doc.ds1, x as u32, y as u32);
            if lines.is_empty() {
                ui.weak("Empty cell");
            }
            for l in lines {
                ui.label(RichText::new(l).monospace());
            }
        }
        _ => {
            ui.weak("Point at a cell on the map");
        }
    }
}

fn tags_section(ui: &mut Ui, state: &mut AppState) {
    let Some(doc) = state.active_doc() else {
        ui.weak("No map open");
        return;
    };
    let ds1 = &doc.ds1;
    match (&ds1.tag, state.hover_cell) {
        (Some(tag), Some((x, y))) if x >= 0 && y >= 0 => {
            let value = tag.get(x as u32, y as u32).unwrap_or(0);
            ui.label(format!("Tag at {x}, {y}: {value:#010x}"));
        }
        (Some(_), _) => {
            ui.weak("Tag layer: point at a cell to read it");
        }
        (None, _) => {
            ui.weak("No tag layer (add one in Map > Layers)");
        }
    }
    if ds1.groups.is_empty() {
        ui.weak("No object groups");
        return;
    }
    egui::Grid::new("groups").num_columns(2).show(ui, |ui| {
        for (i, g) in ds1.groups.iter().enumerate() {
            ui.weak(format!("#{i}"));
            ui.label(format!("{}, {}  {} x {}", g.x, g.y, g.width, g.height));
            ui.end_row();
        }
    });
}

/// The centre: map tabs and the Show bar over the map.
fn centre(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    Frame::NONE
        .fill(s.bg2)
        .stroke(Stroke::new(1.0, s.border))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            map_view::tab_strip(ui, state);
            show_bar(ui, state);
        });
    map_view::show(ui, state);
}

/// A rounded chip (ds1-studio's `.chip`).
pub fn chip(ui: &mut Ui, s: &Shades, label: &str, on: bool, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(11.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, s.text);
    let size = Vec2::new(galley.size().x + 18.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let (fill, stroke, text) = if on {
        (s.accent_dim, s.accent, s.accent_text)
    } else if resp.hovered() && enabled {
        (s.hover, s.border2, s.text)
    } else {
        (s.panel2, s.border, if enabled { s.text } else { s.muted })
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, text);
    resp
}

/// "Show [Floor 1] [Wall 1] ..." layer chips: click toggles, Shift-click
/// shows only that layer (again: back as it was).
fn show_bar(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    let Some(doc) = state.active_doc() else {
        return;
    };
    let mut chips: Vec<(String, LayerName, Option<&'static str>)> = doc
        .keys()
        .into_iter()
        .filter(|k| *k != LayerKey::Tag)
        .map(|k| {
            let id = match k {
                LayerKey::Floor(0) => Some("layer.floor1"),
                LayerKey::Floor(1) => Some("layer.floor2"),
                LayerKey::Wall(0) => Some("layer.wall1"),
                LayerKey::Wall(1) => Some("layer.wall2"),
                LayerKey::Wall(2) => Some("layer.wall3"),
                LayerKey::Wall(3) => Some("layer.wall4"),
                LayerKey::Shadow => Some("layer.shadows"),
                _ => None,
            };
            let label = match k {
                LayerKey::Shadow => "Shadows".to_owned(),
                other => other.label(),
            };
            (label, LayerName::Key(k), id)
        })
        .collect();
    chips.push(("Roofs".to_owned(), LayerName::Roofs, Some("layer.roofs")));
    chips.push((
        "Special tiles".to_owned(),
        LayerName::Specials,
        Some("layer.specials"),
    ));
    let tag = doc.keys().contains(&LayerKey::Tag);
    let objects_on = doc.objects_visible;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("Show").color(s.muted).size(11.5));
        ui.add_space(4.0);
        for (label, layer, id) in chips {
            let on = commands::layer_shown(state, layer);
            let key = id.map(|i| state.shell.key_of(i)).unwrap_or_default();
            let tip = if key.is_empty() {
                "Click: show or hide. Shift+click: show only this layer".to_owned()
            } else {
                format!(
                    "Click: show or hide ({key}). Shift+click: show only this layer (Shift+{key})"
                )
            };
            let resp = chip(ui, &s, &label, on, true).on_hover_text(tip);
            if resp.clicked() {
                if ui.input(|i| i.modifiers.shift) {
                    commands::solo(state, layer);
                } else {
                    commands::set_layer_shown(state, layer, !on);
                }
            }
        }
        if tag {
            let on = commands::layer_shown(state, LayerName::Key(LayerKey::Tag));
            if chip(ui, &s, "Tag", on, true).clicked() {
                commands::set_layer_shown(state, LayerName::Key(LayerKey::Tag), !on);
            }
        }
        ui.add_space(6.0);
        if chip(ui, &s, "Objects", objects_on, true)
            .on_hover_text(format!(
                "Object sprites ({})",
                state.shell.key_of("view.sprites")
            ))
            .clicked()
        {
            commands::run(state, "view.sprites");
        }
    });
}

/// The floating windows of the Map Editor.
pub fn windows(ctx: &egui::Context, state: &mut AppState) {
    let mut open = state.shell.log_window;
    egui::Window::new("Log")
        .id(Id::new("log_window"))
        .open(&mut open)
        .default_size([620.0, 260.0])
        .default_pos([300.0, 520.0])
        .show(ctx, |ui| log::show(ui, state));
    state.shell.log_window = open;

    let mut open = state.shell.objects_window;
    egui::Window::new("Objects")
        .id(Id::new("objects_window"))
        .open(&mut open)
        .default_size([340.0, 480.0])
        .default_pos([320.0, 160.0])
        .show(ctx, |ui| objects::show(ui, state));
    state.shell.objects_window = open;

    let mut open = state.shell.preview_window;
    egui::Window::new("Sprite preview")
        .id(Id::new("sprite_preview_window"))
        .open(&mut open)
        .default_size([360.0, 360.0])
        .default_pos([340.0, 180.0])
        .show(ctx, |ui| sprites::preview::show(ui, state));
    state.shell.preview_window = open;

    let mut open = state.shell.about_open;
    egui::Window::new("About")
        .id(Id::new("about_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, about);
    state.shell.about_open = open;

    finder::show(ctx, state);
    shortcuts_dialog::show(ctx, state);
}

fn about(ui: &mut Ui) {
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.heading(crate::PRODUCT);
        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(6.0);
        ui.label("A Diablo II map and sprite editor.");
        ui.weak("The Map Editor's layout follows DS1 Studio by RoofooEvazan (MIT).");
        ui.weak("Icons: Lucide (ISC). Third-party notices: THIRD-PARTY-LICENSES.md.");
    });
}
