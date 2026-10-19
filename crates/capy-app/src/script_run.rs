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
use crate::edit::script::{Cmd, Line, parse_script};
use crate::edit::state::MoveDialog;
use crate::index_task::IndexState;
use crate::iso::ZOOM_STEPS;
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
}

impl ScriptRunner {
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
        let result = step(&mut state, &cmd);
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

fn step(state: &mut AppState, cmd: &Cmd) -> Step {
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
    if let Some(step) = browser_step(state, cmd) {
        return step;
    }
    // Screenshots also work without a map (Sprites mode).
    if let Cmd::Screenshot(file) = cmd {
        return Step::Wait(Wait::Shot(Shot {
            file: file.clone(),
            started: Instant::now(),
            requested: false,
            frames: 0,
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
            Step::Next
        }
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
                && matches!(
                    d.render.state,
                    RenderState::Ready | RenderState::Failed(_) | RenderState::ProfileClosed
                )
        });
    let settled = shot.started.elapsed().as_secs_f32() >= SETTLE_SECS && shot.frames >= 30;
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
