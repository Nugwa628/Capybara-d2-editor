//! The egui shell.
//!
//! Two modes share the window. The **Map Editor** has a fixed, ds1-studio
//! style layout (ribbon, map list, map, tiles column, status bar; see
//! [`shell`]) driven by the command registry ([`commands`]). **Sprites**
//! keeps its dock layout (sprite list, viewer, editor, converter and log;
//! see [`sprites`]), persisted in `layout-sprites.json`.

mod commands;
mod dialogs;
mod finder;
mod history;
mod layers;
mod log;
mod map_tools;
mod map_view;
mod maps;
mod menu;
mod minimap;
pub mod objects;
mod palette;
mod properties;
mod ribbon;
mod settings;
pub mod shell;
mod shortcuts_dialog;
pub mod sprites;
mod status;
pub mod theme;
mod thumbs;
mod tiles;
mod toasts;
pub mod tools;

use std::path::Path;

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Id, LayerId, Ui, UiBuilder};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};

use crate::config::AppMode;
use crate::state::AppState;

pub use commands::{COMMANDS, Combo, enabled as command_enabled, run as commands_run};
pub use map_tools::cancel_gesture as cancel_map_gesture;
pub use thumbs::tile_key as tile_key_for;

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

/// The dock layout of the Sprites mode.
#[derive(Resource)]
pub struct Dock {
    pub sprites: DockState<SpritesPanel>,
    /// JSON of the state last written, to detect changes.
    last_sprites: String,
}

pub fn default_sprites_layout() -> DockState<SpritesPanel> {
    let mut state = DockState::new(vec![SpritesPanel::Viewer, SpritesPanel::Editor]);
    let main = state.main_surface_mut();
    let [centre, _list] = main.split_left(NodeIndex::root(), 0.18, vec![SpritesPanel::Browser]);
    let [centre, _converter] = main.split_right(centre, 0.74, vec![SpritesPanel::Converter]);
    let [_centre, _log] = main.split_below(centre, 0.8, vec![SpritesPanel::Log]);
    state
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

impl Dock {
    /// Loads the Sprites layout. The returned strings are problems worth a
    /// log line; a missing file is the default layout without a problem.
    pub fn load(sprites_file: &Path) -> (Dock, Vec<String>) {
        let (state, problem) = match std::fs::read_to_string(sprites_file) {
            Ok(text) => match serde_json::from_str::<DockState<SpritesPanel>>(&text) {
                Ok(s) if sprites_layout_is_complete(&s) => (Some(s), None),
                Ok(_) => (
                    None,
                    Some("saved sprites layout is incomplete; using the default".to_owned()),
                ),
                Err(e) => (
                    None,
                    Some(format!(
                        "saved sprites layout is unreadable ({e}); using the default"
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
            Err(e) => (None, Some(format!("cannot read the sprites layout: {e}"))),
        };
        let sprites = state.unwrap_or_else(default_sprites_layout);
        let last_sprites = serde_json::to_string(&sprites).unwrap_or_default();
        (
            Dock {
                sprites,
                last_sprites,
            },
            problem.into_iter().collect(),
        )
    }

    /// Restore the default Sprites layout.
    pub fn reset(&mut self) {
        self.sprites = default_sprites_layout();
    }

    /// Write the layout if it changed. Returns an error text on failure.
    pub fn save_if_changed(&mut self, sprites_file: &Path) -> Result<(), String> {
        // A layout that has not been drawn yet holds NaN rectangles, which JSON
        // writes as `null` and cannot read back: keep the old file for now.
        let sprites = serde_json::to_string(&self.sprites).map_err(|e| e.to_string())?;
        if sprites != self.last_sprites && !sprites.contains("null") {
            crate::config::write_atomic(sprites_file, sprites.as_bytes())
                .map_err(|e| format!("{e:#}"))?;
            self.last_sprites = sprites;
        }
        Ok(())
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

/// The strip above the Sprites mode: Map Editor | Sprites.
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
    shell::prepare_frame(ctx, state);
    state.sync_docs();
    state.history_frame();
    state.browser_tick(ctx);
    let mode = state.config.mode;
    let modal = state.edit.modal_open()
        || state.sprites.modal_open()
        || state.history.modal_open()
        || state.settings.modal_open()
        || state.browser.prompt.is_some();
    // The asset palette and the browser's questions take the keyboard.
    let browser_modal = state.browser.palette.open || state.browser.prompt.is_some();
    match mode {
        _ if browser_modal => {}
        AppMode::Map => commands::handle_keys(ctx, state),
        AppMode::Sprites => {
            if !modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
                state.toggle_palette();
            }
            sprites::shortcuts(ctx, state);
        }
    }
    focus_sprites_panel(state, dock);

    // The map rectangle is re-set by the map view each frame; clear it first
    // so the Sprites mode leaves the 2D viewport inactive.
    state.map_rect = None;
    let mut root = root_ui(ctx);
    match mode {
        AppMode::Map => shell::show(&mut root, state),
        AppMode::Sprites => {
            egui::Panel::top("mode_tabs").show(&mut root, |ui| {
                mode_tabs(ui, state);
            });
            egui::Panel::top("menu_bar").show(&mut root, |ui| {
                menu::show(ui, state, dock);
            });
            egui::Panel::bottom("status_bar").show(&mut root, |ui| {
                status::show(ui, state);
            });
            egui::CentralPanel::no_frame().show(&mut root, |ui| {
                let style = dock_style(ui);
                DockArea::new(&mut dock.sprites)
                    .style(style)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut SpritesViewer { state });
            });
        }
    }

    dialogs::show_all(ctx, state);
    history::dialogs(ctx, state);
    settings::show(ctx, state);
    tiles::windows(ctx, state);
    palette::show(ctx, state);
    sprites::dialogs(ctx, state);
    if mode == AppMode::Map {
        shell::windows(ctx, state);
        tools::windows(ctx, state);
    }
    toasts::show(ctx, state);
}

/// Brings the panel the sprites state asked for to the front.
fn focus_sprites_panel(state: &mut AppState, dock: &mut Dock) {
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
    fn default_layout_is_complete() {
        assert!(sprites_layout_is_complete(&default_sprites_layout()));
    }

    #[test]
    fn the_sprites_layout_persists_and_bad_files_fall_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("sprites.json");
        let (mut dock, problems) = Dock::load(&file);
        assert!(problems.is_empty());
        // Layouts that were never drawn hold NaN rectangles; they are not written.
        dock.sprites.push_to_focused_leaf(SpritesPanel::Viewer);
        dock.save_if_changed(&file).expect("save");
        assert!(!file.exists());
        // A layout with a duplicated panel falls back to the default.
        std::fs::write(&file, drawn_json(&dock.sprites)).expect("write");
        let (back, problems) = Dock::load(&file);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(sprites_layout_is_complete(&back.sprites));
        // A good drawn layout reloads as it was.
        std::fs::write(&file, drawn_json(&default_sprites_layout())).expect("write");
        let (_, problems) = Dock::load(&file);
        assert!(problems.is_empty(), "{problems:?}");
        std::fs::write(&file, "{").expect("write");
        let (back, problems) = Dock::load(&file);
        assert_eq!(problems.len(), 1);
        assert!(sprites_layout_is_complete(&back.sprites));
    }
}
