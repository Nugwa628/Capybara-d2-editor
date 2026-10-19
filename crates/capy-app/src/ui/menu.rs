//! Top bar: File / View menus and the profile picker.

use bevy_egui::egui::{self, Ui};
use d2_assets::GamePath;

use super::{Dock, dialogs};
use crate::config::AppMode;
use crate::edit::ops::EditOp;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState, dock: &mut Dock) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, state));
        ui.menu_button("Edit", |ui| edit_menu(ui, state));
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

fn edit_menu(ui: &mut Ui, state: &mut AppState) {
    let (undo, redo, has_sel, has_obj) = match state.active_doc() {
        Some(d) => (
            d.edit.undo_label().map(str::to_owned),
            d.edit.redo_label().map(str::to_owned),
            !d.selection.is_empty(),
            !d.object_sel.is_empty(),
        ),
        None => (None, None, false, false),
    };
    let has_doc = state.active.is_some();
    let label = |verb: &str, what: &Option<String>| match what {
        Some(w) => format!("{verb} {w}"),
        None => verb.to_owned(),
    };
    if ui
        .add_enabled(
            undo.is_some(),
            egui::Button::new(label("Undo", &undo)).shortcut_text("Ctrl+Z"),
        )
        .clicked()
    {
        if !super::shortcuts::refuse_in_snapshot(state) {
            state.undo();
        }
        ui.close();
    }
    if ui
        .add_enabled(
            redo.is_some(),
            egui::Button::new(label("Redo", &redo)).shortcut_text("Ctrl+Y"),
        )
        .clicked()
    {
        if !super::shortcuts::refuse_in_snapshot(state) {
            state.redo();
        }
        ui.close();
    }
    ui.separator();
    if ui
        .add_enabled(has_sel, egui::Button::new("Cut").shortcut_text("Ctrl+X"))
        .clicked()
    {
        if !super::shortcuts::refuse_in_snapshot(state) {
            state.cut_selection();
        }
        ui.close();
    }
    if ui
        .add_enabled(has_sel, egui::Button::new("Copy").shortcut_text("Ctrl+C"))
        .clicked()
    {
        state.copy_selection();
        ui.close();
    }
    if ui
        .add_enabled(has_doc, egui::Button::new("Paste").shortcut_text("Ctrl+V"))
        .clicked()
    {
        if !super::shortcuts::refuse_in_snapshot(state) {
            state.begin_paste();
        }
        ui.close();
    }
    if ui
        .add_enabled(
            has_sel || has_obj,
            egui::Button::new("Delete").shortcut_text("Del"),
        )
        .clicked()
    {
        if !super::shortcuts::refuse_in_snapshot(state) {
            super::shortcuts::delete(state);
        }
        ui.close();
    }
    ui.separator();
    if ui
        .add_enabled(
            has_doc,
            egui::Button::new("Select all on the active layer").shortcut_text("Ctrl+A"),
        )
        .clicked()
    {
        super::shortcuts::select_all(state);
        ui.close();
    }
    if ui
        .add_enabled(
            has_sel || has_obj,
            egui::Button::new("Deselect").shortcut_text("Ctrl+D"),
        )
        .clicked()
    {
        state.deselect();
        ui.close();
    }
    ui.separator();
    ui.add_enabled_ui(has_sel, |ui| {
        ui.menu_button("Selection flags", |ui| {
            let mut op = None;
            if ui.button("Set hidden").clicked() {
                op = Some((Some(true), None));
            }
            if ui.button("Clear hidden").clicked() {
                op = Some((Some(false), None));
            }
            if ui.button("Set unwalkable").clicked() {
                op = Some((None, Some(true)));
            }
            if ui.button("Clear unwalkable").clicked() {
                op = Some((None, Some(false)));
            }
            if let (Some((hidden, unwalkable)), Some(i)) = (op, state.active) {
                state.exec(i, EditOp::SetFlags { hidden, unwalkable });
                ui.close();
            }
        });
    });
    if ui
        .add_enabled(has_sel, egui::Button::new("Move selection to layer..."))
        .clicked()
    {
        state.edit.move_dialog = dialogs::new_move_dialog(state);
        ui.close();
    }
    if ui
        .add_enabled(has_doc, egui::Button::new("Resize map..."))
        .clicked()
    {
        state.edit.resize_dialog = dialogs::new_resize_dialog(state);
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
    ui.add_enabled(
        map_mode,
        egui::Checkbox::new(&mut state.edit.grid, "Cell grid"),
    )
    .on_hover_text("G");
    if ui
        .add_enabled(
            map_mode,
            egui::Checkbox::new(&mut state.config.animate_objects, "Animate objects"),
        )
        .on_hover_text("Play object and NPC animations at their game speed (25 Hz clock)")
        .changed()
    {
        state.config_dirty = true;
    }
    if ui
        .add_enabled(
            map_mode,
            egui::Checkbox::new(&mut state.config.object_markers, "Object markers"),
        )
        .on_hover_text("Diamond handles on every object, not only with the Objects tool")
        .changed()
    {
        state.config_dirty = true;
    }
    let mut objects = dock.has_objects();
    if ui
        .add_enabled(map_mode, egui::Checkbox::new(&mut objects, "Objects panel"))
        .changed()
    {
        if objects {
            dock.show_objects();
        } else if let Some(location) = dock.map.find_tab(&super::Panel::Objects) {
            let _ = dock.map.remove_tab(location);
        }
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
    let mut history = dock.has_history();
    if ui
        .add_enabled(map_mode, egui::Checkbox::new(&mut history, "History"))
        .changed()
    {
        dock.toggle_history();
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
