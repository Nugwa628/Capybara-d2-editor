//! The egui shell: mode tabs, menu, status bar, toasts and the dock.
//!
//! Two modes share the window, each with its own dock layout (persisted
//! separately): **Map Editor** (the panels below) and **Sprites** (sprite
//! list, viewer, editor, converter and log; see [`sprites`]).

mod dialogs;
mod layers;
mod log;
mod map_tools;
mod map_view;
mod maps;
mod menu;
mod palette;
mod properties;
mod shortcuts;
mod sprites;
mod status;
mod thumbs;
mod tiles;
mod toasts;
mod toolbar;

use std::path::Path;

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Id, LayerId, Ui, UiBuilder};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::AppMode;
use crate::state::AppState;

pub use map_tools::cancel_gesture as cancel_map_gesture;
pub use thumbs::tile_key as tile_key_for;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    Map,
    Maps,
    Tiles,
    Layers,
    Properties,
    Log,
    /// Optional: the View menu adds and removes it.
    SpritePreview,
}

impl Panel {
    /// Panels every layout must hold exactly once.
    pub const REQUIRED: [Panel; 6] = [
        Panel::Map,
        Panel::Maps,
        Panel::Tiles,
        Panel::Layers,
        Panel::Properties,
        Panel::Log,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Panel::Map => "Map",
            Panel::Maps => "Maps",
            Panel::Tiles => "Tiles",
            Panel::Layers => "Layers",
            Panel::Properties => "Properties",
            Panel::Log => "Log",
            Panel::SpritePreview => "Sprite preview",
        }
    }
}

/// Panels of the Sprites mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpritesPanel {
    Browser,
    Viewer,
    Editor,
    Converter,
    Log,
}

impl SpritesPanel {
    pub const ALL: [SpritesPanel; 5] = [
        SpritesPanel::Browser,
        SpritesPanel::Viewer,
        SpritesPanel::Editor,
        SpritesPanel::Converter,
        SpritesPanel::Log,
    ];

    pub fn title(self) -> &'static str {
        match self {
            SpritesPanel::Browser => "Sprites",
            SpritesPanel::Viewer => "Viewer",
            SpritesPanel::Editor => "Editor",
            SpritesPanel::Converter => "Converter",
            SpritesPanel::Log => "Log",
        }
    }
}

/// The dock layouts of both modes.
#[derive(Resource)]
pub struct Dock {
    pub map: DockState<Panel>,
    pub sprites: DockState<SpritesPanel>,
    /// JSON of the states last written, to detect changes.
    last_map: String,
    last_sprites: String,
}

pub fn default_layout() -> DockState<Panel> {
    let mut state = DockState::new(vec![Panel::Map]);
    let main = state.main_surface_mut();
    let [centre, _left] = main.split_left(NodeIndex::root(), 0.2, vec![Panel::Maps, Panel::Tiles]);
    let [centre, _right] = main.split_right(centre, 0.78, vec![Panel::Layers, Panel::Properties]);
    let [_centre, _bottom] = main.split_below(centre, 0.76, vec![Panel::Log]);
    state
}

pub fn default_sprites_layout() -> DockState<SpritesPanel> {
    let mut state = DockState::new(vec![SpritesPanel::Viewer, SpritesPanel::Editor]);
    let main = state.main_surface_mut();
    let [centre, _list] = main.split_left(NodeIndex::root(), 0.18, vec![SpritesPanel::Browser]);
    let [centre, _converter] = main.split_right(centre, 0.74, vec![SpritesPanel::Converter]);
    let [_centre, _log] = main.split_below(centre, 0.8, vec![SpritesPanel::Log]);
    state
}

/// A layout is usable only if it holds each required panel exactly once and
/// no panel twice.
pub fn layout_is_complete(state: &DockState<Panel>) -> bool {
    let mut seen = Vec::new();
    for (_, tab) in state.iter_all_tabs() {
        if seen.contains(tab) {
            return false;
        }
        seen.push(*tab);
    }
    Panel::REQUIRED.iter().all(|p| seen.contains(p))
}

fn sprites_layout_is_complete(state: &DockState<SpritesPanel>) -> bool {
    let mut seen = Vec::new();
    for (_, tab) in state.iter_all_tabs() {
        if seen.contains(tab) {
            return false;
        }
        seen.push(*tab);
    }
    SpritesPanel::ALL.iter().all(|p| seen.contains(p))
}

/// Reads a saved layout. `Err` carries a message for the log; a missing file
/// is `Ok(None)`.
fn load_state<T: DeserializeOwned>(
    file: &Path,
    complete: impl Fn(&DockState<T>) -> bool,
    what: &str,
) -> (Option<DockState<T>>, Option<String>) {
    match std::fs::read_to_string(file) {
        Ok(text) => match serde_json::from_str::<DockState<T>>(&text) {
            Ok(s) if complete(&s) => (Some(s), None),
            Ok(_) => (
                None,
                Some(format!(
                    "saved {what} layout is incomplete; using the default"
                )),
            ),
            Err(e) => (
                None,
                Some(format!(
                    "saved {what} layout is unreadable ({e}); using the default"
                )),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(e) => (None, Some(format!("cannot read the {what} layout: {e}"))),
    }
}

impl Dock {
    /// Loads both layouts. The returned strings are problems worth a log line.
    pub fn load(map_file: &Path, sprites_file: &Path) -> (Dock, Vec<String>) {
        let (map, p1) = load_state(map_file, layout_is_complete, "map editor");
        let (sprites, p2) = load_state(sprites_file, sprites_layout_is_complete, "sprites");
        let map = map.unwrap_or_else(default_layout);
        let sprites = sprites.unwrap_or_else(default_sprites_layout);
        let last_map = serde_json::to_string(&map).unwrap_or_default();
        let last_sprites = serde_json::to_string(&sprites).unwrap_or_default();
        (
            Dock {
                map,
                sprites,
                last_map,
                last_sprites,
            },
            p1.into_iter().chain(p2).collect(),
        )
    }

    /// Restore the default layout of one mode.
    pub fn reset(&mut self, mode: AppMode) {
        match mode {
            AppMode::Map => self.map = default_layout(),
            AppMode::Sprites => self.sprites = default_sprites_layout(),
        }
    }

    pub fn has_sprite_preview(&self) -> bool {
        self.map.find_tab(&Panel::SpritePreview).is_some()
    }

    /// Show or hide the Sprite preview panel (View menu).
    pub fn toggle_sprite_preview(&mut self) {
        match self.map.find_tab(&Panel::SpritePreview) {
            Some(location) => {
                let _ = self.map.remove_tab(location);
            }
            None => self.map.push_to_focused_leaf(Panel::SpritePreview),
        }
    }

    /// Write the layouts that changed. Returns an error text on failure.
    pub fn save_if_changed(&mut self, map_file: &Path, sprites_file: &Path) -> Result<(), String> {
        // A layout that has not been drawn yet holds NaN rectangles, which JSON
        // writes as `null` and cannot read back: keep the old file for now.
        let map = serde_json::to_string(&self.map).map_err(|e| e.to_string())?;
        if map != self.last_map && !map.contains("null") {
            crate::config::write_atomic(map_file, map.as_bytes()).map_err(|e| format!("{e:#}"))?;
            self.last_map = map;
        }
        let sprites = serde_json::to_string(&self.sprites).map_err(|e| e.to_string())?;
        if sprites != self.last_sprites && !sprites.contains("null") {
            crate::config::write_atomic(sprites_file, sprites.as_bytes())
                .map_err(|e| format!("{e:#}"))?;
            self.last_sprites = sprites;
        }
        Ok(())
    }
}

struct Viewer<'a> {
    state: &'a mut AppState,
}

impl TabViewer for Viewer<'_> {
    type Tab = Panel;

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    fn id(&mut self, tab: &mut Panel) -> Id {
        Id::new(("panel", *tab))
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Panel) {
        match tab {
            Panel::Map => map_view::show(ui, self.state),
            Panel::Maps => maps::show(ui, self.state),
            Panel::Tiles => tiles::show(ui, self.state),
            Panel::Layers => layers::show(ui, self.state),
            Panel::Properties => properties::show(ui, self.state),
            Panel::Log => log::show(ui, self.state),
            Panel::SpritePreview => sprites::preview::show(ui, self.state),
        }
    }

    fn is_closeable(&self, tab: &Panel) -> bool {
        *tab == Panel::SpritePreview
    }

    /// The Map panel is transparent so Bevy's scene shows through.
    fn clear_background(&self, tab: &Panel) -> bool {
        *tab != Panel::Map
    }

    fn scroll_bars(&self, tab: &Panel) -> [bool; 2] {
        if *tab == Panel::Map {
            [false, false]
        } else {
            [false, true]
        }
    }
}

struct SpritesViewer<'a> {
    state: &'a mut AppState,
}

impl TabViewer for SpritesViewer<'_> {
    type Tab = SpritesPanel;

    fn title(&mut self, tab: &mut SpritesPanel) -> egui::WidgetText {
        tab.title().into()
    }

    fn id(&mut self, tab: &mut SpritesPanel) -> Id {
        Id::new(("sprites_panel", *tab))
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut SpritesPanel) {
        match tab {
            SpritesPanel::Log => log::show(ui, self.state),
            panel => sprites::show(ui, *panel, self.state),
        }
    }

    fn is_closeable(&self, _tab: &SpritesPanel) -> bool {
        false
    }
}

/// A full-window `Ui` on the background layer, for top-level panels.
pub fn root_ui(ctx: &egui::Context) -> Ui {
    Ui::new(
        ctx.clone(),
        Id::new("capy_root"),
        UiBuilder::new()
            .layer_id(LayerId::background())
            .max_rect(ctx.viewport_rect()),
    )
}

fn dock_style(ui: &Ui) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(ui.style());
    style.main_surface_border_stroke = egui::Stroke::NONE;
    style
}

/// The strip above everything: Map Editor | Sprites.
fn mode_tabs(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        for (mode, label) in [(AppMode::Map, "Map Editor"), (AppMode::Sprites, "Sprites")] {
            let selected = state.config.mode == mode;
            if ui
                .selectable_label(selected, egui::RichText::new(label).strong())
                .clicked()
                && !selected
            {
                state.config.mode = mode;
                state.config_dirty = true;
            }
        }
    });
}

/// Draw the whole UI for this frame.
pub fn draw(ctx: &egui::Context, state: &mut AppState, dock: &mut Dock) {
    state.pixels_per_point = ctx.pixels_per_point();
    state.sync_docs();
    state.browser_tick(ctx);
    if !state.edit.modal_open()
        && !state.sprites.modal_open()
        && state.browser.prompt.is_none()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K))
    {
        state.toggle_palette();
    }
    match state.config.mode {
        AppMode::Map => shortcuts::handle(ctx, state),
        AppMode::Sprites => sprites::shortcuts(ctx, state),
    }
    focus_sprites_panel(state, dock);
    if std::mem::take(&mut state.browser.focus_tiles)
        && let Some(location) = dock.map.find_tab(&Panel::Tiles)
    {
        let _ = dock.map.set_active_tab(location);
    }
    let mut root = root_ui(ctx);

    egui::Panel::top("mode_tabs").show(&mut root, |ui| {
        mode_tabs(ui, state);
    });
    egui::Panel::top("menu_bar").show(&mut root, |ui| {
        menu::show(ui, state, dock);
    });
    egui::Panel::bottom("status_bar").show(&mut root, |ui| {
        status::show(ui, state);
    });

    // The map rectangle is re-set by the Map panel each frame; clear it first
    // so a hidden Map tab (or the Sprites mode) leaves the 2D viewport inactive.
    state.map_rect = None;
    let mode = state.config.mode;
    egui::CentralPanel::no_frame().show(&mut root, |ui| {
        let style = dock_style(ui);
        match mode {
            AppMode::Map => {
                DockArea::new(&mut dock.map)
                    .style(style)
                    .show_close_buttons(true)
                    .show_add_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut Viewer { state });
            }
            AppMode::Sprites => {
                DockArea::new(&mut dock.sprites)
                    .style(style)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut SpritesViewer { state });
            }
        }
    });

    dialogs::show_all(ctx, state);
    tiles::windows(ctx, state);
    palette::show(ctx, state);
    sprites::dialogs(ctx, state);
    toasts::show(ctx, state);
}

/// Brings the panel the sprites state asked for to the front.
fn focus_sprites_panel(state: &mut AppState, dock: &mut Dock) {
    if std::mem::take(&mut state.sprites.show_preview_panel) && !dock.has_sprite_preview() {
        dock.toggle_sprite_preview();
    }
    let Some(focus) = state.sprites.focus.take() else {
        return;
    };
    let panel = match focus {
        crate::sprites::Focus::Viewer => SpritesPanel::Viewer,
        crate::sprites::Focus::Editor => SpritesPanel::Editor,
        crate::sprites::Focus::Converter => SpritesPanel::Converter,
    };
    if let Some(location) = dock.sprites.find_tab(&panel) {
        let _ = dock.sprites.set_active_tab(location);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layouts_are_complete() {
        assert!(layout_is_complete(&default_layout()));
        assert!(sprites_layout_is_complete(&default_sprites_layout()));
    }

    #[test]
    fn sprite_preview_is_optional_and_toggles() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (mut dock, problems) =
            Dock::load(&tmp.path().join("a.json"), &tmp.path().join("b.json"));
        assert!(problems.is_empty());
        assert!(!dock.has_sprite_preview());
        assert!(layout_is_complete(&dock.map));
        dock.toggle_sprite_preview();
        assert!(dock.has_sprite_preview());
        assert!(layout_is_complete(&dock.map), "optional panel is allowed");
        dock.toggle_sprite_preview();
        assert!(!dock.has_sprite_preview());
    }

    /// JSON of a layout as the dock would have it once it has been drawn
    /// (rectangles are NaN, i.e. `null`, until then).
    fn drawn_json<T: serde::Serialize>(state: &DockState<T>) -> String {
        use serde_json::Value;
        fn fix(v: &mut Value) {
            match v {
                Value::Array(items) => {
                    let numeric = items.iter().all(|i| i.is_null() || i.is_number());
                    for item in items.iter_mut() {
                        if numeric && item.is_null() {
                            *item = Value::from(0.0);
                        } else {
                            fix(item);
                        }
                    }
                }
                Value::Object(map) => {
                    for (key, item) in map.iter_mut() {
                        if (key == "x" || key == "y") && item.is_null() {
                            *item = Value::from(0.0);
                        } else {
                            fix(item);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut value = serde_json::to_value(state).expect("json");
        fix(&mut value);
        value.to_string()
    }

    #[test]
    fn each_mode_persists_its_own_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (map_file, sprites_file) =
            (tmp.path().join("map.json"), tmp.path().join("sprites.json"));
        let (mut dock, _) = Dock::load(&map_file, &sprites_file);
        // Layouts that were never drawn hold NaN rectangles; they are not written.
        dock.toggle_sprite_preview();
        dock.save_if_changed(&map_file, &sprites_file)
            .expect("save");
        assert!(!map_file.exists() && !sprites_file.exists());
        // A drawn map layout reloads with its optional panel.
        std::fs::write(&map_file, drawn_json(&dock.map)).expect("write");
        let (back, problems) = Dock::load(&map_file, &sprites_file);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(back.has_sprite_preview(), "map layout restored");
        assert!(sprites_layout_is_complete(&back.sprites));
        // A sprites layout with a duplicated panel falls back to the default,
        // without touching the map layout.
        dock.sprites.push_to_focused_leaf(SpritesPanel::Viewer);
        std::fs::write(&sprites_file, drawn_json(&dock.sprites)).expect("write");
        let (back, problems) = Dock::load(&map_file, &sprites_file);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(back.has_sprite_preview());
        assert!(sprites_layout_is_complete(&back.sprites));
    }

    #[test]
    fn broken_files_fall_back_with_a_message() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (a, b) = (tmp.path().join("a.json"), tmp.path().join("b.json"));
        std::fs::write(&a, "{").expect("write");
        let (dock, problems) = Dock::load(&a, &b);
        assert_eq!(problems.len(), 1);
        assert!(layout_is_complete(&dock.map));
        // A layout missing a required panel is rejected too.
        let partial = DockState::new(vec![Panel::Map]);
        std::fs::write(&a, serde_json::to_string(&partial).expect("json")).expect("write");
        let (dock, problems) = Dock::load(&a, &b);
        assert_eq!(problems.len(), 1);
        assert!(layout_is_complete(&dock.map));
    }
}
