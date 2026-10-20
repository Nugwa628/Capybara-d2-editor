//! File > Settings and the "Add game install..." dialog.

use bevy_egui::egui::{self, Align2, Color32, Context, Id, RichText};
use d2_assets::GameVersion;

use crate::config::{GAMMA_RANGE, MAX_AUTOSAVE_MINUTES};
use crate::edit::tools::SelectScope;
use crate::history::human_size;
use crate::installs::InstallKind;
use crate::state::AppState;

pub fn show(ctx: &Context, state: &mut AppState) {
    settings_window(ctx, state);
    add_install(ctx, state);
}

fn settings_window(ctx: &Context, state: &mut AppState) {
    if !state.settings.open {
        return;
    }
    let mut open = true;
    let max_height = (ctx.content_rect().height() - 120.0).max(240.0);
    egui::Window::new("Preferences")
        .id(Id::new("settings_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| body(ui, state));
        });
    if !open {
        state.settings.open = false;
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
}

/// Theme picker, custom themes from share codes, and the current theme's
/// share code.
fn appearance(ui: &mut egui::Ui, state: &mut AppState) {
    use super::theme;
    section(ui, "Theme");
    let mut themes = theme::builtin_themes();
    themes.extend(
        state
            .config
            .ui
            .custom_themes
            .iter()
            .filter_map(theme::CustomTheme::to_theme),
    );
    let current = super::shell::current_theme(&state.config);
    let mut remove: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        for t in &themes {
            let s = theme::Shades::of(t);
            let on = t.id == current.id;
            let resp = ui
                .add(
                    egui::Button::new(RichText::new(&t.name).color(s.text))
                        .fill(s.panel)
                        .stroke(egui::Stroke::new(
                            if on { 2.0 } else { 1.0 },
                            if on { s.accent } else { s.border2 },
                        )),
                )
                .on_hover_text(if t.custom {
                    "Custom theme (right-click to delete)"
                } else {
                    "Built-in theme"
                });
            if resp.clicked() && !on {
                state.config.ui.theme = t.id.clone();
                state.config_dirty = true;
            }
            if t.custom {
                resp.context_menu(|ui| {
                    if ui.button("Delete this theme").clicked() {
                        remove = Some(t.id.clone());
                        ui.close();
                    }
                });
            }
        }
    });
    if let Some(id) = remove {
        state.config.ui.custom_themes.retain(|c| c.id != id);
        if state.config.ui.theme == id {
            state.config.ui.theme = theme::DEFAULT_THEME.to_owned();
        }
        state.config_dirty = true;
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.shell.theme_code)
                .hint_text("Paste a theme share code (ffthemes:...)")
                .desired_width(300.0),
        );
        if ui
            .add_enabled(
                !state.shell.theme_code.trim().is_empty(),
                egui::Button::new("Add"),
            )
            .clicked()
        {
            let n = state.config.ui.custom_themes.len();
            let id = (n + 1..)
                .map(|i| format!("custom-{i}"))
                .find(|id| !state.config.ui.custom_themes.iter().any(|c| &c.id == id))
                .unwrap_or_else(|| "custom".to_owned());
            match theme::import_code(&state.shell.theme_code, id.clone()) {
                Some(custom) => {
                    state.notify(format!("Added the theme {:?}", custom.name));
                    state.config.ui.custom_themes.push(custom);
                    state.config.ui.theme = id;
                    state.config_dirty = true;
                    state.shell.theme_code.clear();
                }
                None => state.notify("That is not a theme share code"),
            }
        }
    });
    if ui
        .button("Copy this theme's share code")
        .on_hover_text("Share codes work in DS1 Studio and PD2 Filter Forge too")
        .clicked()
    {
        ui.ctx().copy_text(theme::export_code(&current));
        state.notify("Share code copied");
    }
}

fn body(ui: &mut egui::Ui, state: &mut AppState) {
    appearance(ui, state);

    section(ui, "Map view");
    let mut gamma = state.config.gamma;
    ui.horizontal(|ui| {
        ui.label("Gamma");
        if ui
            .add(egui::Slider::new(&mut gamma, GAMMA_RANGE).step_by(0.05))
            .on_hover_text(
                "Brightness of the map view and the object preview (DS1Edit's default is 1.30)",
            )
            .changed()
        {
            state.config.gamma = gamma;
            state.config_dirty = true;
        }
        if ui
            .small_button("Reset")
            .on_hover_text("Back to 1.30")
            .clicked()
        {
            state.config.gamma = crate::config::DEFAULT_GAMMA;
            state.config_dirty = true;
        }
    });

    section(ui, "Layers visible when a map opens");
    let mut v = state.config.layer_visibility;
    let before = v;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut v.floor, "Floors");
        ui.checkbox(&mut v.wall, "Walls");
        ui.checkbox(&mut v.shadow, "Shadow");
        ui.checkbox(&mut v.tag, "Tag");
        ui.checkbox(&mut v.objects, "Objects");
    });
    if v != before {
        state.set_layer_defaults(v);
    }
    if ui
        .add_enabled(
            state.active.is_some(),
            egui::Button::new("Apply to the open map"),
        )
        .clicked()
    {
        state.apply_layer_defaults_to_active();
    }

    section(ui, "Selection");
    let mut scope = state.config.select_scope;
    ui.horizontal(|ui| {
        ui.label("Selection tools start on");
        egui::ComboBox::from_id_salt("settings_scope")
            .selected_text(scope.label())
            .show_ui(ui, |ui| {
                for s in [SelectScope::Active, SelectScope::Visible] {
                    ui.selectable_value(&mut scope, s, s.label());
                }
            });
    });
    if scope != state.config.select_scope {
        state.set_scope_default(scope);
    }

    section(ui, "Start-up");
    if ui
        .checkbox(&mut state.config.show_loading_screen, "Show loading screen")
        .on_hover_text(
            "The door animation while the editor loads its game data (click or Esc skips it)",
        )
        .changed()
    {
        state.config_dirty = true;
    }

    section(ui, "Autosave");
    let mut minutes = state.config.autosave_minutes;
    ui.horizontal(|ui| {
        ui.label("Every");
        if ui
            .add(
                egui::DragValue::new(&mut minutes)
                    .range(0..=MAX_AUTOSAVE_MINUTES)
                    .suffix(" min"),
            )
            .on_hover_text("0 turns autosave off")
            .changed()
        {
            state.config.autosave_minutes = minutes;
            state.config_dirty = true;
            state.history.timer.reset();
        }
        ui.weak(if minutes == 0 { "(off)" } else { "" });
    });
    ui.weak(
        "Maps with unsaved changes are recorded in capybara\\history\\<profile>\\. \
         Autosave never writes saves or CHANGED.txt.",
    );

    section(ui, "History retention");
    ui.horizontal(|ui| {
        ui.label("Keep the newest");
        let mut keep = state.config.keep_last;
        if ui
            .add(egui::DragValue::new(&mut keep).range(1..=100_000))
            .changed()
        {
            state.config.keep_last = keep.max(1);
            state.config_dirty = true;
        }
        ui.label("saves/autosaves per map");
    });
    ui.horizontal(|ui| {
        ui.label("Drop those older than");
        let mut days = state.config.max_age_days;
        if ui
            .add(
                egui::DragValue::new(&mut days)
                    .range(0..=36_500)
                    .suffix(" days"),
            )
            .on_hover_text("0 = no age limit")
            .changed()
        {
            state.config.max_age_days = days;
            state.config_dirty = true;
        }
        if days == 0 {
            ui.weak("(no limit)");
        }
    });
    ui.weak("Checkpoints are never pruned. The policy also runs when a profile opens.");
    let has_profile = state.active_profile_id().is_some();
    if ui
        .add_enabled(has_profile, egui::Button::new("Prune now"))
        .on_disabled_hover_text("Open a profile first")
        .clicked()
    {
        let policy = state.config.retention();
        match state.prune_now(&policy) {
            Ok(r) => state.notify(format!(
                "Pruned history: {} snapshot(s) and {} stored file(s) removed, {} freed",
                r.snapshots_removed,
                r.blobs_removed,
                human_size(r.bytes_freed)
            )),
            Err(e) => state.error(format!("Prune: {e}")),
        }
    }

    section(ui, "Game profiles");
    let current = state.active_profile_id().map(str::to_owned);
    egui::Grid::new("settings_profiles")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for p in &state.profiles {
                let open = current.as_deref() == Some(p.id.as_str());
                ui.label(if open {
                    RichText::new(&p.id).strong()
                } else {
                    RichText::new(&p.id)
                });
                ui.label(&p.name);
                ui.weak(version_label(p.game_version));
                ui.end_row();
            }
        });
    if state.profiles.is_empty() {
        ui.weak("No profiles yet.");
    }
    ui.horizontal(|ui| {
        if ui
            .button("Add game install...")
            .on_hover_text("Make a profile for a Diablo II or Median XL folder")
            .clicked()
        {
            state.open_add_install(None);
        }
        if ui.button("Open profiles folder").clicked() {
            let dir = state.paths.profiles.clone();
            if let Err(e) = std::process::Command::new("explorer").arg(&dir).spawn() {
                state.error(format!("Cannot open {}: {e}", dir.display()));
            }
        }
    });
}

pub fn version_label(v: GameVersion) -> &'static str {
    match v {
        GameVersion::V113c => "1.13c",
        GameVersion::V114d => "1.14d",
        GameVersion::D2r => "D2R",
    }
}

fn add_install(ctx: &Context, state: &mut AppState) {
    let Some(mut add) = state.settings.add.clone() else {
        return;
    };
    let suggestions = state.install_suggestions();
    let used_by =
        crate::installs::profiles_using(&state.profiles, std::path::Path::new(add.dir.trim()));
    let picking = state.settings.picking();
    let mut cancel = false;
    let mut confirm = false;
    let mut new_dir: Option<String> = None;
    let mut new_kind: Option<InstallKind> = None;
    let response = egui::Modal::new(Id::new("add_install_modal")).show(ctx, |ui| {
        ui.set_max_width(520.0);
        ui.heading("Add game install");
        ui.weak(
            r"The install folder is only read. The new profile is written to capybara\profiles\.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Folder");
            let edit = ui.add(egui::TextEdit::singleline(&mut add.dir).desired_width(320.0));
            if edit.lost_focus() || (edit.changed() && std::path::Path::new(&add.dir).is_dir()) {
                new_dir = Some(add.dir.clone());
            }
            if ui
                .add_enabled(!picking, egui::Button::new("Choose folder..."))
                .clicked()
            {
                state.pick_install_folder();
            }
        });
        if !suggestions.is_empty() {
            ui.label("Found on this computer:");
            for c in &suggestions {
                ui.horizontal(|ui| {
                    if ui.small_button("Use").clicked() {
                        new_dir = Some(c.path.to_string_lossy().into_owned());
                    }
                    ui.monospace(c.path.display().to_string());
                    ui.weak(&c.source);
                });
            }
        } else {
            ui.weak("No install was found in the registry or at the default locations.");
        }
        ui.separator();
        if !add.dir.trim().is_empty() && !used_by.is_empty() {
            ui.weak(format!(
                "Already used by profile(s): {}. Adding it again makes another profile.",
                used_by.join(", ")
            ));
        }
        match add.detected_kind() {
            Some(InstallKind::Vanilla) => {
                ui.colored_label(
                    Color32::from_rgb(120, 210, 130),
                    "Detected: Diablo II (d2data.mpq and d2exp.mpq)",
                );
            }
            Some(InstallKind::MedianXl) => {
                ui.colored_label(
                    Color32::from_rgb(120, 210, 130),
                    "Detected: Median XL (medianxl-*.mpq)",
                );
            }
            Some(InstallKind::Unknown) => {
                ui.colored_label(
                    Color32::from_rgb(235, 170, 80),
                    "Not recognised: no medianxl-*.mpq, and not both d2data.mpq and d2exp.mpq.",
                );
                ui.horizontal(|ui| {
                    ui.label("Treat it as");
                    for k in [InstallKind::Vanilla, InstallKind::MedianXl] {
                        if ui.radio(add.kind == k, k.label()).clicked() {
                            new_kind = Some(k);
                        }
                    }
                });
            }
            None => {}
        }
        if add.kind != InstallKind::Unknown {
            egui::Grid::new("add_install_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Profile id");
                    ui.text_edit_singleline(&mut add.id);
                    ui.end_row();
                    ui.label("Name");
                    ui.text_edit_singleline(&mut add.name);
                    ui.end_row();
                    ui.label("Game version");
                    egui::ComboBox::from_id_salt("add_install_version")
                        .selected_text(version_label(add.version))
                        .show_ui(ui, |ui| {
                            for v in [GameVersion::V113c, GameVersion::V114d] {
                                ui.selectable_value(&mut add.version, v, version_label(v));
                            }
                        });
                    ui.end_row();
                });
        }
        if let Some(e) = &add.error {
            ui.colored_label(Color32::from_rgb(235, 110, 90), e);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let blocker = add.blocker();
            if ui
                .add_enabled(blocker.is_none(), egui::Button::new("Add profile"))
                .on_disabled_hover_text(blocker.unwrap_or(""))
                .clicked()
            {
                confirm = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
            if let Some(b) = blocker {
                ui.weak(b);
            }
        });
    });
    if cancel || (response.should_close() && !confirm) {
        state.settings.add = None;
        return;
    }
    // Keep what was typed, then apply the actions.
    if let Some(open) = state.settings.add.as_mut() {
        open.dir = add.dir.clone();
        open.id = add.id.clone();
        open.name = add.name.clone();
        open.version = add.version;
    }
    if let Some(d) = new_dir {
        state.add_install_set_dir(&d);
    }
    if let Some(k) = new_kind {
        state.add_install_set_kind(k);
    }
    if confirm {
        match state.confirm_add_install() {
            Ok(id) => state.notify(format!("Profile {id} added and opened")),
            Err(e) => {
                if let Some(open) = state.settings.add.as_mut() {
                    open.error = Some(e);
                }
            }
        }
    }
}
