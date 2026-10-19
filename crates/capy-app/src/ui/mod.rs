//! The egui shell: menu, status bar, toasts and the dock.

mod layers;
mod log;
mod map_view;
mod maps;
mod menu;
mod properties;
mod status;
mod tiles;
mod toasts;

use std::path::Path;

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Id, LayerId, Ui, UiBuilder};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    Map,
    Maps,
    Tiles,
    Layers,
    Properties,
    Log,
}

impl Panel {
    pub const ALL: [Panel; 6] = [
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
        }
    }
}

#[derive(Resource)]
pub struct Dock {
    pub state: DockState<Panel>,
    /// JSON of the state last written, to detect changes.
    last_saved: String,
}

pub fn default_layout() -> DockState<Panel> {
    let mut state = DockState::new(vec![Panel::Map]);
    let main = state.main_surface_mut();
    let [centre, _left] = main.split_left(NodeIndex::root(), 0.2, vec![Panel::Maps, Panel::Tiles]);
    let [centre, _right] = main.split_right(centre, 0.78, vec![Panel::Layers, Panel::Properties]);
    let [_centre, _bottom] = main.split_below(centre, 0.76, vec![Panel::Log]);
    state
}

/// A layout is usable only if it holds each panel exactly once.
pub fn layout_is_complete(state: &DockState<Panel>) -> bool {
    let mut seen = Vec::new();
    for (_, tab) in state.iter_all_tabs() {
        if seen.contains(tab) {
            return false;
        }
        seen.push(*tab);
    }
    Panel::ALL.iter().all(|p| seen.contains(p))
}

impl Dock {
    pub fn load(file: &Path) -> (Dock, Option<String>) {
        let mut problem = None;
        let state = match std::fs::read_to_string(file) {
            Ok(text) => match serde_json::from_str::<DockState<Panel>>(&text) {
                Ok(s) if layout_is_complete(&s) => Some(s),
                Ok(_) => {
                    problem = Some("saved layout is incomplete; using the default".to_owned());
                    None
                }
                Err(e) => {
                    problem = Some(format!(
                        "saved layout is unreadable ({e}); using the default"
                    ));
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                problem = Some(format!("cannot read layout: {e}"));
                None
            }
        };
        let state = state.unwrap_or_else(default_layout);
        let last_saved = serde_json::to_string(&state).unwrap_or_default();
        (Dock { state, last_saved }, problem)
    }

    pub fn reset(&mut self) {
        self.state = default_layout();
    }

    /// Write the layout if it changed. Returns an error text on failure.
    pub fn save_if_changed(&mut self, file: &Path) -> Result<(), String> {
        let json = serde_json::to_string(&self.state).map_err(|e| e.to_string())?;
        if json == self.last_saved {
            return Ok(());
        }
        crate::config::write_atomic(file, json.as_bytes()).map_err(|e| format!("{e:#}"))?;
        self.last_saved = json;
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
        }
    }

    fn is_closeable(&self, _tab: &Panel) -> bool {
        false
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

/// Draw the whole UI for this frame.
pub fn draw(ctx: &egui::Context, state: &mut AppState, dock: &mut Dock) {
    state.pixels_per_point = ctx.pixels_per_point();
    let mut root = root_ui(ctx);

    egui::Panel::top("menu_bar").show(&mut root, |ui| {
        menu::show(ui, state, dock);
    });
    egui::Panel::bottom("status_bar").show(&mut root, |ui| {
        status::show(ui, state);
    });

    // The map rectangle is re-set by the Map panel each frame; clear it first
    // so a hidden Map tab leaves the 3D viewport inactive.
    state.map_rect = None;
    egui::CentralPanel::no_frame().show(&mut root, |ui| {
        let mut style = egui_dock::Style::from_egui(ui.style());
        style.main_surface_border_stroke = egui::Stroke::NONE;
        DockArea::new(&mut dock.state)
            .style(style)
            .show_close_buttons(false)
            .show_add_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut Viewer { state });
    });

    toasts::show(ctx, state);
}
