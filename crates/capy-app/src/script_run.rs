//! Runs a `CAPY_SCRIPT` file (see [`crate::edit::script`] for the language):
//! one command per line once the profile is ready, then the app exits with
//! code 0, or with code 1 and a message on the first command that fails.
//!
//! `CAPY_KEEP_OPEN=1` keeps the window open after the last command.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::egui;
use capy_edit::{CellRect, CellValue, SelectOp};
use capy_history::{SnapshotId, SnapshotKind};
use d2_assets::GamePath;

use capy_paint::{GradientShape, OutlineSide};
use capy_sprite::SpriteFormat;

use crate::browser::scope::Scope;
use crate::browser::{SearchStatus, THUMB_MAX, THUMB_MIN};
use crate::config::AppMode;
use crate::doc::RenderState;
use crate::edit::ops::EditOp;
use crate::edit::paste::PasteSession;
use crate::edit::script::GradientSpec;
use crate::edit::script::{Cmd, Line, SplashArg, parse_script};
use crate::edit::state::MoveDialog;
use crate::history;
use crate::index_task::IndexState;
use crate::iso::ZOOM_STEPS;
use crate::splash::{Pin, Splash};
use crate::sprites::convert::Layout;
use crate::sprites::dialog::Purpose;
use crate::sprites::editor::{EditorState, GradientMode, Mods, PaintTool};
use crate::sprites::{Focus, Target};
use crate::state::AppState;

/// Seconds a screenshot waits after the map is drawn, for pipelines and the
/// atlas upload (and for a rebuild after the last edit).
const SETTLE_SECS: f32 = 4.0;
/// Give up on a screenshot or a profile that never appears.
const GIVE_UP_FRAMES: u32 = 3600;
/// Give up on a command that keeps waiting (an index build included).
const GIVE_UP_RETRY_SECS: u64 = 30 * 60;
/// Where `tile-menu` opens the tile menu (screen points).
const TILE_MENU_AT: (f32, f32) = (360.0, 220.0);

enum Wait {
    Frames(u32),
    Shot(Shot),
}

struct Shot {
    file: PathBuf,
    started: Instant,
    requested: bool,
    frames: u32,
    /// When the map was first seen drawn: the settle time counts from
    /// there, so the atlas upload and pipelines are done.
    drawn_since: Option<Instant>,
}

#[derive(Resource, Default)]
pub struct ScriptRunner {
    active: bool,
    cmds: VecDeque<Line>,
    problem: Option<String>,
    wait: Option<Wait>,
    done: bool,
    keep_open: bool,
    idle_frames: u32,
    started: bool,
    /// When the command at the front started waiting (`Step::Retry`).
    retry_since: Option<Instant>,
    /// `autosave-now`: the autosave run count when it started its worker.
    awaiting_autosave: Option<u64>,
}

impl ScriptRunner {
    /// A script is (or was) running.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Reads `CAPY_SCRIPT`. An unreadable or unparsable script is kept as a
    /// problem and reported (exit code 1) when the app starts running it.
    pub fn from_env() -> ScriptRunner {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let Some(file) = var("CAPY_SCRIPT") else {
            return ScriptRunner::default();
        };
        let keep_open = var("CAPY_KEEP_OPEN").is_some();
        let mut runner = ScriptRunner {
            active: true,
            keep_open,
            ..ScriptRunner::default()
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => match parse_script(&text) {
                Ok(lines) => runner.cmds = lines.into(),
                Err(e) => runner.problem = Some(format!("script {file}: {e}")),
            },
            Err(e) => runner.problem = Some(format!("cannot read script {file}: {e}")),
        }
        runner
    }
}

fn fail(
    runner: &mut ScriptRunner,
    state: &mut AppState,
    exit: &mut MessageWriter<AppExit>,
    line: Option<usize>,
    text: String,
) {
    let message = match line {
        Some(n) => format!("script line {n}: {text}"),
        None => text,
    };
    eprintln!("{message}");
    state.error(message);
    runner.done = true;
    exit.write(AppExit::error());
}

/// The most recent log line, for "what went wrong" in a failure message.
fn last_log(state: &AppState) -> String {
    state
        .log
        .last()
        .map_or_else(String::new, |e| format!(": {}", e.text))
}

/// What running one command asks of the loop.
enum Step {
    Next,
    /// Stop for this frame; the command is run again next frame.
    Retry,
    /// Stop for this frame; the command is finished but must settle.
    Wait(Wait),
    Fail(String),
}

pub fn run_script(
    mut commands: Commands,
    mut runner: ResMut<ScriptRunner>,
    mut state: ResMut<AppState>,
    mut splash: ResMut<Splash>,
    mut exit: MessageWriter<AppExit>,
) {
    if !runner.active || runner.done {
        return;
    }
    if let Some(problem) = runner.problem.take() {
        fail(&mut runner, &mut state, &mut exit, None, problem);
        return;
    }
    if state.session.is_none() {
        // The profile is still opening (or never will).
        runner.idle_frames += 1;
        let gave_up = state.loading.is_none() && runner.idle_frames > 60;
        if gave_up || runner.idle_frames > GIVE_UP_FRAMES {
            fail(
                &mut runner,
                &mut state,
                &mut exit,
                None,
                "script: the profile did not open".to_owned(),
            );
        }
        return;
    }
    if !runner.started {
        runner.started = true;
        state.edit.use_os_clipboard = false; // never disturb the user's clipboard
        state.sprites.use_os_clipboard = false;
    }

    // A command that is still settling.
    match runner.wait.take() {
        Some(Wait::Frames(n)) if n > 1 => {
            runner.wait = Some(Wait::Frames(n - 1));
            return;
        }
        Some(Wait::Frames(_)) | None => {}
        Some(Wait::Shot(shot)) => {
            match poll_shot(&mut commands, &mut state, shot) {
                ShotPoll::Waiting(shot) => runner.wait = Some(Wait::Shot(shot)),
                ShotPoll::Done => {}
                ShotPoll::Failed(text) => {
                    fail(&mut runner, &mut state, &mut exit, None, text);
                }
            }
            return;
        }
    }

    while let Some((line, cmd)) = runner.cmds.pop_front() {
        let waited = runner.retry_since.map(|t| t.elapsed());
        let result = match splash_step(&mut splash, &state, &cmd, waited) {
            Some(result) => result,
            None => step(&mut state, &cmd, &mut runner.awaiting_autosave),
        };
        if !matches!(result, Step::Retry) {
            runner.retry_since = None;
        }
        match result {
            Step::Next => {}
            Step::Retry => {
                let since = *runner.retry_since.get_or_insert_with(Instant::now);
                if since.elapsed().as_secs() > GIVE_UP_RETRY_SECS {
                    fail(
                        &mut runner,
                        &mut state,
                        &mut exit,
                        Some(line),
                        "timed out waiting".to_owned(),
                    );
                    return;
                }
                runner.cmds.push_front((line, cmd));
                return;
            }
            Step::Wait(wait) => {
                runner.wait = Some(wait);
                return;
            }
            Step::Fail(text) => {
                fail(&mut runner, &mut state, &mut exit, Some(line), text);
                return;
            }
        }
    }
    runner.done = true;
    state.info("Script finished");
    if !runner.keep_open {
        exit.write(AppExit::Success);
    }
}

/// The loading-screen commands (`splash ...`).
fn splash_step(
    splash: &mut Splash,
    state: &AppState,
    cmd: &Cmd,
    waited: Option<std::time::Duration>,
) -> Option<Step> {
    let build = |splash: &mut Splash| {
        let profile = state.session.as_ref().map(|s| s.profile.clone());
        splash.start_build(profile, state.paths.saves.clone());
    };
    Some(match cmd {
        Cmd::Splash(SplashArg::Off) => {
            splash.close();
            Step::Next
        }
        Cmd::Splash(SplashArg::Plain) => {
            build(splash);
            splash.pin(Pin::Plain);
            Step::Next
        }
        Cmd::Splash(SplashArg::Auto) => {
            build(splash);
            splash.restart();
            Step::Next
        }
        Cmd::Splash(SplashArg::Frame(n)) => {
            build(splash);
            match splash.built() {
                None => Step::Retry,
                Some(Err(e)) => Step::Fail(format!("splash {n}: no door frames: {e}")),
                Some(Ok(count)) if *n >= count => {
                    Step::Fail(format!("splash {n}: the door has {count} frames"))
                }
                Some(Ok(_)) => {
                    splash.pin(Pin::Frame(*n));
                    Step::Next
                }
            }
        }
        Cmd::SplashWaitClosed => {
            if !splash.active() {
                Step::Next
            } else if waited.is_some_and(|w| w.as_secs() >= 30) {
                Step::Fail("splash-wait-closed: still showing after 30 s".to_owned())
            } else {
                Step::Retry
            }
        }
        Cmd::SplashSave { frame, scale, file } => {
            build(splash);
            if splash.built().is_none() {
                return Some(Step::Retry);
            }
            match splash.view(*frame, *scale) {
                Ok(img) => {
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    match image::save_buffer(
                        file,
                        &img.data,
                        img.width as u32,
                        img.height as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => Step::Next,
                        Err(e) => Step::Fail(format!("splash-save {}: {e}", file.display())),
                    }
                }
                Err(e) => Step::Fail(format!("splash-save: {e}")),
            }
        }
        _ => return None,
    })
}

fn step(state: &mut AppState, cmd: &Cmd, awaiting_autosave: &mut Option<u64>) -> Step {
    if let Cmd::Open(path) = cmd {
        let wanted = GamePath::new(path);
        state.open_map(&wanted);
        return match state.active_doc() {
            Some(doc) if doc.path == wanted => Step::Next,
            _ => Step::Fail(format!(
                "open {path}: could not open the map{}",
                last_log(state)
            )),
        };
    }
    if let Cmd::Wait(n) = cmd {
        return Step::Wait(Wait::Frames(*n));
    }
    if let Some(step) = sprite_step(state, cmd) {
        return step;
    }
    if let Some(step) = tool_step(state, cmd) {
        return step;
    }
    if let Some(step) = palette_step(state, cmd) {
        return step;
    }
    if let Some(step) = browser_step(state, cmd) {
        return step;
    }
    if let Some(step) = history_step(state, cmd, awaiting_autosave) {
        return step;
    }
    if let Some(step) = objects_step(state, cmd) {
        return step;
    }
    if let Some(step) = shell_step(state, cmd) {
        return step;
    }
    if let Cmd::Studio(c) = cmd {
        return match crate::ui::studio::script::step(state, c) {
            Ok(true) => Step::Next,
            Ok(false) => Step::Retry,
            Err(e) => Step::Fail(e),
        };
    }
    // Screenshots also work without a map (Sprites mode).
    if let Cmd::Screenshot(file) = cmd {
        return Step::Wait(Wait::Shot(Shot {
            file: file.clone(),
            started: Instant::now(),
            requested: false,
            frames: 0,
            drawn_since: None,
        }));
    }
    let Some(index) = state.active else {
        return Step::Fail("no map is open (use `open <game path>` first)".to_owned());
    };
    match cmd {
        Cmd::Open(_) | Cmd::Wait(_) => Step::Next,
        Cmd::SelectBox {
            layer,
            x0,
            y0,
            x1,
            y1,
        } => {
            let doc = &mut state.docs[index];
            let rect = CellRect::new((*x0, *y0), (*x1, *y1));
            doc.selection
                .box_select(&doc.ds1, rect, &[*layer], false, SelectOp::Replace);
            Step::Next
        }
        Cmd::MoveLayer { from, to, mode } => {
            let ok = state.exec_upgrading(
                index,
                EditOp::MoveToLayer {
                    from: *from,
                    to: *to,
                    mode: *mode,
                },
            );
            done_or_fail(state, ok, "move-layer")
        }
        Cmd::Paint {
            layer,
            x,
            y,
            cell,
            orientation,
        } => {
            let value = CellValue {
                cell: *cell,
                orientation: *orientation,
            };
            let ok = state.exec_upgrading(
                index,
                EditOp::Paint {
                    layer: *layer,
                    cells: vec![(*x, *y, value)],
                },
            );
            done_or_fail(state, ok, "paint")
        }
        Cmd::Copy => {
            if state.docs[index].selection.is_empty() {
                return Step::Fail("copy: nothing is selected".to_owned());
            }
            state.copy_selection();
            Step::Next
        }
        Cmd::Paste { x, y, mode } => {
            let Some(clip) = state.edit.clipboard.clone() else {
                return Step::Fail("paste: nothing was copied".to_owned());
            };
            let ok = state.exec_upgrading(
                index,
                EditOp::Paste {
                    clip: Box::new(clip),
                    at: (*x, *y),
                    mode: *mode,
                },
            );
            done_or_fail(state, ok, "paste")
        }
        Cmd::Undo => {
            if !state.docs[index].edit.can_undo() {
                return Step::Fail("undo: nothing to undo".to_owned());
            }
            state.undo();
            Step::Next
        }
        Cmd::Redo => {
            if !state.docs[index].edit.can_redo() {
                return Step::Fail("redo: nothing to redo".to_owned());
            }
            state.redo();
            Step::Next
        }
        Cmd::Save => match state.save_doc(index) {
            crate::edit::actions::SaveStatus::Saved => Step::Next,
            crate::edit::actions::SaveStatus::NeedsUpgrade => {
                let detail = state
                    .edit
                    .save_problem
                    .take()
                    .map(|p| format!("{} (needs v{})", p.detail, p.min_version))
                    .unwrap_or_default();
                Step::Fail(format!("save: the map cannot be written: {detail}"))
            }
            crate::edit::actions::SaveStatus::Failed => {
                Step::Fail(format!("save failed{}", last_log(state)))
            }
        },
        Cmd::AssertDirty(expected) => {
            let actual = state.docs[index].is_dirty();
            if actual == *expected {
                Step::Next
            } else {
                Step::Fail(format!(
                    "assert-dirty {expected}: the map is {}",
                    if actual { "dirty" } else { "clean" }
                ))
            }
        }
        Cmd::AssertSelection(expected) => {
            let actual = state.docs[index].selection.len();
            if actual == *expected {
                Step::Next
            } else {
                Step::Fail(format!(
                    "assert-selection {expected}: {actual} cell(s) are selected"
                ))
            }
        }
        Cmd::Screenshot(file) => Step::Wait(Wait::Shot(Shot {
            file: file.clone(),
            started: Instant::now(),
            requested: false,
            frames: 0,
            drawn_since: None,
        })),
        Cmd::PreviewPaste { x, y, mode } => {
            let Some(clip) = state.edit.clipboard.clone() else {
                return Step::Fail("preview-paste: nothing was copied".to_owned());
            };
            state.edit.paste = Some(PasteSession::new(clip, (*x, *y), *mode, false));
            state.edit.paste_cache = None;
            Step::Next
        }
        Cmd::MoveDialog { from, to, mode } => {
            state.edit.move_dialog = Some(MoveDialog {
                from: *from,
                to: *to,
                mode: *mode,
            });
            Step::Next
        }
        Cmd::SelectObject(i) => {
            let doc = &mut state.docs[index];
            if *i >= doc.ds1.objects.len() {
                return Step::Fail(format!(
                    "select-object {i}: the map has {} objects",
                    doc.ds1.objects.len()
                ));
            }
            doc.object_sel = vec![*i];
            state.objects.path.follow_object(Some(*i));
            Step::Next
        }
        // Handled by `objects_step`.
        Cmd::Animate(_)
        | Cmd::ObjectFrameTick(_)
        | Cmd::PlaceObject { .. }
        | Cmd::AssertObjects(_)
        | Cmd::PathAdd { .. }
        | Cmd::PathMove { .. }
        | Cmd::PathAction { .. }
        | Cmd::PathDelete(_)
        | Cmd::AssertPath { .. }
        | Cmd::ObjectsMode(_)
        | Cmd::PickerSearch(_)
        | Cmd::PickerChoose { .. }
        | Cmd::ImportIni(_)
        | Cmd::CenterCell { .. } => Step::Next,
        // Handled by `sprite_step`.
        Cmd::SpritePreview
        | Cmd::Mode(_)
        | Cmd::SpriteOpen(_)
        | Cmd::SpriteDir(_)
        | Cmd::SpriteFrame(_)
        | Cmd::SpritePlay(_)
        | Cmd::SpriteImport { .. }
        | Cmd::SpriteSaveGame(_)
        | Cmd::SpriteExportFrames(_)
        | Cmd::SpriteGifOpen
        | Cmd::SpriteGifSet(_)
        | Cmd::SpriteGifExport(_)
        | Cmd::SpriteEdit
        | Cmd::SpriteTab(_)
        | Cmd::PaintPencil { .. }
        | Cmd::PaintGradient { .. }
        | Cmd::PaintOutline { .. }
        | Cmd::PaintSelect { .. }
        | Cmd::PaintOnion(_)
        | Cmd::PaintUndo => Step::Next,
        // Handled by `browser_step`.
        Cmd::Search(_)
        | Cmd::AssertSearchTop { .. }
        | Cmd::BrushFrom(_)
        | Cmd::AssertBrush { .. }
        | Cmd::AssertRecent(_)
        | Cmd::FavoriteAdd { .. }
        | Cmd::Alias { .. }
        | Cmd::CtrlK(_)
        | Cmd::CtrlKClose
        | Cmd::BrowserScope(_)
        | Cmd::ThumbSize(_)
        | Cmd::TileMenu(_)
        | Cmd::TilesTab => Step::Next,
        // Handled by `history_step`.
        Cmd::AutosaveNow
        | Cmd::Checkpoint(_)
        | Cmd::AssertHistory { .. }
        | Cmd::HistorySelect(_)
        | Cmd::AssertDiff { .. }
        | Cmd::ShowSnapshot(_)
        | Cmd::HistoryJump(_)
        | Cmd::Restore(_)
        | Cmd::Milestone(_)
        | Cmd::AssertMilestones(_)
        | Cmd::Prune(_)
        | Cmd::AssertChanged(_)
        | Cmd::Setting { .. }
        | Cmd::HistoryTab
        | Cmd::Settings(_)
        | Cmd::AddInstall(_)
        | Cmd::CloseDialogs => Step::Next,
        // Handled by `shell_step`.
        Cmd::Click(_)
        | Cmd::ClickRibbonAll
        | Cmd::Press(_)
        | Cmd::PressAllKeys
        | Cmd::Theme(_)
        | Cmd::RibbonTab(_) => Step::Next,
        // Handled by `splash_step`.
        Cmd::Splash(_) | Cmd::SplashWaitClosed | Cmd::SplashSave { .. } => Step::Next,
        // Handled by `tool_step`.
        Cmd::Tool(_) => Step::Next,
        // Handled by `palette_step` and `ui::studio::script`.
        Cmd::Palette(_) | Cmd::Studio(_) => Step::Next,
        Cmd::PaintBrush { x, y } => {
            let Some(layer) = state.edit.current_layer else {
                return Step::Fail("paint-brush: there is no brush (use brush-from)".to_owned());
            };
            let value = state.edit.current;
            let ok = state.exec_upgrading(
                index,
                EditOp::Paint {
                    layer,
                    cells: vec![(*x, *y, value)],
                },
            );
            if ok {
                state.note_paint(index, layer, value);
            }
            done_or_fail(state, ok, "paint-brush")
        }
        Cmd::Zoom(divisor) => {
            let doc = &mut state.docs[index];
            if !doc.fitted {
                return Step::Retry; // the view is fitted by the first UI pass
            }
            match ZOOM_STEPS.iter().position(|d| d == divisor) {
                Some(zoom) => {
                    doc.view.zoom = zoom;
                    Step::Next
                }
                None => Step::Fail(format!("zoom: {divisor} is not a zoom step")),
            }
        }
    }
}

/// The editor, opened from the viewer's sprite when there is none.
fn script_editor(state: &mut AppState) -> Result<&mut EditorState, String> {
    if state.sprites.editor.is_none() {
        state.edit_viewer_sprite(None)?;
    }
    state
        .sprites
        .editor
        .as_mut()
        .ok_or_else(|| "no sprite is open in the editor".to_owned())
}

/// Where sprite saves and exports come from: the editor when it is open.
fn script_target(state: &AppState) -> Target {
    if state.sprites.editor.is_some() {
        Target::Editor
    } else {
        Target::Viewer
    }
}

/// Runs a Sprites-mode command; `None` for every other command.
fn sprite_step(state: &mut AppState, cmd: &Cmd) -> Option<Step> {
    let result: Result<(), String> = match cmd {
        Cmd::Mode(sprites) => {
            state.config.mode = if *sprites {
                AppMode::Sprites
            } else {
                AppMode::Map
            };
            Ok(())
        }
        Cmd::SpritePreview => {
            state.sprites.show_preview_panel = true;
            Ok(())
        }
        Cmd::SpriteOpen(path) => state.open_sprite(&GamePath::new(path)),
        Cmd::SpriteDir(d) => {
            let dirs = state.sprites.viewer.directions();
            if *d >= dirs {
                Err(format!("sprite-dir {d}: the sprite has {dirs} directions"))
            } else {
                state.sprites.viewer.set_dir(*d);
                if let Some(e) = state.sprites.editor.as_mut() {
                    let frame = e.cur.frame;
                    e.go_to(*d, frame);
                }
                Ok(())
            }
        }
        Cmd::SpriteFrame(f) => {
            let frames = state.sprites.viewer.frames();
            if *f >= frames {
                Err(format!("sprite-frame {f}: the sprite has {frames} frames"))
            } else {
                state.sprites.viewer.playing = false;
                state.sprites.viewer.set_frame(*f);
                if let Some(e) = state.sprites.editor.as_mut() {
                    let dir = e.cur.dir;
                    e.go_to(dir, *f);
                }
                Ok(())
            }
        }
        Cmd::SpritePlay(on) => {
            state.sprites.viewer.playing = *on;
            Ok(())
        }
        Cmd::SpriteImport {
            files,
            dirs,
            frames,
            dcc,
        } => {
            let s = &mut state.sprites.converter.import;
            s.layout = Layout::Frames;
            s.directions = (*dirs).max(1);
            s.frames = *frames;
            s.format = if *dcc {
                SpriteFormat::Dcc
            } else {
                SpriteFormat::Dc6
            };
            s.files = match files.as_slice() {
                // A folder: its images in name order (sprite.json gives offsets).
                [dir] if dir.is_dir() => list_images(dir),
                many => many.to_vec(),
            };
            // The import replaces what the editor showed.
            state.sprites.editor = None;
            state.sprite_import()
        }
        Cmd::SpriteSaveGame(path) => {
            let target = script_target(state);
            state.sprite_save_game(target, path).map(|_| ())
        }
        Cmd::SpriteExportFrames(dir) => {
            let target = script_target(state);
            state
                .sprite_export(Purpose::ExportFrames, dir, target)
                .map(|_| ())
        }
        Cmd::SpriteGifOpen => state.open_gif_dialog(state.sprites.editor.is_some()),
        Cmd::SpriteGifSet(pairs) => (|| {
            let d = state
                .sprites
                .gif
                .as_mut()
                .ok_or("sprite-gif-set: open the window with sprite-gif-open")?;
            let (frames, dirs) = (d.frame_count, d.settings.ticked.len());
            for p in pairs {
                let (k, v) = p
                    .split_once('=')
                    .ok_or_else(|| format!("sprite-gif-set: {p:?} is not key=value"))?;
                crate::sprites::gif_export::apply_setting(&mut d.settings, k, v, frames, dirs)?;
            }
            d.range_text = crate::sprites::gif_export::range_label(&d.settings);
            state.sync_gif_sequence();
            Ok(())
        })(),
        Cmd::SpriteGifExport(file) => state.gif_export(file).map(|_| ()),
        Cmd::SpriteEdit => {
            let r = state.edit_viewer_sprite(None);
            if state.sprites.pending_editor.is_some() {
                state.answer_pending_editor(true);
            }
            r
        }
        Cmd::SpriteTab(t) => {
            state.sprites.focus = Some(match t.as_str() {
                "editor" => Focus::Editor,
                "converter" => Focus::Converter,
                _ => Focus::Viewer,
            });
            Ok(())
        }
        Cmd::PaintPencil { x, y, index } => script_editor(state).and_then(|e| {
            e.tool = PaintTool::Pencil;
            e.set_primary(*index);
            let p = (*x as f32 + 0.5, *y as f32 + 0.5);
            e.press(p, false, Mods::default())?;
            e.release(p)
        }),
        Cmd::PaintGradient {
            from,
            to,
            colors,
            dither,
            radial,
        } => script_editor(state).and_then(|e| {
            e.gradient.dither = *dither;
            e.gradient.shape = if *radial {
                GradientShape::Radial
            } else {
                GradientShape::Linear
            };
            match colors {
                GradientSpec::Ramp(i) => {
                    e.gradient.mode = GradientMode::Ramp;
                    e.set_primary(*i);
                }
                GradientSpec::Two(a, b) => {
                    e.gradient.mode = GradientMode::Two;
                    e.set_primary(*a);
                    e.set_secondary(*b);
                }
            }
            e.apply_gradient(*from, *to)
        }),
        Cmd::PaintOutline {
            color,
            inside,
            thickness,
        } => script_editor(state).and_then(|e| {
            e.outline.side = if *inside {
                OutlineSide::Inside
            } else {
                OutlineSide::Outside
            };
            e.outline.thickness = *thickness;
            e.outline.auto = color.is_none();
            e.apply_outline(*color)
        }),
        Cmd::PaintSelect { x0, y0, x1, y1 } => script_editor(state).and_then(|e| {
            e.tool = PaintTool::SelectRect;
            e.press((*x0 as f32, *y0 as f32), false, Mods::default())?;
            e.release((*x1 as f32, *y1 as f32))
        }),
        Cmd::PaintOnion(on) => script_editor(state).map(|e| e.onion = *on),
        Cmd::PaintUndo => script_editor(state).and_then(|e| {
            if e.undo() {
                Ok(())
            } else {
                Err("nothing to undo".to_owned())
            }
        }),
        _ => return None,
    };
    Some(match result {
        Ok(()) => Step::Next,
        Err(e) => Step::Fail(e),
    })
}

/// `tool <verb> ...`: the tool windows' actions (see `ui::tools::script`).
fn tool_step(state: &mut AppState, cmd: &Cmd) -> Option<Step> {
    use crate::ui::tools::script::Outcome;
    let Cmd::Tool(line) = cmd else { return None };
    Some(match crate::ui::tools::script::run(state, line) {
        Ok(Outcome::Done) => Step::Next,
        Ok(Outcome::Retry) => Step::Retry,
        Err(e) => Step::Fail(format!("tool {line}: {e}")),
    })
}

fn palette_choice(label: &str) -> Result<crate::sprites::palette::PaletteChoice, String> {
    crate::sprites::palette::PaletteChoice::from_label(label)
        .ok_or_else(|| format!("{label:?} is not a palette"))
}

fn convert_setting(state: &mut AppState, key: &str, value: &[String]) -> Result<(), String> {
    use crate::sprites::convert_palette::settings::{Allowed, Distance, save_preset};
    let one = || -> Result<&str, String> {
        match value {
            [v] => Ok(v.as_str()),
            _ => Err(format!("convert-setting {key}: expected one value")),
        }
    };
    let f = || -> Result<f32, String> {
        one()?
            .parse::<f32>()
            .map_err(|_| format!("convert-setting {key}: not a number"))
    };
    match key {
        "distance" => {
            state.sprites.convert.settings.distance =
                Distance::parse(one()?).ok_or("distance: oklab, redmean or rgb")?;
        }
        "targets" => {
            state.sprites.convert.settings.allowed = match value {
                [a] if a.eq_ignore_ascii_case("all") => Allowed::All,
                [a] if a.eq_ignore_ascii_case("shared") => Allowed::Shared,
                [a, from, to] if a.eq_ignore_ascii_case("range") => Allowed::Range {
                    from: from.parse().map_err(|_| "range: from 1..255")?,
                    to: to.parse().map_err(|_| "range: to 1..255")?,
                },
                _ => return Err("targets: all, shared or range <from> <to>".to_owned()),
            };
        }
        "keep-exact" => {
            state.sprites.convert.settings.keep_exact = matches!(
                one()?.to_ascii_lowercase().as_str(),
                "on" | "true" | "yes" | "1"
            );
        }
        "lightness" => state.sprites.convert.settings.lightness_weight = f()?,
        "brightness" => state.sprites.convert.settings.brightness = f()?,
        "contrast" => state.sprites.convert.settings.contrast = f()?,
        "saturation" => state.sprites.convert.settings.saturation = f()?,
        "hue" => state.sprites.convert.settings.hue = f()?,
        "reset" => state.sprites.convert.settings.reset_keep_overrides(),
        "preset-save" => {
            let name = value.join(" ");
            let settings = state.sprites.convert.settings.clone();
            if !save_preset(&mut state.config.palette_presets, &name, &settings) {
                return Err("preset-save: expected a name".to_owned());
            }
            state.config_dirty = true;
        }
        "preset-load" => state.convert_load_preset(&value.join(" "))?,
        other => return Err(format!("convert-setting: unknown key {other:?}")),
    }
    let s = std::mem::take(&mut state.sprites.convert.settings);
    state.sprites.convert.settings = s.clamped();
    Ok(())
}

/// Runs a "Convert to palette" / palette memory command; `None` for every
/// other command.
fn palette_step(state: &mut AppState, cmd: &Cmd) -> Option<Step> {
    use crate::edit::script_palette::PaletteCmd as P;
    use crate::sprites::convert_palette::TargetChoice;
    let Cmd::Palette(p) = cmd else { return None };
    let result: Result<(), String> = match p {
        P::Open(t) => {
            if t.eq_ignore_ascii_case("viewer") {
                state.convert_open_viewer()
            } else if t.to_ascii_lowercase().starts_with("data\\") {
                state.convert_open_game(&GamePath::new(t))
            } else {
                state.convert_open_file(std::path::Path::new(t))
            }
        }
        P::Source(l) => palette_choice(l).map(|c| state.convert_set_source(c)),
        P::Target(l) => TargetChoice::parse(l)
            .ok_or_else(|| format!("{l:?} is not a target"))
            .map(|t| state.sprites.convert.target = t),
        P::Setting { key, value } => convert_setting(state, key, value),
        P::Override { source, target } => {
            match source {
                None => state.sprites.convert.settings.clear_overrides(),
                Some(s) => state.sprites.convert.settings.set_override(*s, *target),
            }
            Ok(())
        }
        P::Item(i) => {
            let n = state.sprites.convert.item_count();
            if *i < n {
                state.sprites.convert.set_item(*i);
                Ok(())
            } else {
                Err(format!("convert-item {i}: the input has {n} items"))
            }
        }
        P::Tile { o, m, s, n } => {
            match state
                .sprites
                .convert
                .input
                .as_ref()
                .and_then(|i| i.find_tile(*o, *m, *s, *n))
            {
                Some(i) => {
                    state.sprites.convert.set_item(i);
                    Ok(())
                }
                None => Err(format!("convert-tile {o} {m} {s}: no such tile")),
            }
        }
        P::Picker(x) => {
            state.sprites.convert.picker = *x;
            Ok(())
        }
        P::Hover(x) => {
            state.sprites.convert.script_hover = *x;
            Ok(())
        }
        P::Highlight(h) => {
            let c = &mut state.sprites.convert;
            c.highlight_changed = h == "changed";
            c.highlight_worst = h == "worst";
            Ok(())
        }
        P::Apply => state.convert_apply(),
        P::SaveGame(path) => {
            let text = path.clone().unwrap_or_else(|| state.converted_save_text());
            state.convert_save_game(&text).map(|_| ())
        }
        P::SaveAs(f) => state.convert_save_as(f).map(|_| ()),
        P::Close => {
            state.sprites.convert.open = false;
            state.sprites.convert.picker = None;
            Ok(())
        }
        P::BatchAddGame(f) => match state.convert_batch_add_game(f) {
            0 => Err(format!("convert-batch-add-game {f:?}: no files match")),
            _ => Ok(()),
        },
        P::BatchAddFolder(d) => {
            let items = crate::sprites::convert_palette::batch::collect_folder(d, true);
            match state.sprites.convert.batch.add(items) {
                0 => Err(format!(
                    "convert-batch-add-folder: nothing in {}",
                    d.display()
                )),
                _ => Ok(()),
            }
        }
        P::BatchOutput(o) => {
            let b = &mut state.sprites.convert.batch;
            b.to_saves = o.is_none();
            b.out_dir.clone_from(o);
            Ok(())
        }
        P::BatchStart => {
            if state.convert_batch_running() {
                return Some(Step::Retry);
            }
            if std::mem::take(&mut state.sprites.convert.batch.script_started) {
                return Some(match &state.sprites.convert.batch.outcome {
                    Some((false, text)) if text.contains(" 0 failed") => Step::Next,
                    Some((_, text)) => Step::Fail(format!("convert-batch-start: {text}")),
                    None => Step::Fail("convert-batch-start: no outcome".to_owned()),
                });
            }
            match state.convert_batch_start() {
                Ok(()) => {
                    state.sprites.convert.batch.script_started = true;
                    return Some(Step::Retry);
                }
                Err(e) => Err(e),
            }
        }
        P::Pick(l) => palette_choice(l).map(|c| {
            let key = crate::ui::sprites::panel_key(state, Focus::Viewer);
            state.pick_palette(c, key.as_deref());
        }),
        P::AssertBadge {
            label,
            source,
            panel,
        } => {
            let panel = match panel.as_str() {
                "editor" => Focus::Editor,
                "converter" => Focus::Converter,
                _ => Focus::Viewer,
            };
            let key = crate::ui::sprites::panel_key(state, panel);
            let badge = state.palette_badge(key.as_deref());
            let want = palette_choice(label);
            match want {
                Ok(w) if w == badge.current && badge.source.word() == source => Ok(()),
                Ok(_) => Err(format!(
                    "assert-palette-badge: the badge says {:?} ({}), expected {label} {source}",
                    badge.text,
                    badge.source.word()
                )),
                Err(e) => Err(e),
            }
        }
        P::SaveChecked(path) => {
            let target = script_target(state);
            state.sprite_write_checked(target, crate::sprites::DccWrite::Game(path.clone()));
            Ok(())
        }
        P::Warning(a) => {
            if state.sprites.palette_warning.is_none() {
                Err("palette-warning: no palette question is open".to_owned())
            } else {
                state.answer_palette_warning(match a.as_str() {
                    "use" => crate::sprites::PaletteAnswer::UseRemembered,
                    "keep" => crate::sprites::PaletteAnswer::KeepCurrent,
                    _ => crate::sprites::PaletteAnswer::Cancel,
                });
                Ok(())
            }
        }
    };
    Some(match result {
        Ok(()) => Step::Next,
        Err(e) => Step::Fail(e),
    })
}

/// Where the asset index stands for a browser command: `Ok(true)` when it
/// can be searched, `Ok(false)` while it is being checked or built.
fn index_ready(state: &AppState) -> Result<bool, String> {
    if state.index_disabled {
        return Err("the asset index is off (CAPY_NO_INDEX)".to_owned());
    }
    match &state.index.state {
        IndexState::Ready(_) => Ok(true),
        IndexState::Idle | IndexState::Checking | IndexState::Building(_) => Ok(false),
        IndexState::Cancelled => Err("the asset index build was cancelled".to_owned()),
        IndexState::Failed(e) => Err(format!("the asset index failed: {e}")),
    }
}

/// `assert-search-top`: `key` is among the first `n` results.
fn check_search_top(state: &AppState, n: usize, key: &str) -> Result<(), String> {
    let hits = &state.browser.results.hits;
    let wanted = key.to_ascii_lowercase();
    match hits.iter().position(|h| h.asset.key == wanted) {
        Some(i) if i < n => Ok(()),
        found => {
            let top: Vec<String> = hits
                .iter()
                .take(n)
                .map(|h| format!("{} ({:.2})", h.asset.key, h.score))
                .collect();
            let place = found.map_or_else(|| "not one".to_owned(), |i| format!("number {}", i + 1));
            Err(format!(
                "assert-search-top {n}: {key} is {place} of {} results for {:?}; the top is:\n  {}",
                hits.len(),
                state.browser.query,
                top.join("\n  ")
            ))
        }
    }
}

/// Runs an asset-browser command; `None` for every other command.
fn browser_step(state: &mut AppState, cmd: &Cmd) -> Option<Step> {
    let needs_index = matches!(
        cmd,
        Cmd::Search(_)
            | Cmd::AssertSearchTop { .. }
            | Cmd::BrushFrom(_)
            | Cmd::CtrlK(_)
            | Cmd::TileMenu(_)
    );
    if needs_index {
        match index_ready(state) {
            Ok(true) => {}
            Ok(false) => return Some(Step::Retry),
            Err(e) => return Some(Step::Fail(e)),
        }
    }
    let result: Result<(), String> = match cmd {
        Cmd::Search(query) => {
            state.browser.set_query(query);
            state.browser.focus_tiles = true;
            Ok(())
        }
        Cmd::AssertSearchTop { n, key } => match state.browser_search_status() {
            SearchStatus::Pending => return Some(Step::Retry),
            SearchStatus::Failed(e) => Err(format!("assert-search-top: {e}")),
            SearchStatus::Done => check_search_top(state, *n, key),
        },
        Cmd::BrushFrom(key) => {
            let lower = key.to_ascii_lowercase();
            match state.browser.asset(&lower) {
                None => return Some(Step::Retry), // being looked up
                Some(None) => Err(format!("brush-from: {key} is not in the index")),
                Some(Some(_)) => state.set_brush_from_key(&lower),
            }
        }
        Cmd::AssertBrush {
            layer,
            cell,
            orientation,
        } => {
            let (have_layer, have) = (state.edit.current_layer, state.edit.current);
            let orientation_ok = orientation.is_none_or(|o| o == have.orientation);
            if have_layer == Some(*layer) && have.cell == *cell && orientation_ok {
                Ok(())
            } else {
                Err(format!(
                    "assert-brush: the brush is {} cell {:#010x} orientation {}",
                    have_layer.map_or_else(|| "nothing".to_owned(), crate::doc::layer_label),
                    have.cell,
                    have.orientation
                ))
            }
        }
        Cmd::AssertRecent(key) => {
            let wanted = key.to_ascii_lowercase();
            state.recent_keys().and_then(|keys| {
                if keys.contains(&wanted) {
                    Ok(())
                } else {
                    Err(format!(
                        "assert-recent: {key} is not among the {} recent key(s): {keys:?}",
                        keys.len()
                    ))
                }
            })
        }
        Cmd::FavoriteAdd { group, key } => state
            .favorite_add_named(group, &key.to_ascii_lowercase())
            .map_err(|e| format!("favorite-add: {e}")),
        Cmd::Alias { key, text } => state
            .set_alias(&key.to_ascii_lowercase(), text)
            .map_err(|e| format!("alias: {e}")),
        Cmd::CtrlK(query) => {
            state.open_palette_with(query);
            Ok(())
        }
        Cmd::CtrlKClose => {
            state.browser.palette.open = false;
            Ok(())
        }
        Cmd::BrowserScope(scope) => {
            if *scope == Scope::MapTileSet && state.map_dt1s().is_empty() {
                Err("browser-scope map: no open map with a known tile set".to_owned())
            } else {
                state.browser.scope = *scope;
                state.browser.focus_tiles = true;
                Ok(())
            }
        }
        Cmd::ThumbSize(size) => {
            state.config.browser_thumb = size.clamp(THUMB_MIN, THUMB_MAX);
            state.browser.focus_tiles = true;
            Ok(())
        }
        Cmd::TileMenu(key) => {
            let lower = key.to_ascii_lowercase();
            state.browser.selected = Some(lower.clone());
            state.open_tile_menu(&lower, egui::pos2(TILE_MENU_AT.0, TILE_MENU_AT.1));
            Ok(())
        }
        Cmd::TilesTab => {
            state.browser.focus_tiles = true;
            Ok(())
        }
        _ => return None,
    };
    Some(match result {
        Ok(()) => Step::Next,
        Err(e) => Step::Fail(e),
    })
}

/// The active map's timeline entry named `key` (index from the newest, or a
/// checkpoint name / label).
fn find_snapshot(state: &mut AppState, index: usize, key: &str) -> Result<SnapshotId, String> {
    let timeline = state.timeline_of(index);
    history::find_in_timeline(&timeline, key).ok_or_else(|| {
        let names: Vec<String> = timeline
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "{i}: {}{}",
                    history::kind_slug(&s.kind),
                    history::display_name(s)
                        .map(|n| format!(" {n:?}"))
                        .unwrap_or_default()
                )
            })
            .collect();
        format!("no snapshot {key:?} in the timeline [{}]", names.join(", "))
    })
}

/// Runs a History / Settings command; `None` for every other command.
fn history_step(
    state: &mut AppState,
    cmd: &Cmd,
    awaiting_autosave: &mut Option<u64>,
) -> Option<Step> {
    let needs_map = matches!(
        cmd,
        Cmd::Checkpoint(_)
            | Cmd::AssertHistory { .. }
            | Cmd::HistorySelect(_)
            | Cmd::AssertDiff { .. }
            | Cmd::ShowSnapshot(_)
            | Cmd::HistoryJump(_)
            | Cmd::Restore(_)
    );
    let index = state.active;
    if needs_map && index.is_none() {
        return Some(Step::Fail(
            "no map is open (use `open <game path>` first)".to_owned(),
        ));
    }
    let index = index.unwrap_or(0);
    let result: Result<(), String> = match cmd {
        Cmd::AutosaveNow => {
            if state.history.autosave_in_flight {
                return Some(Step::Retry);
            }
            match *awaiting_autosave {
                Some(run) if state.history.autosave_runs > run => {
                    *awaiting_autosave = None;
                    if state.history.autosave_errors.is_empty() {
                        Ok(())
                    } else {
                        Err(format!(
                            "autosave-now: {}",
                            state.history.autosave_errors.join("; ")
                        ))
                    }
                }
                Some(_) => return Some(Step::Retry),
                None => {
                    let run = state.history.autosave_runs;
                    if state.autosave_now() == 0 {
                        state.info("autosave-now: no map has unrecorded changes");
                        Ok(())
                    } else {
                        *awaiting_autosave = Some(run);
                        return Some(Step::Retry);
                    }
                }
            }
        }
        Cmd::Checkpoint(name) => state.checkpoint(index, name).map(|s| {
            state.info(format!(
                "Checkpoint {name:?} recorded (snapshot {})",
                s.id.0
            ));
        }),
        Cmd::AssertHistory { n, kind } => {
            let timeline = state.timeline_of(index);
            let actual = timeline
                .iter()
                .filter(|s| {
                    kind.as_deref()
                        .is_none_or(|k| history::kind_slug(&s.kind) == k)
                })
                .count();
            if actual == *n {
                Ok(())
            } else {
                let all: Vec<&str> = timeline
                    .iter()
                    .map(|s| history::kind_slug(&s.kind))
                    .collect();
                Err(format!(
                    "assert-history {n}{}: found {actual} (timeline, newest first: {all:?})",
                    kind.as_deref().map(|k| format!(" {k}")).unwrap_or_default()
                ))
            }
        }
        Cmd::HistorySelect(key) => {
            find_snapshot(state, index, key).and_then(|id| state.select_snapshot(id))
        }
        Cmd::AssertDiff { layer, count } => {
            state.history_frame();
            let doc_id = state.docs[index].id;
            match state.history.overlay_for(doc_id) {
                None => Err("assert-diff: no snapshot is selected (history-select)".to_owned()),
                Some(o) if o.count(*layer) == *count => Ok(()),
                Some(o) => Err(format!(
                    "assert-diff {} {count}: {} cell(s) differ ({:?})",
                    crate::doc::layer_label(*layer),
                    o.count(*layer),
                    o.per_layer.get(layer)
                )),
            }
        }
        Cmd::ShowSnapshot(on) => state.set_show_snapshot(*on),
        Cmd::HistoryJump(layer) => {
            state.history_frame();
            let doc_id = state.docs[index].id;
            let n = state.history.jump_cursor;
            match state
                .history
                .overlay_for(doc_id)
                .map(|o| o.nth_of(*layer, n))
            {
                None => Err("history-jump: no snapshot is selected".to_owned()),
                Some(None) => Err(format!(
                    "history-jump: no change on {}",
                    crate::doc::layer_label(*layer)
                )),
                Some(Some((x, y))) => {
                    state.history.jump_cursor = n + 1;
                    state.jump_to_cell(x, y);
                    Ok(())
                }
            }
        }
        Cmd::Restore(key) => find_snapshot(state, index, key).and_then(|id| {
            let profile = state.docs[index].profile_id.clone();
            let expected = history::read_snapshot(&state.paths.history, &profile, id)?;
            state.restore_snapshot(id)?;
            let now = state.docs[index]
                .edit
                .ds1()
                .to_bytes()
                .map_err(|e| format!("restore: cannot serialise the map: {e}"))?;
            if now != expected {
                return Err("restore: the map does not equal the snapshot".to_owned());
            }
            if !state.docs[index].is_dirty() {
                return Err("restore: the map should be dirty".to_owned());
            }
            match state.timeline_of(index).first() {
                Some(s) if s.kind == SnapshotKind::BeforeRestore => Ok(()),
                other => Err(format!(
                    "restore: the newest snapshot is {:?}, not BeforeRestore",
                    other.map(|s| history::kind_slug(&s.kind))
                )),
            }
        }),
        Cmd::Milestone(message) => state.milestone(message).map(|id| match id {
            Some(id) => state.info(format!("Milestone {id} committed")),
            None => state.info("Milestone: nothing changed"),
        }),
        Cmd::AssertMilestones(n) => {
            state.refresh_milestones();
            match state.milestones().clone() {
                Ok(list) if list.len() == *n => Ok(()),
                Ok(list) => Err(format!("assert-milestones {n}: there are {}", list.len())),
                Err(e) => Err(format!("assert-milestones: {e}")),
            }
        }
        Cmd::Prune(keep) => {
            let mut policy = state.config.retention();
            policy.keep_last = *keep;
            state.prune_now(&policy).map(|r| {
                state.info(format!(
                    "Pruned {} snapshot(s), {} blob(s), {} bytes",
                    r.snapshots_removed, r.blobs_removed, r.bytes_freed
                ));
            })
        }
        Cmd::AssertChanged(n) => {
            let actual = state
                .active_profile_id()
                .map_or(0, |p| crate::edit::save::changed_count(&state.paths, p));
            if actual == *n {
                Ok(())
            } else {
                Err(format!("assert-changed {n}: CHANGED.txt lists {actual}"))
            }
        }
        Cmd::Setting { key, value } => state.apply_setting(key, value),
        Cmd::HistoryTab => {
            state.history.focus = true;
            Ok(())
        }
        Cmd::Settings(open) => {
            state.settings.open = *open;
            Ok(())
        }
        Cmd::AddInstall(dir) => {
            state.open_add_install(dir.as_deref());
            Ok(())
        }
        Cmd::CloseDialogs => {
            crate::ui::shell::close_dialogs(state);
            Ok(())
        }
        _ => return None,
    };
    Some(match result {
        Ok(()) => Step::Next,
        Err(e) => Step::Fail(e),
    })
}

/// The one selected object of the active map, for the path commands.
fn script_object(state: &AppState, what: &str) -> Result<(usize, usize), String> {
    state
        .single_object()
        .ok_or_else(|| format!("{what}: select exactly one object first (select-object <index>)"))
}

/// Runs an object / NPC path command; `None` for every other command.
fn objects_step(state: &mut AppState, cmd: &Cmd) -> Option<Step> {
    let needs_map = !matches!(
        cmd,
        Cmd::Animate(_) | Cmd::ObjectFrameTick(_) | Cmd::ImportIni(_)
    );
    let is_object_cmd = matches!(
        cmd,
        Cmd::Animate(_)
            | Cmd::ObjectFrameTick(_)
            | Cmd::PlaceObject { .. }
            | Cmd::AssertObjects(_)
            | Cmd::PathAdd { .. }
            | Cmd::PathMove { .. }
            | Cmd::PathAction { .. }
            | Cmd::PathDelete(_)
            | Cmd::AssertPath { .. }
            | Cmd::ObjectsMode(_)
            | Cmd::PickerSearch(_)
            | Cmd::PickerChoose { .. }
            | Cmd::ImportIni(_)
            | Cmd::CenterCell { .. }
    );
    if !is_object_cmd {
        return None;
    }
    if needs_map && state.active.is_none() {
        return Some(Step::Fail(
            "no map is open (use `open <game path>` first)".to_owned(),
        ));
    }
    let index = state.active.unwrap_or(0);
    let exec = |state: &mut AppState, op: EditOp, what: &str| -> Result<(), String> {
        if state.exec_upgrading(index, op) {
            Ok(())
        } else {
            Err(format!("{what} failed{}", last_log(state)))
        }
    };
    let result: Result<(), String> = match cmd {
        Cmd::Animate(on) => {
            state.config.animate_objects = *on;
            state.config_dirty = true;
            Ok(())
        }
        Cmd::ObjectFrameTick(tick) => {
            state.objects.frame_tick = *tick;
            Ok(())
        }
        Cmd::PlaceObject { kind, id, x, y } => {
            let before = state.docs[index].ds1.objects.len();
            exec(
                state,
                EditOp::AddObject(d2_formats::ds1::Object {
                    kind: *kind,
                    id: *id,
                    x: *x,
                    y: *y,
                    flags: 0,
                }),
                "place-object",
            )
            .and_then(|()| {
                let doc = &state.docs[index];
                if doc.ds1.objects.len() == before + 1 && doc.object_sel == vec![before] {
                    Ok(())
                } else {
                    Err("place-object: the new object is not selected".to_owned())
                }
            })
        }
        Cmd::AssertObjects(n) => {
            let actual = state.docs[index].ds1.objects.len();
            if actual == *n {
                Ok(())
            } else {
                Err(format!("assert-objects {n}: the map has {actual}"))
            }
        }
        Cmd::PathAdd { x, y, action } => {
            script_object(state, "path-add").and_then(|(_, object)| {
                exec(
                    state,
                    EditOp::AddPathPoint {
                        object,
                        point: d2_formats::ds1::PathPoint {
                            x: *x,
                            y: *y,
                            action: *action,
                        },
                        at: None,
                    },
                    "path-add",
                )
            })
        }
        Cmd::PathMove { index: i, x, y } => {
            script_object(state, "path-move").and_then(|(_, object)| {
                exec(
                    state,
                    EditOp::MovePathPoint {
                        object,
                        index: *i,
                        x: *x,
                        y: *y,
                    },
                    "path-move",
                )
            })
        }
        Cmd::PathAction { index: i, action } => {
            script_object(state, "path-action").and_then(|(_, object)| {
                exec(
                    state,
                    EditOp::SetPathAction {
                        object,
                        index: *i,
                        action: *action,
                    },
                    "path-action",
                )
            })
        }
        Cmd::PathDelete(i) => script_object(state, "path-delete").and_then(|(_, object)| {
            exec(
                state,
                EditOp::DeletePathPoints {
                    object,
                    indices: vec![*i],
                },
                "path-delete",
            )
        }),
        Cmd::AssertPath { object, points } => {
            let actual = state.docs[index]
                .edit
                .path_of(*object)
                .map_or(0, |p| p.points.len());
            if actual == *points {
                Ok(())
            } else {
                Err(format!(
                    "assert-path {object} {points}: the object's path has {actual} point(s)"
                ))
            }
        }
        Cmd::ObjectsMode(mode) => {
            crate::ui::cancel_map_gesture(state);
            state.edit.tool = crate::edit::tools::Tool::Objects;
            state.objects.mode = *mode;
            state.objects.focus = true;
            Ok(())
        }
        Cmd::PickerSearch(text) => {
            state.objects.picker.query = text.clone();
            state.objects.focus = true;
            Ok(())
        }
        Cmd::PickerChoose { kind, id } => {
            let act = state.docs[index].ds1.effective_act();
            match state.placeables(act) {
                // The object tables are still loading.
                None => return Some(Step::Retry),
                Some(list) if list.iter().any(|i| (i.ds1_kind, i.ds1_id) == (*kind, *id)) => {
                    state.objects.picker.chosen = Some((*kind, *id));
                    state.objects.focus = true;
                    Ok(())
                }
                Some(_) => Err(format!(
                    "picker-choose {kind} {id}: not placeable in act {}",
                    act + 1
                )),
            }
        }
        Cmd::CenterCell { x, y } => {
            if !state.docs[index].fitted {
                return Some(Step::Retry); // the first UI pass fits the view
            }
            state.jump_to_cell(*x, *y);
            Ok(())
        }
        Cmd::ImportIni(file) => {
            if matches!(state.catalog, crate::state::CatalogState::Building) {
                return Some(Step::Retry);
            }
            state
                .import_ini_files(std::slice::from_ref(file))
                .and_then(|r| {
                    if r.applied == 0 {
                        Err(format!("import-ini: no mapping applied ({})", r.summary()))
                    } else {
                        Ok(())
                    }
                })
        }
        _ => return None,
    };
    Some(match result {
        Ok(()) => Step::Next,
        Err(e) => Step::Fail(e),
    })
}

/// The images directly inside `dir`, in natural name order.
fn list_images(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|x| x.to_str())
                            .and_then(capy_sprite::ImageKind::from_extension)
                            .is_some()
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| crate::sprites::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    out
}

/// A scripted click or key press waiting to be seen (frames since the
/// events went out).
pub struct Probe {
    /// Commands still to press (`press-all-keys`).
    queue: VecDeque<&'static str>,
    current: Option<(Expect, usize, u32)>,
    pressed: Vec<&'static str>,
    skipped: Vec<&'static str>,
}

/// What a probe waits for: a ribbon click or a command run from a key.
#[derive(Clone, Copy)]
enum Expect {
    Click(&'static str),
    Ran(Option<&'static str>),
}

fn key_events(combo: crate::ui::Combo) -> Vec<Vec<egui::Event>> {
    let event = |pressed| egui::Event::Key {
        key: combo.key,
        physical_key: Some(combo.key),
        pressed,
        repeat: false,
        modifiers: combo.modifiers(),
    };
    vec![vec![event(true)], vec![event(false)]]
}

/// Waits for the probe's events to be delivered and handled. `Some` when
/// it is decided: `Ok` when the expected click or command happened.
fn settle_probe(state: &mut AppState, probe: &mut Probe) -> Option<Result<(), String>> {
    let (expect, before, frames) = probe.current?;
    if !state.shell.inject.is_empty() || frames < 2 {
        probe.current = Some((expect, before, frames + 1));
        return None;
    }
    probe.current = None;
    Some(match expect {
        Expect::Click(id) => {
            let seen = &state.shell.clicked[before.min(state.shell.clicked.len())..];
            if seen.contains(&id) {
                Ok(())
            } else {
                Err(format!("click {id}: the button did not react"))
            }
        }
        Expect::Ran(None) => Ok(()),
        Expect::Ran(Some(id)) => {
            let seen = &state.shell.ran[before.min(state.shell.ran.len())..];
            if seen.contains(&id) {
                Ok(())
            } else {
                Err(format!("{id}: its key did not run it"))
            }
        }
    })
}

fn new_probe(queue: VecDeque<&'static str>) -> Probe {
    Probe {
        queue,
        current: None,
        pressed: Vec::new(),
        skipped: Vec::new(),
    }
}

/// The Map Editor shell's commands: real clicks and key presses, themes.
fn shell_step(state: &mut AppState, cmd: &Cmd) -> Option<Step> {
    let result = match cmd {
        Cmd::Theme(id) => {
            let known = crate::ui::theme::builtin_themes()
                .iter()
                .any(|t| &t.id == id)
                || state.config.ui.custom_themes.iter().any(|t| &t.id == id);
            if known {
                state.config.ui.theme = id.clone();
                state.config_dirty = true;
                return Some(Step::Wait(Wait::Frames(3)));
            }
            Err(format!("theme {id:?}: no such theme"))
        }
        Cmd::RibbonTab(tab) => {
            if crate::ui::shell::RIBBON_TABS.contains(&tab.as_str()) {
                state.config.ui.ribbon_tab = tab.clone();
                return Some(Step::Wait(Wait::Frames(2)));
            }
            Err(format!(
                "ribbon-tab {tab:?}: home, view, map, game, diag or help"
            ))
        }
        Cmd::Click(id) => return Some(click_step(state, id)),
        Cmd::Press(text) => return Some(press_step(state, text)),
        Cmd::ClickRibbonAll => {
            if state.shell.click_all.is_none() {
                state.shell.click_all = Some(crate::ui::shell::new_click_all());
            }
            return Some(match crate::ui::shell::click_all_step(state) {
                Ok(true) => {
                    state.shell.click_all = None;
                    Step::Next
                }
                Ok(false) => Step::Retry,
                Err(e) => {
                    state.shell.click_all = None;
                    Step::Fail(format!("click-ribbon-all: {e}"))
                }
            });
        }
        Cmd::PressAllKeys => return Some(press_all_step(state)),
        _ => return None,
    };
    Some(match result {
        Ok(()) => Step::Next,
        Err(e) => Step::Fail(e),
    })
}

fn click_step(state: &mut AppState, id: &str) -> Step {
    let mut probe = match state.shell.probe.take() {
        Some(p) => p,
        None => {
            let found = state
                .shell
                .ribbon_rects
                .iter()
                .find(|(i, _, _)| *i == id)
                .map(|(i, r, e)| (*i, *r, *e));
            match found {
                Some((id, rect, true)) => {
                    let before = state.shell.clicked.len();
                    state
                        .shell
                        .inject
                        .extend(crate::ui::shell::click_events(rect.center()));
                    let mut p = new_probe(VecDeque::new());
                    p.current = Some((Expect::Click(id), before, 0));
                    p
                }
                Some(_) => return Step::Fail(format!("click {id}: the button is disabled")),
                None => return Step::Fail(format!("click {id}: no such button on screen")),
            }
        }
    };
    match settle_probe(state, &mut probe) {
        None => {
            state.shell.probe = Some(probe);
            Step::Retry
        }
        Some(Ok(())) => Step::Next,
        Some(Err(e)) => Step::Fail(e),
    }
}

/// Sends `combo`; the probe expects the command it is bound to (if that is
/// enabled now) to run.
fn send_key(state: &mut AppState, combo: crate::ui::Combo) -> Probe {
    let id = state
        .shell
        .bindings
        .command_for(combo)
        .filter(|id| crate::ui::command_enabled(state, id))
        // The command finder (and key capture) take the keyboard first.
        .filter(|_| !state.shell.blocks_keys());
    let before = state.shell.ran.len();
    state.shell.inject.extend(key_events(combo));
    let mut p = new_probe(VecDeque::new());
    p.current = Some((Expect::Ran(id), before, 0));
    p
}

fn press_step(state: &mut AppState, text: &str) -> Step {
    let mut probe = match state.shell.probe.take() {
        Some(p) => p,
        None => match crate::ui::Combo::parse(text) {
            Some(combo) => send_key(state, combo),
            None => return Step::Fail(format!("press {text:?}: not a key")),
        },
    };
    match settle_probe(state, &mut probe) {
        None => {
            state.shell.probe = Some(probe);
            Step::Retry
        }
        Some(Ok(())) => Step::Next,
        Some(Err(e)) => Step::Fail(format!("press {text}: {e}")),
    }
}

fn press_all_step(state: &mut AppState) -> Step {
    let mut probe = match state.shell.probe.take() {
        Some(p) => p,
        None => new_probe(
            crate::ui::COMMANDS
                .iter()
                .filter(|c| c.available && !c.external)
                .filter(|c| state.shell.bindings.get(c.id).is_some())
                .map(|c| c.id)
                .collect(),
        ),
    };
    loop {
        if probe.current.is_some() {
            match settle_probe(state, &mut probe) {
                None => {
                    state.shell.probe = Some(probe);
                    return Step::Retry;
                }
                Some(Err(e)) => return Step::Fail(format!("press-all-keys: {e}")),
                Some(Ok(())) => {
                    // Windows the key opened would hold the next keys.
                    state.shell.close_windows();
                    state.shell.close_popups = true;
                }
            }
        }
        let Some(id) = probe.queue.pop_front() else {
            state.info(format!(
                "press-all-keys: {} key(s) pressed; not enabled then: {}",
                probe.pressed.len(),
                probe.skipped.join(", ")
            ));
            return Step::Next;
        };
        let combo = state.shell.bindings.get(id);
        match combo {
            Some(combo) if crate::ui::command_enabled(state, id) => {
                let mut p = send_key(state, combo);
                if let Some((Expect::Ran(Some(_)), before, _)) = p.current {
                    p.current = Some((Expect::Ran(Some(id)), before, 0));
                }
                probe.current = p.current;
                probe.pressed.push(id);
            }
            _ => probe.skipped.push(id),
        }
    }
}

fn done_or_fail(state: &AppState, ok: bool, what: &str) -> Step {
    if ok {
        Step::Next
    } else {
        Step::Fail(format!("{what} failed{}", last_log(state)))
    }
}

enum ShotPoll {
    Waiting(Shot),
    Done,
    Failed(String),
}

fn poll_shot(commands: &mut Commands, state: &mut AppState, mut shot: Shot) -> ShotPoll {
    shot.frames += 1;
    if shot.requested {
        if shot.file.exists() {
            state.info(format!("Screenshot saved to {}", shot.file.display()));
            return ShotPoll::Done;
        }
        if shot.frames > GIVE_UP_FRAMES {
            return ShotPoll::Failed(format!(
                "script: the screenshot {} was never written",
                shot.file.display()
            ));
        }
        return ShotPoll::Waiting(shot);
    }
    // The Sprites mode draws through egui only: it is ready once settled.
    let drawn = state.config.mode == AppMode::Sprites
        || state.active_doc().is_none_or(|d| {
            d.fitted
                && (matches!(
                    d.render.state,
                    RenderState::Failed(_) | RenderState::ProfileClosed
                ) || (d.render.state == RenderState::Ready && state.objects_settled(d)))
        });
    if !drawn {
        shot.drawn_since = None;
    } else if shot.drawn_since.is_none() {
        shot.drawn_since = Some(Instant::now());
    }
    let settled = shot.started.elapsed().as_secs_f32() >= SETTLE_SECS
        && shot
            .drawn_since
            .is_some_and(|t| t.elapsed().as_secs_f32() >= SETTLE_SECS / 2.0)
        && shot.frames >= 30;
    if drawn && settled {
        if let Some(dir) = shot.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::remove_file(&shot.file);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(shot.file.clone()));
        shot.requested = true;
        shot.frames = 0;
    } else if shot.frames > GIVE_UP_FRAMES {
        return ShotPoll::Failed(
            "script: the map never finished drawing for the screenshot".to_owned(),
        );
    }
    ShotPoll::Waiting(shot)
}
