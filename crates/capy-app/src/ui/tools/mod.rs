//! The tool windows of P11-P13: tile libraries, presets, game data (tables,
//! level tools, automap, cube recipes, entering text, custom objects),
//! diagnostics and map packages. Each is a floating window opened by a
//! registry command (`tiles.*`, `presets.*`, `game.*`, `diag.*`, `file.*`);
//! the ribbon, the command finder and `CAPY_SCRIPT` reach them the same way.
//!
//! Every write goes to `capybara\saves\<profile>\<game path>` with a
//! CHANGED.txt entry and a history snapshot (see [`io`]); nothing here
//! touches the game folder.

use std::collections::BTreeSet;

use bevy_egui::egui::{self, Id};

use crate::state::AppState;

pub mod addmap;
pub mod automap_ed;
pub mod column_help;
pub mod diag;
pub mod dt1;
pub mod game;
pub mod io;
pub mod package;
pub mod presets;
pub mod review;
pub mod script;
pub mod tables;

/// What a window needs before its command is enabled.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    /// A game profile is open.
    Profile,
    /// A map is open.
    Map,
}

pub struct Spec {
    pub id: &'static str,
    pub title: &'static str,
    pub size: [f32; 2],
    pub needs: Needs,
}

/// The windows, one per command.
pub const WINDOWS: &[Spec] = &[
    Spec {
        id: "tiles.dt1Manager",
        title: "DT1 libraries",
        size: [760.0, 620.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "tiles.settings",
        title: "Tile settings",
        size: [520.0, 560.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "tiles.collision",
        title: "Collision flags",
        size: [700.0, 520.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "tiles.paint",
        title: "Paint tile pixels",
        size: [520.0, 700.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "tiles.actSafe",
        title: "Make act-safe",
        size: [640.0, 560.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "tiles.review",
        title: "Review tile libraries",
        size: [620.0, 560.0],
        needs: Needs::Map,
    },
    Spec {
        id: "diag.unusedDt1",
        title: "Unused DT1s",
        size: [620.0, 520.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "diag.restore",
        title: "Restore assets",
        size: [560.0, 420.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "diag.compat",
        title: "Compatibility check",
        size: [700.0, 560.0],
        needs: Needs::Map,
    },
    Spec {
        id: "diag.crashLog",
        title: "Crash log",
        size: [660.0, 520.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "presets.save",
        title: "Save selection as preset",
        size: [380.0, 220.0],
        needs: Needs::Map,
    },
    Spec {
        id: "presets.builder",
        title: "Preset builder",
        size: [560.0, 620.0],
        needs: Needs::Map,
    },
    Spec {
        id: "presets.suggest",
        title: "Suggested presets",
        size: [520.0, 520.0],
        needs: Needs::Map,
    },
    Spec {
        id: "game.addToGame",
        title: "Add to game",
        size: [560.0, 560.0],
        needs: Needs::Map,
    },
    Spec {
        id: "game.changeType",
        title: "Change level type",
        size: [560.0, 520.0],
        needs: Needs::Map,
    },
    Spec {
        id: "game.typePackages",
        title: "Type packages",
        size: [560.0, 480.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "game.entryText",
        title: "Entering text",
        size: [560.0, 460.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "game.cube",
        title: "Cube recipe",
        size: [580.0, 560.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "game.customObject",
        title: "Custom object",
        size: [560.0, 560.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "game.automap",
        title: "Automap editor",
        size: [760.0, 600.0],
        needs: Needs::Map,
    },
    Spec {
        id: "game.tables",
        title: "Tables",
        size: [980.0, 640.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "file.exportPackage",
        title: "Map package",
        size: [600.0, 520.0],
        needs: Needs::Map,
    },
    Spec {
        id: "file.importPackage",
        title: "Import a map package",
        size: [600.0, 460.0],
        needs: Needs::Profile,
    },
    Spec {
        id: "file.importDs1Dt1",
        title: "Import a DS1 with its DT1s",
        size: [600.0, 420.0],
        needs: Needs::Profile,
    },
];

pub fn spec(id: &str) -> Option<&'static Spec> {
    WINDOWS.iter().find(|s| s.id == id)
}

/// Everything the tool windows keep between frames.
#[derive(Default)]
pub struct ToolsState {
    pub open: BTreeSet<&'static str>,
    pub addmap: addmap::AddMapUi,
    pub dt1: dt1::Dt1Ui,
    pub review: review::ReviewUi,
    pub diag: diag::DiagUi,
    pub presets: presets::PresetsUi,
    pub game: game::GameUi,
    pub automap: automap_ed::AutomapUi,
    pub tables: tables::TablesUi,
    pub package: package::PackageUi,
    /// The right column shows the Presets tab instead of Tiles.
    pub presets_tab: bool,
}

impl ToolsState {
    pub fn close_all(&mut self) {
        self.open.clear();
    }
}

/// `Some(enabled)` when `id` is one of the tool commands.
pub fn enabled(state: &AppState, id: &str) -> Option<bool> {
    let s = spec(id)?;
    Some(match s.needs {
        Needs::Profile => state.session.is_some(),
        Needs::Map => state.session.is_some() && state.active_doc().is_some(),
    })
}

pub fn active(state: &AppState, id: &str) -> Option<bool> {
    spec(id).map(|s| state.tools.open.contains(s.id))
}

/// Toggles the window of `id`. False when `id` is not a tool command.
pub fn run(state: &mut AppState, id: &str) -> bool {
    let Some(s) = spec(id) else { return false };
    if !state.tools.open.remove(s.id) {
        state.tools.open.insert(s.id);
        opened(state, s.id);
    }
    true
}

/// Prepares a window when it opens.
fn opened(state: &mut AppState, id: &str) {
    match id {
        "tiles.dt1Manager" | "tiles.settings" | "tiles.collision" | "tiles.paint"
        | "tiles.actSafe" => dt1::on_open(state),
        "game.addToGame" | "game.changeType" | "game.entryText" => game::on_open(state, id),
        "game.automap" => automap_ed::on_open(state),
        "file.exportPackage" => package::on_open(state),
        _ => {}
    }
}

/// Draws every open tool window.
pub fn windows(ctx: &egui::Context, state: &mut AppState) {
    let open: Vec<&'static str> = state.tools.open.iter().copied().collect();
    for id in open {
        let Some(s) = spec(id) else { continue };
        let mut is_open = true;
        egui::Window::new(s.title)
            .id(Id::new(("tool_window", s.id)))
            .open(&mut is_open)
            .default_size(s.size)
            .default_pos([320.0, 140.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("tool_scroll", s.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| show(ui, state, s.id));
            });
        if !is_open {
            state.tools.open.remove(s.id);
        }
    }
}

fn show(ui: &mut egui::Ui, state: &mut AppState, id: &str) {
    match id {
        "tiles.dt1Manager" => dt1::manager(ui, state),
        "tiles.settings" => dt1::settings(ui, state),
        "tiles.collision" => dt1::collision(ui, state),
        "tiles.paint" => dt1::paint(ui, state),
        "tiles.actSafe" => dt1::act_safe(ui, state),
        "tiles.review" => review::show(ui, state),
        "diag.unusedDt1" => diag::unused(ui, state),
        "diag.restore" => diag::restore(ui, state),
        "diag.compat" => diag::compat(ui, state),
        "diag.crashLog" => diag::crash_log(ui, state),
        "presets.save" => presets::save_dialog(ui, state),
        "presets.builder" => presets::builder(ui, state),
        "presets.suggest" => presets::suggest(ui, state),
        "game.addToGame" => game::add_to_game(ui, state),
        "game.changeType" => game::change_type(ui, state),
        "game.typePackages" => game::type_packages(ui, state),
        "game.entryText" => game::entry_text(ui, state),
        "game.cube" => game::cube(ui, state),
        "game.customObject" => game::custom_object(ui, state),
        "game.automap" => automap_ed::show(ui, state),
        "game.tables" => tables::show(ui, state),
        "file.exportPackage" => package::export(ui, state),
        "file.importPackage" => package::import(ui, state),
        "file.importDs1Dt1" => package::import_loose(ui, state),
        _ => {}
    }
}

/// A one-line status under a form: red for errors.
pub fn message(ui: &mut egui::Ui, text: &str) {
    if text.is_empty() {
        return;
    }
    let bad = text.starts_with("Cannot") || text.starts_with("Error") || text.contains(" failed");
    let color = if bad {
        egui::Color32::from_rgb(230, 110, 100)
    } else {
        ui.visuals().weak_text_color()
    };
    ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap());
}
