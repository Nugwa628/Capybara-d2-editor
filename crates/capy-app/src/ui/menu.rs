//! The Sprites mode's menu bar: File / View menus and the profile picker.
//! (The Map Editor uses the ribbon, see [`super::ribbon`].)

use bevy_egui::egui::{self, Ui};
use d2_assets::GamePath;

use super::Dock;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState, dock: &mut Dock) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, state));
        ui.menu_button("View", |ui| {
            if ui.button("Reset panel layout").clicked() {
                dock.reset();
                ui.close();
            }
        });
        ui.separator();
        profile_picker(ui, state);
    });
}

fn file_menu(ui: &mut Ui, state: &mut AppState) {
    let has_profile = state.active_profile_id().is_some();

    ui.menu_button("Open recent", |ui| {
        let profile = state.active_profile_id().map(str::to_owned);
        let recent: Vec<String> = state
            .config
            .recent_maps
            .iter()
            .filter(|r| Some(&r.profile) == profile.as_ref())
            .map(|r| r.path.clone())
            .collect();
        if recent.is_empty() {
            ui.label("Nothing yet");
        }
        for path in recent {
            if ui.button(&path).clicked() {
                state.open_map(&GamePath::new(&path));
                ui.close();
            }
        }
    });

    ui.separator();
    let can_close = state.active.is_some();
    if ui
        .add_enabled(can_close, egui::Button::new("Save").shortcut_text("Ctrl+S"))
        .on_hover_text(r"Write the map to capybara\saves\<profile>\ and record a snapshot")
        .clicked()
    {
        state.save_active();
        ui.close();
    }
    if ui
        .add_enabled(can_close, egui::Button::new("Close map"))
        .clicked()
    {
        if let Some(i) = state.active {
            state.request_close(i);
        }
        ui.close();
    }
    ui.separator();

    if ui
        .add_enabled(has_profile, egui::Button::new("Open saves folder"))
        .on_hover_text("capybara\\saves\\<profile>\\ in Explorer")
        .clicked()
    {
        open_saves_folder(state);
        ui.close();
    }
    let changed = state.edit.changed;
    if ui
        .add_enabled(
            has_profile && changed > 0,
            egui::Button::new(format!("Mark as packed ({changed})")),
        )
        .on_hover_text("Empty CHANGED.txt after packing the saved files into your MPQ")
        .on_disabled_hover_text("CHANGED.txt is empty")
        .clicked()
    {
        state.edit.confirm_packed = true;
        ui.close();
    }
    ui.weak(format!("CHANGED.txt: {changed} file(s)"));

    if ui
        .add_enabled(
            has_profile,
            egui::Button::new("Import DS1Edit ini mappings..."),
        )
        .on_hover_text(
            "DS1Edit's assets\\*.ini (<LvlTypesId> <LvlPrestDef> <path> per line): each map \
             listed uses that LvlType and Def from now on (saved per profile)",
        )
        .clicked()
    {
        state.pick_ini_files();
        ui.close();
    }

    ui.separator();
    if ui.button("Settings...").clicked() {
        state.settings.open = true;
        ui.close();
    }
    if ui
        .button("Add game install...")
        .on_hover_text("Make a profile for a Diablo II or Median XL folder")
        .clicked()
    {
        state.open_add_install(None);
        ui.close();
    }
    ui.separator();
    if ui.button("Quit").clicked() {
        state.request_quit();
        ui.close();
    }
}

pub(super) fn profile_picker(ui: &mut Ui, state: &mut AppState) {
    ui.label("Profile");
    let current = state.active_profile_id().map(str::to_owned);
    let selected_text = match &current {
        Some(id) => state
            .profiles
            .iter()
            .find(|p| &p.id == id)
            .map_or_else(|| id.clone(), |p| p.name.clone()),
        None => "(none)".to_owned(),
    };
    let mut chosen: Option<String> = None;
    egui::ComboBox::from_id_salt("profile_picker")
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for p in &state.profiles {
                let selected = current.as_deref() == Some(p.id.as_str());
                if ui.selectable_label(selected, &p.name).clicked() && !selected {
                    chosen = Some(p.id.clone());
                }
            }
            if state.profiles.is_empty() {
                ui.label("No profiles found");
            }
        });
    if let Some(id) = chosen {
        state.open_profile_by_id(&id);
    }
    if state.profiles.is_empty() {
        ui.weak(format!("add *.toml to {}", state.paths.profiles.display()));
    }
}

/// Open `capybara\saves\<profile>\` in Explorer, creating it first.
pub(super) fn open_saves_folder(state: &mut AppState) {
    let Some(id) = state.active_profile_id().map(str::to_owned) else {
        return;
    };
    let dir = state.paths.saves_for(&id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        state.error(format!("Cannot create {}: {e}", dir.display()));
        return;
    }
    // Explorer exits non-zero even on success, so only the launch is checked.
    if let Err(e) = std::process::Command::new("explorer").arg(&dir).spawn() {
        state.error(format!("Cannot open {} in Explorer: {e}", dir.display()));
    }
}
