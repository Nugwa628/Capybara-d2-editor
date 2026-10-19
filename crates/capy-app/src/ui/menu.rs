//! Top bar: File / View menus and the profile picker.

use bevy_egui::egui::{self, Ui};
use d2_assets::GamePath;

use super::Dock;
use crate::config::AppMode;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState, dock: &mut Dock) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, state));
        ui.menu_button("View", |ui| view_menu(ui, state, dock));
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
        .add_enabled(can_close, egui::Button::new("Close map"))
        .clicked()
    {
        if let Some(i) = state.active {
            state.close_doc(i);
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
    ui.add_enabled(false, egui::Button::new("Mark as packed"))
        .on_disabled_hover_text("coming with capy-history");

    ui.separator();
    if ui.button("Quit").clicked() {
        state.quit_requested = true;
        ui.close();
    }
}

fn view_menu(ui: &mut Ui, state: &mut AppState, dock: &mut Dock) {
    let map_mode = state.config.mode == AppMode::Map;
    if ui
        .add_enabled(
            map_mode && state.active.is_some(),
            egui::Button::new("Fit map to window"),
        )
        .clicked()
    {
        state.request_fit = true;
        ui.close();
    }
    let mut preview = dock.has_sprite_preview();
    if ui
        .add_enabled(
            map_mode,
            egui::Checkbox::new(&mut preview, "Sprite preview"),
        )
        .changed()
    {
        dock.toggle_sprite_preview();
        ui.close();
    }
    if ui.button("Reset panel layout").clicked() {
        dock.reset(state.config.mode);
        ui.close();
    }
}

fn profile_picker(ui: &mut Ui, state: &mut AppState) {
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
fn open_saves_folder(state: &mut AppState) {
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
