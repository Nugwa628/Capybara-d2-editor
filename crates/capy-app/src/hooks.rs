//! Development and verification hooks, driven by environment variables.
//!
//! * `CAPY_PROFILE=<id>`        open this profile instead of the last one
//! * `CAPY_OPEN_MAP=<game path>` open this map once the profile is ready
//! * `CAPY_ZOOM=<divisor>`      zoom to 1:N (one of the zoom steps) after the fit
//! * `CAPY_LVLTYPE=<id>` `CAPY_DEF=<def>`  use this LvlType (and LvlPrest Def)
//!   for every map that is opened instead of the catalog's choice (not saved)
//! * `CAPY_TAB=<panel title>`   show that panel (Tiles, Layers, Properties, History, Log, Objects, Sprite preview)
//! * `CAPY_EXIT_WHEN_INDEXED=1` quit once the asset index is ready (timing runs)
//! * `CAPY_NO_INDEX=1`          do not build the asset index
//! * `CAPY_SCREENSHOT=<file>`   save a PNG of the window a moment after the map
//!   is on screen, then quit (set `CAPY_KEEP_OPEN=1` to stay open)
//! * `CAPY_SCREENSHOT_MAP_ONLY=1`  with `CAPY_SCREENSHOT`: no UI. Render only
//!   the map, black background, at exactly 1:N (`CAPY_ZOOM`, any N from 1 to
//!   16, default 1), into an off-screen image of the size the reference
//!   renderer would produce, and save that. This is what the GPU-vs-CPU
//!   parity test compares.
//! * `CAPY_SCALE_FACTOR=<f>`    UI scale instead of the monitor's (screenshots)

use std::path::PathBuf;

use bevy::camera::{ClearColorConfig, RenderTarget, ScalingMode};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use d2_assets::GamePath;

use crate::doc::RenderState;
use crate::iso::ZOOM_STEPS;
use crate::render::{MapRender, map_camera_extras};
use crate::state::AppState;

/// Frames to let the freshly built meshes and UI settle before the shot.
const SETTLE_FRAMES: u32 = 30;
/// Seconds to let pipelines compile and the atlas upload (debug builds are slow
/// and a frame count says nothing about that).
const SETTLE_SECS: f32 = 4.0;
/// Give up on a screenshot that never appears.
const GIVE_UP_FRAMES: u32 = 1800;
/// Largest off-screen image edge (what every GPU we care about accepts).
const MAX_SHOT_EDGE: i32 = 8192;

/// Where the map-only screenshot stands.
#[derive(Default)]
enum MapOnlyStage {
    /// Waiting for the map to be built.
    #[default]
    Waiting,
    /// The off-screen camera exists; let a few frames render.
    Settling(u32),
    /// The capture was requested.
    Capturing(u32),
}

#[derive(Resource, Default)]
pub struct DebugHooks {
    pub profile: Option<String>,
    open_map: Option<String>,
    zoom: Option<u32>,
    screenshot: Option<PathBuf>,
    keep_open: bool,
    /// `CAPY_SCREENSHOT_MAP_ONLY`: the UI is not drawn at all.
    pub map_only: bool,
    pub forced_context: Option<(u32, Option<u32>)>,
    pub no_index: bool,
    /// `CAPY_SCALE_FACTOR`: the window's UI scale instead of the monitor's
    /// (1 gives a 1600x900 shot of a 1600x900 window on a 150% display).
    pub scale_factor: Option<f32>,
    tab: Option<String>,
    exit_when_indexed: bool,
    frames_since_map: u32,
    frames_since_shot: Option<u32>,
    settle_since: Option<std::time::Instant>,
    map_only_stage: MapOnlyStage,
}

impl DebugHooks {
    pub fn from_env() -> DebugHooks {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let lvl_type: Option<u32> = var("CAPY_LVLTYPE").and_then(|v| v.parse().ok());
        let def: Option<u32> = var("CAPY_DEF").and_then(|v| v.parse().ok());
        DebugHooks {
            profile: var("CAPY_PROFILE"),
            open_map: var("CAPY_OPEN_MAP"),
            zoom: var("CAPY_ZOOM").and_then(|v| v.parse().ok()),
            screenshot: var("CAPY_SCREENSHOT").map(PathBuf::from),
            keep_open: var("CAPY_KEEP_OPEN").is_some(),
            map_only: var("CAPY_SCREENSHOT_MAP_ONLY").is_some_and(|v| v != "0")
                && var("CAPY_SCREENSHOT").is_some(),
            forced_context: lvl_type.map(|t| (t, def)),
            tab: var("CAPY_TAB"),
            exit_when_indexed: var("CAPY_EXIT_WHEN_INDEXED").is_some_and(|v| v != "0"),
            no_index: var("CAPY_NO_INDEX").is_some_and(|v| v != "0"),
            scale_factor: var("CAPY_SCALE_FACTOR")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|f| f.is_finite() && *f > 0.0),
            ..DebugHooks::default()
        }
    }

    /// A verification run (screenshot or timing) that must not wait for
    /// the loading screen.
    pub fn automated(&self) -> bool {
        self.screenshot.is_some() || self.map_only || self.exit_when_indexed
    }

    /// The divisor a map-only screenshot is laid out for.
    pub fn map_only_div(&self) -> u32 {
        self.zoom.unwrap_or(1).clamp(1, 16)
    }
}

/// Marks the off-screen camera of the map-only screenshot.
#[derive(Component)]
struct ShotCamera;

pub fn run_hooks(
    mut commands: Commands,
    mut hooks: ResMut<DebugHooks>,
    mut state: ResMut<AppState>,
    render: Res<MapRender>,
    mut images: ResMut<Assets<Image>>,
    mut exit: MessageWriter<AppExit>,
) {
    if hooks.exit_when_indexed
        && matches!(
            state.index.state,
            crate::index_task::IndexState::Ready(_)
                | crate::index_task::IndexState::Failed(_)
                | crate::index_task::IndexState::Cancelled
        )
    {
        exit.write(AppExit::Success);
    }
    if let Some(title) = hooks.tab.take()
        && !crate::ui::shell::focus_panel(&mut state, &title)
    {
        state.warn(format!("CAPY_TAB={title:?} is not a panel"));
    }
    if state.session.is_some()
        && let Some(path) = hooks.open_map.take()
    {
        state.open_map(&GamePath::new(&path));
        hooks.frames_since_map = 0;
    }

    if hooks.map_only {
        map_only_shot(
            &mut commands,
            &mut hooks,
            &mut state,
            &render,
            &mut images,
            &mut exit,
        );
        return;
    }

    let fitted = state.active_doc().is_some_and(|d| d.fitted);
    if fitted {
        if let Some(divisor) = hooks.zoom.take() {
            match ZOOM_STEPS.iter().position(|&d| d == divisor) {
                Some(index) => {
                    if let Some(doc) = state.active_doc_mut() {
                        doc.view.zoom = index;
                    }
                }
                None => state.warn(format!("CAPY_ZOOM={divisor} is not a zoom step")),
            }
        }
        hooks.frames_since_map += 1;
    }

    let Some(file) = hooks.screenshot.clone() else {
        return;
    };
    // Wait until the textures are in (or failed for good) so the shot shows
    // the map, not the placeholder fill.
    let drawn = state.active_doc().is_some_and(|d| {
        matches!(
            d.render.state,
            RenderState::Ready | RenderState::Failed(_) | RenderState::ProfileClosed
        )
    });
    if fitted && drawn && hooks.zoom.is_none() && hooks.settle_since.is_none() {
        hooks.settle_since = Some(std::time::Instant::now());
    }
    let settled = hooks
        .settle_since
        .is_some_and(|t| t.elapsed().as_secs_f32() >= SETTLE_SECS);
    match hooks.frames_since_shot {
        None => {
            if fitted
                && drawn
                && settled
                && hooks.zoom.is_none()
                && hooks.frames_since_map >= SETTLE_FRAMES
            {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::remove_file(&file);
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(file));
                hooks.frames_since_shot = Some(0);
            }
        }
        Some(n) => {
            hooks.frames_since_shot = Some(n + 1);
            if file.exists() {
                state.info(format!("Screenshot saved to {}", file.display()));
                hooks.screenshot = None;
                if !hooks.keep_open {
                    exit.write(AppExit::Success);
                }
            } else if n > GIVE_UP_FRAMES {
                state.error("Screenshot was never written");
                hooks.screenshot = None;
                if !hooks.keep_open {
                    exit.write(AppExit::error());
                }
            }
        }
    }
}

fn fail(
    hooks: &mut DebugHooks,
    state: &mut AppState,
    exit: &mut MessageWriter<AppExit>,
    text: String,
) {
    state.error(text);
    hooks.screenshot = None;
    if !hooks.keep_open {
        exit.write(AppExit::error());
    }
}

/// The map-only screenshot state machine (see the module docs).
fn map_only_shot(
    commands: &mut Commands,
    hooks: &mut DebugHooks,
    state: &mut AppState,
    render: &MapRender,
    images: &mut Assets<Image>,
    exit: &mut MessageWriter<AppExit>,
) {
    let Some(file) = hooks.screenshot.clone() else {
        return;
    };
    let div = hooks.map_only_div();
    hooks.frames_since_map += 1;
    match hooks.map_only_stage {
        MapOnlyStage::Waiting => {
            if hooks.frames_since_map > GIVE_UP_FRAMES * 4 {
                fail(
                    hooks,
                    state,
                    exit,
                    "the map never finished building".to_owned(),
                );
                return;
            }
            let Some(doc) = state.active_doc() else {
                return;
            };
            match &doc.render.state {
                RenderState::Failed(e) => {
                    let text = format!("map-only screenshot: {e}");
                    fail(hooks, state, exit, text);
                    return;
                }
                RenderState::Ready => {}
                _ => return,
            }
            if render.mesh_div(doc.id) != Some(div) {
                return; // still laid out for another zoom
            }
            if !state.objects_settled(doc) {
                return; // drawn without objects so far
            }
            let Some((x0, y0, x1, y1)) = render.canvas(doc, div) else {
                fail(hooks, state, exit, "the map draws nothing".to_owned());
                return;
            };
            let (w, h) = (x1 - x0, y1 - y0);
            if w <= 0 || h <= 0 || w > MAX_SHOT_EDGE || h > MAX_SHOT_EDGE {
                let text = format!("canvas {w}x{h} at 1:{div} is out of range; raise CAPY_ZOOM");
                fail(hooks, state, exit, text);
                return;
            }
            let image = Image::new_target_texture(
                w as u32,
                h as u32,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                None,
            );
            let handle = images.add(image);
            let n = div as f32;
            // The canvas' left/top edge in 1:1 pixels, then to its centre.
            let (cx, cy) = (
                (x0 as f32 + w as f32 / 2.0) * n,
                (y0 as f32 + h as f32 / 2.0) * n,
            );
            commands.spawn((
                ShotCamera,
                Camera2d,
                Camera {
                    order: -1,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                RenderTarget::Image(handle.clone().into()),
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed {
                        width: w as f32 * n,
                        height: h as f32 * n,
                    },
                    ..OrthographicProjection::default_2d()
                }),
                Transform::from_xyz(
                    cx + crate::render::SAMPLE_BIAS + crate::render::WORLD_SHIFT_X,
                    -cy - crate::render::SAMPLE_BIAS,
                    0.0,
                ),
                map_camera_extras(),
            ));
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::remove_file(&file);
            state.info(format!("Map-only screenshot: {w}x{h} at 1:{div}"));
            // Keep the handle alive in the resource-free way: the camera holds it.
            hooks.map_only_stage = MapOnlyStage::Settling(0);
            hooks.settle_since = Some(std::time::Instant::now());
            commands.insert_resource(ShotImage(handle));
        }
        MapOnlyStage::Settling(n) => {
            let settled = hooks
                .settle_since
                .is_some_and(|t| t.elapsed().as_secs_f32() >= SETTLE_SECS);
            if n >= 10 && settled {
                // The target image handle lives in `ShotImage`; capture it next frame.
                hooks.map_only_stage = MapOnlyStage::Capturing(0);
                commands.queue(move |world: &mut World| {
                    let handle = world.get_resource::<ShotImage>().map(|s| s.0.clone());
                    if let Some(handle) = handle {
                        world
                            .spawn(Screenshot::image(handle))
                            .observe(save_to_disk(file));
                    }
                });
            } else {
                hooks.map_only_stage = MapOnlyStage::Settling(n + 1);
            }
        }
        MapOnlyStage::Capturing(n) => {
            hooks.map_only_stage = MapOnlyStage::Capturing(n + 1);
            if file.exists() {
                state.info(format!("Map-only screenshot saved to {}", file.display()));
                hooks.screenshot = None;
                if !hooks.keep_open {
                    exit.write(AppExit::Success);
                }
            } else if n > GIVE_UP_FRAMES {
                fail(
                    hooks,
                    state,
                    exit,
                    "Screenshot was never written".to_owned(),
                );
            }
        }
    }
}

/// Keeps the off-screen target alive while the capture runs.
#[derive(Resource)]
struct ShotImage(Handle<Image>);
