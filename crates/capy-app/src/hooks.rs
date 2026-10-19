//! Development and verification hooks, driven by environment variables.
//!
//! * `CAPY_PROFILE=<id>`        open this profile instead of the last one
//! * `CAPY_OPEN_MAP=<game path>` open this map once the profile is ready
//! * `CAPY_ZOOM=<divisor>`      zoom to 1:N (one of the zoom steps) after the fit
//! * `CAPY_SCREENSHOT=<file>`   save a PNG of the window a moment after the map
//!   is on screen, then quit (set `CAPY_KEEP_OPEN=1` to stay open)

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use d2_assets::GamePath;

use crate::iso::ZOOM_STEPS;
use crate::state::AppState;

/// Frames to let the freshly built meshes and UI settle before the shot.
const SETTLE_FRAMES: u32 = 30;
/// Give up on a screenshot that never appears.
const GIVE_UP_FRAMES: u32 = 1800;

#[derive(Resource, Default)]
pub struct DebugHooks {
    pub profile: Option<String>,
    open_map: Option<String>,
    zoom: Option<u32>,
    screenshot: Option<PathBuf>,
    keep_open: bool,
    frames_since_map: u32,
    frames_since_shot: Option<u32>,
}

impl DebugHooks {
    pub fn from_env() -> DebugHooks {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        DebugHooks {
            profile: var("CAPY_PROFILE"),
            open_map: var("CAPY_OPEN_MAP"),
            zoom: var("CAPY_ZOOM").and_then(|v| v.parse().ok()),
            screenshot: var("CAPY_SCREENSHOT").map(PathBuf::from),
            keep_open: var("CAPY_KEEP_OPEN").is_some(),
            ..DebugHooks::default()
        }
    }
}

pub fn run_hooks(
    mut commands: Commands,
    mut hooks: ResMut<DebugHooks>,
    mut state: ResMut<AppState>,
    mut exit: MessageWriter<AppExit>,
) {
    if state.session.is_some()
        && let Some(path) = hooks.open_map.take()
    {
        state.open_map(&GamePath::new(&path));
        hooks.frames_since_map = 0;
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
    match hooks.frames_since_shot {
        None => {
            if fitted && hooks.zoom.is_none() && hooks.frames_since_map >= SETTLE_FRAMES {
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
