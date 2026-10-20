//! The Bevy app: plugins, cameras and the systems that tie state to the UI.

use std::time::Duration;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::{PrimaryWindow, WindowResolution};
use bevy_egui::{
    EguiContexts, EguiGlobalSettings, EguiInput, EguiPlugin, EguiPreUpdateSet,
    EguiPrimaryContextPass, PrimaryEguiContext,
};
use d2_assets::Profile;

use crate::config::Config;
use crate::hooks::{self, DebugHooks};
use crate::paths::Paths;
use crate::render::{MapCamera, MapRender, RenderPlugin};
use crate::script_run::{self, ScriptRunner};
use crate::splash::Splash;
use crate::state::{AppState, CatalogState};
use crate::ui::{self, Dock};

const PERSIST_DEBOUNCE: Duration = Duration::from_millis(1500);

#[derive(Resource, Default)]
struct PersistTimer {
    since_change: Option<Duration>,
}

/// Everything loaded before the window exists.
pub struct Prepared {
    pub state: AppState,
    pub dock: Dock,
}

pub fn prepare(paths: Paths) -> Prepared {
    let first_run = !paths.config_file.exists();
    let mut notes: Vec<(bool, String)> = Vec::new(); // (is_error, text)

    let config = match Config::load(&paths.config_file) {
        Ok(c) => c,
        Err(e) => {
            // Keep the broken file for the user, then start from defaults.
            let bad = paths.config_file.with_extension("toml.bad");
            let kept = std::fs::rename(&paths.config_file, &bad).is_ok();
            notes.push((
                true,
                format!(
                    "config.toml is unreadable ({e:#}); using defaults{}",
                    if kept {
                        format!(" (old file kept as {})", bad.display())
                    } else {
                        String::new()
                    }
                ),
            ));
            Config::default()
        }
    };

    if first_run || !d2_assets::installs::has_profile_files(&paths.profiles) {
        // The default locations, plus the install path the Windows registry
        // names (read-only).
        let (vanilla, mxl) = d2_assets::installs::InstallSearch::system().candidates();
        match crate::installs::seed_profiles(&paths.profiles, &vanilla, &mxl) {
            Ok(written) => {
                for f in written {
                    notes.push((false, format!("Wrote default profile {}", f.display())));
                }
            }
            Err(e) => notes.push((true, format!("Could not write default profiles: {e:#}"))),
        }
    }

    let profiles = match Profile::load_dir(&paths.profiles) {
        Ok(p) => p,
        Err(e) => {
            notes.push((true, format!("Profile error: {e}")));
            load_profiles_leniently(&paths.profiles, &mut notes)
        }
    };

    let (dock, dock_problems) = Dock::load(&paths.sprites_layout_file);
    let mut state = AppState::new(paths.clone(), config, profiles);
    state.info(format!("Data folder: {}", paths.root.display()));
    for (is_error, text) in notes {
        if is_error {
            state.error(text);
        } else {
            state.info(text);
        }
    }
    for p in dock_problems {
        state.warn(p);
    }
    if state.profiles.is_empty() {
        state.warn(format!(
            "No game profiles. Add a profile .toml to {}",
            paths.profiles.display()
        ));
    }
    Prepared { state, dock }
}

/// When `load_dir` rejects the folder because of one bad file, keep the rest.
fn load_profiles_leniently(dir: &std::path::Path, notes: &mut Vec<(bool, String)>) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    for file in files {
        match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|t| Profile::from_toml(&t).map_err(|e| e.to_string()))
        {
            Ok(p) if out.iter().any(|o| o.id == p.id) => notes.push((
                true,
                format!(
                    "{}: duplicate profile id {:?}, skipped",
                    file.display(),
                    p.id
                ),
            )),
            Ok(p) => out.push(p),
            Err(e) => notes.push((true, format!("{}: {e}", file.display()))),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub fn run(startup: Prepared) -> AppExit {
    let Prepared { state, dock } = startup;
    let hooks = DebugHooks::from_env();
    let script = ScriptRunner::from_env();
    // Verification runs start without the loading screen (a script can
    // still show it with `splash`).
    let splash =
        Splash::new(state.config.show_loading_screen && !hooks.automated() && !script.active());
    let window = Window {
        title: crate::PRODUCT.into(),
        resolution: match hooks.scale_factor {
            Some(f) => WindowResolution::new(
                state.config.window.width as u32,
                state.config.window.height as u32,
            )
            .with_scale_factor_override(f),
            None => WindowResolution::new(
                state.config.window.width as u32,
                state.config.window.height as u32,
            ),
        },
        ..default()
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(window),
            // Closing the window asks about unsaved maps first.
            close_when_requested: false,
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_plugins(RenderPlugin)
        .insert_resource(ClearColor(Color::srgb(0.07, 0.07, 0.08)))
        .insert_resource(state)
        .insert_resource(dock)
        .insert_resource(PersistTimer::default())
        .insert_resource(hooks)
        .insert_resource(script)
        .insert_resource(splash)
        .add_systems(Startup, (setup_cameras, begin_startup_profile))
        .add_systems(Update, crate::branding::set_window_icons)
        .add_systems(Update, crate::ui::studio::export::capture_view)
        .add_systems(
            Update,
            (
                poll_background,
                autosave_tick,
                hooks::run_hooks,
                script_run::run_script,
                track_window_size,
                handle_close_request,
                handle_quit,
                persist,
            )
                .chain(),
        )
        .add_systems(
            PreUpdate,
            inject_input
                .after(EguiPreUpdateSet::ProcessInput)
                .before(EguiPreUpdateSet::BeginPass),
        )
        .add_systems(EguiPrimaryContextPass, ui_system)
        .add_systems(
            Last,
            persist_on_exit.after(bevy::window::exit_on_all_closed),
        )
        .run()
}

fn setup_cameras(mut commands: Commands, mut egui: ResMut<EguiGlobalSettings>) {
    // The world camera is restricted to the Map panel; egui gets its own.
    egui.auto_create_primary_context = false;
    commands.spawn((
        Camera2d,
        MapCamera,
        Camera {
            order: 0,
            ..default()
        },
        crate::render::map_camera_extras(),
    ));
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        // Cameras on one target share its depth texture, so they must agree
        // with the map camera on the sample count.
        bevy::render::view::Msaa::Off,
        Camera {
            order: 1,
            output_mode: bevy::camera::CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));
}

/// Open the profile from the last session (or `CAPY_PROFILE`).
fn begin_startup_profile(
    mut state: ResMut<AppState>,
    hooks: Res<DebugHooks>,
    mut render: ResMut<MapRender>,
    mut splash: ResMut<Splash>,
) {
    state.forced_context = hooks.forced_context;
    state.index_disabled = hooks.no_index;
    if hooks.map_only {
        render.force_div = Some(hooks.map_only_div());
        render.hide_outlines = true;
        render.static_objects = true;
    }
    let wanted = hooks
        .profile
        .clone()
        .or_else(|| state.config.last_profile.clone());
    // First run (nothing wanted): vanilla before alphabetical order.
    let id = crate::installs::startup_profile(&state.profiles, wanted.as_deref());
    if splash.active() {
        // The loading screen reads its door frames from the same profile.
        let profile = id
            .as_ref()
            .and_then(|id| state.profiles.iter().find(|p| &p.id == id).cloned());
        splash.start_build(profile, state.paths.saves.clone());
    }
    if let Some(id) = id {
        state.open_profile_by_id(&id);
    }
}

/// Feeds the input events scripts queued (`click`, `press`) to egui, one
/// batch per frame, as if they came from the mouse and keyboard.
fn inject_input(
    mut state: ResMut<AppState>,
    mut inputs: Query<&mut EguiInput, With<PrimaryEguiContext>>,
) {
    if state.shell.inject.is_empty() {
        return;
    }
    let Ok(mut input) = inputs.single_mut() else {
        return;
    };
    if let Some(batch) = state.shell.inject.pop_front() {
        input.0.events.extend(batch);
    }
}

fn poll_background(mut state: ResMut<AppState>) {
    state.poll_tasks();
}

/// Autosave dirty maps to history every `autosave_minutes`.
fn autosave_tick(time: Res<Time>, mut state: ResMut<AppState>) {
    state.autosave_tick(time.delta());
}

fn ui_system(
    mut contexts: EguiContexts,
    mut state: ResMut<AppState>,
    mut dock: ResMut<Dock>,
    hooks: Res<DebugHooks>,
    mut splash: ResMut<Splash>,
) -> Result {
    if hooks.map_only {
        return Ok(()); // the map-only screenshot has no UI
    }
    let ctx = contexts.ctx_mut()?.clone();
    crate::splash::take_input(&ctx, &mut splash);
    ui::draw(&ctx, &mut state, &mut dock);
    if splash.active() {
        // Ready once the start profile's maps can be listed with their
        // context (or there is nothing to load).
        let ready = state.loading.is_none() && !matches!(state.catalog, CatalogState::Building);
        let dialog_open = state.edit.modal_open()
            || state.sprites.modal_open()
            || state.history.modal_open()
            || state.settings.modal_open()
            || state.settings.open
            || state.browser.prompt.is_some()
            || state.browser.palette.open;
        crate::splash::draw(&ctx, &mut splash, ready, dialog_open);
    }
    Ok(())
}

/// Remember the window size (logical points) for the next launch.
fn track_window_size(window: Query<&Window, With<PrimaryWindow>>, mut state: ResMut<AppState>) {
    let Ok(w) = window.single() else { return };
    let (width, height) = (w.width(), w.height());
    if width < 320.0 || height < 240.0 {
        return; // minimised or mid-resize
    }
    let c = &mut state.config.window;
    if (c.width - width).abs() > 0.5 || (c.height - height).abs() > 0.5 {
        c.width = width;
        c.height = height;
        state.config_dirty = true;
    }
}

/// The window's close button: quit, after asking about unsaved maps.
fn handle_close_request(
    mut requests: MessageReader<bevy::window::WindowCloseRequested>,
    mut state: ResMut<AppState>,
) {
    if requests.read().next().is_some() {
        state.request_quit();
    }
}

fn handle_quit(state: Res<AppState>, mut exit: MessageWriter<AppExit>) {
    if state.quit_requested {
        exit.write(AppExit::Success);
    }
}

fn write_out(state: &mut AppState, dock: &mut Dock) {
    if state.config_dirty {
        match state.config.save(&state.paths.config_file) {
            Ok(()) => state.config_dirty = false,
            Err(e) => {
                state.config_dirty = false; // do not retry every frame
                state.error(format!("Could not save config.toml: {e:#}"));
            }
        }
    }
    let sprites_file = state.paths.sprites_layout_file.clone();
    if let Err(e) = dock.save_if_changed(&sprites_file) {
        state.warn(format!("Could not save the panel layout: {e}"));
    }
}

/// Debounced write of config and layout.
fn persist(
    time: Res<Time>,
    mut timer: ResMut<PersistTimer>,
    mut state: ResMut<AppState>,
    mut dock: ResMut<Dock>,
) {
    // Layout changes are detected inside `save_if_changed`; checking it every
    // frame would serialise the dock constantly, so poll on the debounce tick.
    let elapsed = timer.since_change.get_or_insert(Duration::ZERO);
    *elapsed += time.delta();
    if *elapsed >= PERSIST_DEBOUNCE {
        *elapsed = Duration::ZERO;
        write_out(&mut state, &mut dock);
    }
}

fn persist_on_exit(
    mut exits: MessageReader<AppExit>,
    mut state: ResMut<AppState>,
    mut dock: ResMut<Dock>,
) {
    if exits.read().next().is_some() {
        write_out(&mut state, &mut dock);
    }
}
