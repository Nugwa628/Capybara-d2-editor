//! Sprites mode (P6): the Viewer (DC6/DCC/COF), the Converter (import,
//! export, batch, "Save to game path") and the palette-locked Editor, plus
//! the map editor's Sprite preview box. Everything here is state and logic;
//! the egui drawing is in `crate::ui::sprites`.

pub mod cof;
pub mod convert;
pub mod dialog;
pub mod editor;
pub mod palette;
pub mod preview;
pub mod viewer;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use capy_sprite::{Sprite, SpriteFormat};
use d2_assets::{GamePath, Overlay};
use d2_formats::IndexedImage;
use d2_formats::animdata::AnimData;
use d2_formats::cof::Cof;
use d2_formats::palette::Palette;

use crate::state::AppState;
use convert::{BatchSettings, BatchTask, ImportSettings, PaletteFormat};
use dialog::{Dialogs, Purpose};
use editor::EditorState;
use palette::PaletteStore;
use preview::PreviewState;
use viewer::{Loaded, SingleSprite, ViewerState, fps_for_speed};

/// Where the sprite on show came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpriteSource {
    /// Read through the profile overlay.
    Game(GamePath),
    /// A loose file anywhere on disk (read-only).
    File(PathBuf),
    /// Made in the Converter.
    Imported(String),
}

impl SpriteSource {
    pub fn label(&self) -> String {
        match self {
            SpriteSource::Game(p) => p.as_str().to_owned(),
            SpriteSource::File(f) => f.display().to_string(),
            SpriteSource::Imported(what) => format!("(imported) {what}"),
        }
    }

    pub fn game_path(&self) -> Option<&GamePath> {
        match self {
            SpriteSource::Game(p) => Some(p),
            _ => None,
        }
    }

    /// A file name for exports (`zmtrlitnuhth`).
    pub fn stem(&self) -> String {
        let name = match self {
            SpriteSource::Game(p) => p.file_name().to_owned(),
            SpriteSource::File(f) => f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            SpriteSource::Imported(_) => "sprite".to_owned(),
        };
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
        if stem.is_empty() {
            "sprite".to_owned()
        } else {
            stem.to_owned()
        }
    }
}

/// Which sprite the Converter's exports and saves use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Viewer,
    Editor,
}

/// The game-path prompt of "Save to game path".
pub struct SavePrompt {
    pub target: Target,
    pub text: String,
    pub error: Option<String>,
}

/// A panel of the Sprites mode to bring to the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Viewer,
    Editor,
    Converter,
}

/// The sprites list of the open profile.
#[derive(Default)]
pub struct SpriteList {
    pub filter: String,
    pub show_dc6: bool,
    pub show_dcc: bool,
    pub show_cof: bool,
    /// Indices into `Session::sprites` that pass the filter.
    pub filtered: Vec<usize>,
    computed_for: Option<(String, [bool; 3], u64)>,
}

pub struct ConverterState {
    pub import: ImportSettings,
    pub batch: BatchSettings,
    pub batch_task: Option<BatchTask>,
    pub target: Target,
    /// Open an import in the editor too.
    pub edit_after_import: bool,
    /// How the last batch ended ("Done: ..." or "Cancelled ..."), and
    /// whether it was cancelled.
    pub batch_outcome: Option<(bool, String)>,
}

impl Default for ConverterState {
    fn default() -> Self {
        ConverterState {
            import: ImportSettings::default(),
            batch: BatchSettings::default(),
            batch_task: None,
            target: Target::Viewer,
            edit_after_import: false,
            batch_outcome: None,
        }
    }
}

pub struct SpritesState {
    pub list: SpriteList,
    pub palette: PaletteStore,
    pub viewer: ViewerState,
    pub editor: Option<EditorState>,
    pub converter: ConverterState,
    pub preview: PreviewState,
    pub dialogs: Dialogs,
    pub save_prompt: Option<SavePrompt>,
    /// An editor session waiting for "discard the unsaved one?".
    pub pending_editor: Option<Box<EditorState>>,
    pub quit_prompt: bool,
    pub quit_confirmed: bool,
    pub focus: Option<Focus>,
    /// Scripts ask for the map editor's Sprite preview panel.
    pub show_preview_panel: bool,
    /// The panel under the pointer (keyboard shortcuts go there).
    pub hot: Option<Focus>,
    /// Use the OS clipboard (off in scripted runs).
    pub use_os_clipboard: bool,
    animdata: Option<(u64, Option<Arc<AnimData>>)>,
}

impl Default for SpritesState {
    fn default() -> Self {
        SpritesState {
            list: SpriteList {
                show_dc6: true,
                show_dcc: true,
                show_cof: true,
                ..SpriteList::default()
            },
            palette: PaletteStore::default(),
            viewer: ViewerState::default(),
            editor: None,
            converter: ConverterState::default(),
            preview: PreviewState::default(),
            dialogs: Dialogs::default(),
            save_prompt: None,
            pending_editor: None,
            quit_prompt: false,
            quit_confirmed: false,
            focus: None,
            hot: None,
            show_preview_panel: false,
            use_os_clipboard: true,
            animdata: None,
        }
    }
}

impl SpritesState {
    pub fn editor_dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(EditorState::is_dirty)
    }

    /// True while a modal of the Sprites mode is open.
    pub fn modal_open(&self) -> bool {
        self.save_prompt.is_some() || self.pending_editor.is_some() || self.quit_prompt
    }
}

/// Names that are sprites (`.dc6`, `.dcc`, `.cof`), sorted.
pub fn sprite_list<'a>(names: impl Iterator<Item = &'a GamePath>) -> Vec<GamePath> {
    let mut out: Vec<GamePath> = names
        .filter(|p| {
            p.extension().is_some_and(|e| {
                ["dc6", "dcc", "cof"]
                    .iter()
                    .any(|x| e.eq_ignore_ascii_case(x))
            })
        })
        .cloned()
        .collect();
    out.sort();
    out
}

/// Every `pal.dat` among the names, sorted.
pub fn palette_list<'a>(names: impl Iterator<Item = &'a GamePath>) -> Vec<GamePath> {
    let mut out: Vec<GamePath> = names
        .filter(|p| p.file_name().eq_ignore_ascii_case("pal.dat"))
        .cloned()
        .collect();
    out.sort();
    out
}

/// Case-insensitive substring filter plus the type toggles.
pub fn filter_sprites(paths: &[GamePath], filter: &str, kinds: [bool; 3]) -> Vec<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    paths
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let kind_ok = match p.extension().map(str::to_ascii_lowercase).as_deref() {
                Some("dc6") => kinds[0],
                Some("dcc") => kinds[1],
                Some("cof") => kinds[2],
                _ => false,
            };
            kind_ok && (needle.is_empty() || p.as_str().contains(&needle))
        })
        .map(|(i, _)| i)
        .collect()
}

fn format_of_name(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "dc6" => Some("dc6"),
        "dcc" => Some("dcc"),
        "cof" => Some("cof"),
        _ => None,
    }
}

/// What the Converter works on, copied out of the viewer or the editor.
pub struct TargetData {
    pub sprite: Sprite,
    pub format: SpriteFormat,
    pub source: Option<SpriteSource>,
    pub dir: usize,
    pub frame: usize,
    pub fps: f32,
}

impl AppState {
    pub fn sprite_overlay(&self) -> Option<Arc<Overlay>> {
        self.session.as_ref().map(|s| s.overlay.clone())
    }

    /// The raw palette sprites are matched against.
    pub fn sprite_palette(&mut self) -> Palette {
        let overlay = self.sprite_overlay();
        let epoch = self.session_epoch;
        let (palette, ok) = self.sprites.palette.raw(overlay.as_deref(), epoch);
        if !ok && overlay.is_some() {
            let label = self.sprites.palette.choice.label();
            self.warn(format!(
                "Palette {label} cannot be read; using the grey ramp"
            ));
            self.sprites.palette.choice = palette::PaletteChoice::Grey;
        }
        palette
    }

    /// The palette sprites are drawn with (gamma applied).
    pub fn sprite_display_palette(&mut self) -> Palette {
        let _ = self.sprite_palette(); // falls back to grey on errors
        let overlay = self.sprite_overlay();
        self.sprites
            .palette
            .display(overlay.as_deref(), self.session_epoch)
    }

    fn animdata(&mut self) -> Option<Arc<AnimData>> {
        let epoch = self.session_epoch;
        if self.sprites.animdata.as_ref().map(|(e, _)| *e) != Some(epoch) {
            let parsed = self.sprite_overlay().and_then(|o| {
                let bytes = o
                    .read_bytes(&GamePath::new("data\\global\\animdata.d2"))
                    .ok()
                    .flatten()?;
                AnimData::parse(&bytes).ok().map(Arc::new)
            });
            self.sprites.animdata = Some((epoch, parsed));
        }
        self.sprites.animdata.as_ref().and_then(|(_, a)| a.clone())
    }

    /// Playback speed of a COF from AnimData.
    fn cof_fps(&mut self, key: &str) -> (f32, String) {
        match self.animdata().and_then(|a| a.get(key).map(|r| r.speed)) {
            Some(speed) => (fps_for_speed(speed), format!("AnimData speed {speed}")),
            None => (viewer::DEFAULT_FPS, "default (not in AnimData)".to_owned()),
        }
    }

    // ───────── opening ─────────

    /// Opens a sprite by game path through the profile overlay.
    pub fn open_sprite(&mut self, path: &GamePath) -> Result<(), String> {
        self.open_sprite_with(path, &[])
    }

    /// [`AppState::open_sprite`] with preferred armour per component code.
    pub fn open_sprite_with(
        &mut self,
        path: &GamePath,
        prefer: &[(String, String)],
    ) -> Result<(), String> {
        let result = (|| {
            let overlay = self.sprite_overlay().ok_or("no profile is open")?;
            let bytes = overlay
                .read_bytes(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?
                .ok_or_else(|| format!("{path} is not in the profile"))?;
            self.open_sprite_bytes(
                path.as_str(),
                &bytes,
                SpriteSource::Game(path.clone()),
                prefer,
            )
        })();
        self.report_open(result)
    }

    /// Opens a loose DC6/DCC/COF from disk, read-only.
    pub fn open_sprite_file(&mut self, file: &Path) -> Result<(), String> {
        let result = std::fs::read(file)
            .map_err(|e| format!("cannot read {}: {e}", file.display()))
            .and_then(|bytes| {
                self.open_sprite_bytes(
                    &file.to_string_lossy(),
                    &bytes,
                    SpriteSource::File(file.to_path_buf()),
                    &[],
                )
            });
        self.report_open(result)
    }

    fn report_open(&mut self, result: Result<(), String>) -> Result<(), String> {
        match &result {
            Ok(()) => {
                self.sprites.focus = Some(Focus::Viewer);
                let title = self.sprites.viewer.title();
                let detail = match self.sprites.viewer.cof() {
                    Some(c) => format!(
                        "COF {} directions x {} frames, {} layers ({} missing)",
                        c.directions(),
                        c.frames(),
                        c.layers.len(),
                        c.missing()
                    ),
                    None => format!(
                        "{} directions x {} frames",
                        self.sprites.viewer.directions(),
                        self.sprites.viewer.frames()
                    ),
                };
                self.info(format!("Sprite {title}: {detail}"));
            }
            Err(e) => {
                self.sprites.viewer.error = Some(e.clone());
                self.error(format!("Cannot open the sprite: {e}"));
            }
        }
        result
    }

    fn open_sprite_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        source: SpriteSource,
        prefer: &[(String, String)],
    ) -> Result<(), String> {
        match format_of_name(name) {
            Some("cof") => {
                let cof = Cof::parse(bytes).map_err(|e| format!("not a COF: {e}"))?;
                if cof.directions == 0 || cof.frames_per_direction == 0 {
                    return Err("the COF has no frames".to_owned());
                }
                let cof_name = cof::parse_cof_path(name)
                    .ok_or_else(|| format!("{name} is not named token+mode+class.cof"))?;
                let overlay = self.sprite_overlay();
                let names = self
                    .session
                    .as_ref()
                    .map(|s| s.sprites.clone())
                    .unwrap_or_default();
                let game = source.game_path().cloned();
                let view =
                    cof::CofView::load(cof, cof_name, game, overlay.as_deref(), &names, prefer);
                let (fps, note) = self.cof_fps(&view.name.anim_key());
                self.sprites
                    .viewer
                    .set_loaded(Loaded::Cof(Box::new(view)), source, fps, note);
                Ok(())
            }
            Some(ext) => {
                let format = SpriteFormat::from_extension(ext).ok_or("unknown format")?;
                let sprite = Sprite::decode(bytes, format).map_err(|e| e.to_string())?;
                if sprite.directions.is_empty() {
                    return Err("the sprite has no directions".to_owned());
                }
                self.sprites.viewer.set_loaded(
                    Loaded::Sprite(SingleSprite { sprite, format }),
                    source,
                    viewer::DEFAULT_FPS,
                    "default".to_owned(),
                );
                Ok(())
            }
            None => Err(format!("{name} is not a .dc6, .dcc or .cof file")),
        }
    }

    /// Switches the open COF to another (mode, class) of the same token.
    pub fn open_cof_variant(&mut self, mode: &str, class: &str) {
        let Some(c) = self.sprites.viewer.cof() else {
            return;
        };
        let mut name = c.name.clone();
        name.mode = mode.to_owned();
        name.class = class.to_owned();
        let path = name.game_path(c.kind);
        let prefer: Vec<(String, String)> = c
            .layers
            .iter()
            .map(|l| (l.code.to_owned(), l.armor.clone()))
            .collect();
        let (dir, frame) = (self.sprites.viewer.dir, self.sprites.viewer.frame);
        if self.open_sprite_with(&path, &prefer).is_ok() {
            self.sprites.viewer.set_dir(dir);
            self.sprites.viewer.set_frame(frame);
        }
    }

    pub fn set_cof_armor(&mut self, layer: usize, armor: &str) {
        let overlay = self.sprite_overlay();
        if let Some(c) = self.sprites.viewer.cof_mut() {
            c.set_armor(layer, armor, overlay.as_deref());
            if let Some(p) = c.layers.get(layer).and_then(|l| l.problem.clone()) {
                self.warn(p);
            }
        }
        self.sprites.viewer.bump();
    }

    pub fn set_cof_layer_visible(&mut self, layer: usize, visible: bool) {
        if let Some(c) = self.sprites.viewer.cof_mut() {
            c.set_visible(layer, visible);
        }
        self.sprites.viewer.bump();
    }

    // ───────── editor ─────────

    /// Opens the viewer's sprite (or one COF layer's sprite) in the editor.
    pub fn edit_viewer_sprite(&mut self, cof_layer: Option<usize>) -> Result<(), String> {
        let v = &self.sprites.viewer;
        let (sprite, format, source) = match (cof_layer, v.cof()) {
            (Some(i), Some(c)) => {
                let layer = c.layers.get(i).ok_or("no such layer")?;
                let sprite = layer.sprite.clone().ok_or("that layer has no sprite")?;
                let path = layer.path.clone().ok_or("that layer has no file")?;
                let format = convert::format_of(path.as_str()).unwrap_or(SpriteFormat::Dcc);
                (sprite, format, SpriteSource::Game(path))
            }
            _ => {
                let (sprite, format) = v
                    .sprite()
                    .ok_or("open a DC6 or DCC first (a COF is edited one layer at a time)")?;
                let source = v
                    .source
                    .clone()
                    .unwrap_or_else(|| SpriteSource::Imported("sprite".into()));
                (sprite.clone(), format, source)
            }
        };
        let (dir, frame) = (v.dir, v.frame);
        let palette = self.sprite_palette();
        let mut editor = EditorState::new(sprite, format, source, palette)?;
        editor.go_to(dir, frame);
        self.install_editor(editor);
        Ok(())
    }

    /// Makes `editor` the editor session, asking first when the current one
    /// has unsaved changes.
    pub fn install_editor(&mut self, editor: EditorState) {
        if self.sprites.editor_dirty() {
            self.sprites.pending_editor = Some(Box::new(editor));
            return;
        }
        let title = editor.source.label();
        self.sprites.editor = Some(editor);
        self.sprites.converter.target = Target::Editor;
        self.sprites.focus = Some(Focus::Editor);
        self.info(format!("Editing {title}"));
    }

    /// Answer to the "discard unsaved changes" prompt.
    pub fn answer_pending_editor(&mut self, discard: bool) {
        let Some(pending) = self.sprites.pending_editor.take() else {
            return;
        };
        if discard {
            self.sprites.editor = None;
            self.install_editor(*pending);
        }
    }

    /// Keeps the editor's palette in step with the palette picker.
    pub fn sync_editor_palette(&mut self) {
        if self.sprites.editor.is_none() {
            return;
        }
        let palette = self.sprite_palette();
        if let Some(e) = self.sprites.editor.as_mut() {
            e.set_palette(palette);
        }
    }

    // ───────── converter ─────────

    /// The sprite the Converter acts on.
    pub fn target_data(&self, target: Target) -> Option<TargetData> {
        match target {
            Target::Viewer => {
                let v = &self.sprites.viewer;
                let (sprite, format) = v.sprite()?;
                Some(TargetData {
                    sprite: sprite.clone(),
                    format,
                    source: v.source.clone(),
                    dir: v.dir,
                    frame: v.frame,
                    fps: v.fps,
                })
            }
            Target::Editor => {
                let e = self.sprites.editor.as_ref()?;
                Some(TargetData {
                    sprite: e.to_sprite(),
                    format: e.format,
                    source: Some(e.source.clone()),
                    dir: e.cur.dir,
                    frame: e.cur.frame,
                    fps: self.sprites.viewer.fps,
                })
            }
        }
    }

    /// Imports the Converter's files into a new sprite (Viewer, optionally
    /// the editor too).
    pub fn sprite_import(&mut self) -> Result<(), String> {
        let settings = self.sprites.converter.import.clone();
        let palette = self.sprite_palette();
        let result = convert::build_import(&settings, &palette);
        match result {
            Ok(sprite) => {
                let what = match settings.files.as_slice() {
                    [one] => one
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    many => format!("{} images", many.len()),
                };
                let format = settings.format;
                let (dirs, frames) = (sprite.directions.len(), sprite.frames_per_direction());
                let source = SpriteSource::Imported(what.clone());
                self.sprites.viewer.set_loaded(
                    Loaded::Sprite(SingleSprite {
                        sprite: sprite.clone(),
                        format,
                    }),
                    source.clone(),
                    viewer::DEFAULT_FPS,
                    "default".to_owned(),
                );
                self.sprites.focus = Some(Focus::Viewer);
                self.info(format!(
                    "Imported {what}: {dirs} directions x {frames} frames as {}",
                    format.extension().to_ascii_uppercase()
                ));
                if self.sprites.converter.edit_after_import {
                    let editor = EditorState::new(sprite, format, source, palette)?;
                    self.install_editor(editor);
                }
                Ok(())
            }
            Err(e) => {
                self.error(format!("Import failed: {e}"));
                Err(e)
            }
        }
    }

    /// Opens the game-path prompt, prefilled from the sprite's source.
    pub fn begin_sprite_save(&mut self, target: Target) {
        let Some(data) = self.target_data(target) else {
            self.error("Nothing to save: open or import a DC6/DCC first");
            return;
        };
        let text = match data.source.as_ref().and_then(SpriteSource::game_path) {
            Some(p) => p.as_str().to_owned(),
            None => format!(
                "data\\global\\{}.{}",
                data.source
                    .map_or_else(|| "sprite".to_owned(), |s| s.stem()),
                data.format.extension()
            ),
        };
        self.sprites.save_prompt = Some(SavePrompt {
            target,
            text,
            error: None,
        });
    }

    /// "Save to game path": `capybara\saves\<profile>\<game path>`, CHANGED.txt
    /// and a snapshot.
    pub fn sprite_save_game(&mut self, target: Target, text: &str) -> Result<PathBuf, String> {
        let result = (|| {
            let (path, format) = convert::check_game_path(text)?;
            let profile = self
                .session
                .as_ref()
                .map(|s| s.profile.id.clone())
                .ok_or("no profile is open")?;
            let data = self.target_data(target).ok_or("nothing to save")?;
            let palette = self.sprite_palette();
            let report =
                convert::save_to_game(&self.paths, &profile, &path, &data.sprite, &palette)?;
            Ok((path, format, report))
        })();
        match result {
            Ok((path, format, report)) => {
                for w in &report.warnings {
                    self.warn(format!("{path}: {w}"));
                }
                self.info(format!(
                    "Saved {path} ({} bytes, {}) to {}; CHANGED.txt lists {} file(s)",
                    report.bytes,
                    format.extension().to_ascii_uppercase(),
                    report.path.display(),
                    report.changed_total
                ));
                if target == Target::Editor
                    && let Some(e) = self.sprites.editor.as_mut()
                {
                    e.session.mark_saved();
                    e.source = SpriteSource::Game(path.clone());
                    e.format = format;
                }
                self.refresh_changed();
                self.start_saves_refresh();
                Ok(report.path)
            }
            Err(e) => {
                self.error(format!("Save failed: {e}"));
                Err(e)
            }
        }
    }

    /// Runs one "Export to..." with the path picked in the dialog.
    pub fn sprite_export(
        &mut self,
        purpose: Purpose,
        path: &Path,
        target: Target,
    ) -> Result<String, String> {
        let palette = self.sprite_palette();
        let palette_name = self.sprites.palette.choice.label();
        let result = match purpose {
            Purpose::ExportPaletteGpl => {
                convert::export_palette(&palette, &palette_name, PaletteFormat::Gpl, path)
            }
            Purpose::ExportPaletteAct => {
                convert::export_palette(&palette, &palette_name, PaletteFormat::Act, path)
            }
            Purpose::ExportPalettePal => {
                convert::export_palette(&palette, &palette_name, PaletteFormat::Jasc, path)
            }
            _ => match self.target_data(target) {
                None => Err("nothing to export: open or import a DC6/DCC first".to_owned()),
                Some(d) => {
                    let source = d.source.as_ref().map(SpriteSource::label);
                    let current: Option<&IndexedImage> =
                        d.sprite.directions.get(d.dir).and_then(|f| f.get(d.frame));
                    match purpose {
                        Purpose::ExportFrames => convert::export_frames(
                            &d.sprite,
                            d.format,
                            &palette,
                            &palette_name,
                            source.as_deref(),
                            path,
                        ),
                        Purpose::ExportSheet => convert::export_sheet(&d.sprite, &palette, path),
                        Purpose::ExportGif => match d.sprite.directions.get(d.dir) {
                            Some(frames) => convert::export_gif(frames, &palette, d.fps, path),
                            None => Err("no such direction".to_owned()),
                        },
                        Purpose::ExportPng => match current {
                            Some(img) => convert::export_indexed_png(img, &palette, path),
                            None => Err("no such frame".to_owned()),
                        },
                        Purpose::ExportSprite => {
                            let format =
                                convert::format_of(&path.to_string_lossy()).unwrap_or(d.format);
                            convert::export_sprite_file(&d.sprite, format, &palette, path)
                        }
                        _ => Err("not an export".to_owned()),
                    }
                }
            },
        };
        match &result {
            Ok(text) => self.info(text.clone()),
            Err(e) => self.error(format!("Export failed: {e}")),
        }
        result
    }

    /// Opens a file dialog; its answer is handled by [`AppState::poll_sprites`].
    pub fn sprite_dialog(&mut self, purpose: Purpose) {
        let target = self.sprites.converter.target;
        let stem = self
            .target_data(target)
            .and_then(|d| d.source)
            .map_or_else(|| "sprite".to_owned(), |s| s.stem());
        let name = match purpose {
            Purpose::ExportSheet => Some(format!("{stem}-sheet.png")),
            Purpose::ExportGif => Some(format!("{stem}.gif")),
            Purpose::ExportPng => Some(format!("{stem}-frame.png")),
            Purpose::ExportPaletteGpl => {
                Some(format!("{}.gpl", self.sprites.palette.choice.slug()))
            }
            Purpose::ExportPaletteAct => {
                Some(format!("{}.act", self.sprites.palette.choice.slug()))
            }
            Purpose::ExportPalettePal => {
                Some(format!("{}.pal", self.sprites.palette.choice.slug()))
            }
            Purpose::ExportSprite => {
                let ext = self
                    .target_data(target)
                    .map_or("dcc", |d| d.format.extension());
                Some(format!("{stem}.{ext}"))
            }
            _ => None,
        };
        self.sprites.dialogs.open(purpose, name);
    }

    pub fn start_batch(&mut self) {
        if self.sprites.converter.batch_task.is_some() {
            return;
        }
        let palette = self.sprite_palette();
        match BatchTask::spawn(&self.sprites.converter.batch, palette) {
            Ok(task) => {
                let s = &self.sprites.converter.batch;
                let text = format!(
                    "Batch started: {} -> {} ({})",
                    s.input
                        .as_deref()
                        .map_or_else(String::new, |p| p.display().to_string()),
                    s.output
                        .as_deref()
                        .map_or_else(String::new, |p| p.display().to_string()),
                    if s.to_sprite {
                        "images to sprites"
                    } else {
                        "sprites to PNG"
                    }
                );
                self.sprites.converter.batch_task = Some(task);
                self.sprites.converter.batch_outcome = None;
                self.info(text);
            }
            Err(e) => self.error(format!("Batch: {e}")),
        }
    }

    /// Dialog answers and batch progress; once per frame.
    pub fn poll_sprites(&mut self) {
        if let Some(answer) = self.sprites.dialogs.poll()
            && !answer.paths.is_empty()
        {
            self.handle_dialog(answer.purpose, answer.paths);
        }
        let finished = self
            .sprites
            .converter
            .batch_task
            .as_ref()
            .and_then(BatchTask::poll);
        if let Some(result) = finished {
            let task = self.sprites.converter.batch_task.take();
            let secs = task.map_or(0.0, |t| t.started.elapsed().as_secs_f32());
            match result {
                Ok(report) => {
                    for line in convert::report_lines(&report) {
                        self.info(line);
                    }
                    let outcome = convert::batch_outcome(&report, secs);
                    if report.cancelled {
                        self.warn(format!("{outcome}; files already written stay"));
                    } else {
                        self.info(outcome.clone());
                    }
                    self.sprites.converter.batch_outcome = Some((report.cancelled, outcome));
                    if !report.failed.is_empty() {
                        self.error(format!(
                            "Batch: {} file(s) failed (see the Log)",
                            report.failed.len()
                        ));
                    }
                }
                Err(e) => {
                    self.sprites.converter.batch_outcome = Some((false, format!("Failed: {e}")));
                    self.error(format!("Batch failed: {e}"));
                }
            }
        }
    }

    fn handle_dialog(&mut self, purpose: Purpose, paths: Vec<PathBuf>) {
        let target = self.sprites.converter.target;
        match purpose {
            Purpose::OpenSprite => {
                if let Some(p) = paths.first() {
                    let _ = self.open_sprite_file(p);
                }
            }
            Purpose::ImportImages => {
                let mut paths = paths;
                paths.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
                self.sprites.converter.import.files = paths;
            }
            Purpose::ImportFolder => {
                if let Some(dir) = paths.first() {
                    let palette = self.sprite_palette();
                    // With sprite.json the library turns colour keying off.
                    let opts = self.sprites.converter.import.matching.quantize();
                    match capy_sprite::import_frames(dir, &palette, &opts) {
                        Ok((sprite, meta)) => {
                            let format =
                                meta.map_or(self.sprites.converter.import.format, |m| m.format);
                            let what = dir.display().to_string();
                            self.sprites.viewer.set_loaded(
                                Loaded::Sprite(SingleSprite { sprite, format }),
                                SpriteSource::Imported(what.clone()),
                                viewer::DEFAULT_FPS,
                                "default".to_owned(),
                            );
                            self.sprites.focus = Some(Focus::Viewer);
                            self.info(format!("Imported the frames in {what}"));
                        }
                        Err(e) => self.error(format!("Import failed: {e}")),
                    }
                }
            }
            Purpose::BatchInput => self.sprites.converter.batch.input = paths.into_iter().next(),
            Purpose::BatchOutput => self.sprites.converter.batch.output = paths.into_iter().next(),
            export => {
                if let Some(p) = paths.first() {
                    let _ = self.sprite_export(export, p, target);
                }
            }
        }
    }
}

/// Natural order for file names (`f2` before `f10`).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| -> Vec<(u8, u64, String)> {
        let mut out = Vec::new();
        let mut digits = String::new();
        let mut text = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                if !text.is_empty() {
                    out.push((1, 0, std::mem::take(&mut text)));
                }
                digits.push(c);
            } else {
                if !digits.is_empty() {
                    out.push((
                        0,
                        digits.parse().unwrap_or(u64::MAX),
                        std::mem::take(&mut digits),
                    ));
                }
                text.push(c.to_ascii_lowercase());
            }
        }
        if !digits.is_empty() {
            out.push((0, digits.parse().unwrap_or(u64::MAX), digits));
        }
        if !text.is_empty() {
            out.push((1, 0, text));
        }
        out
    };
    key(a).cmp(&key(b))
}

impl AppState {
    /// Recomputes the filtered sprite list when the filter or session changed.
    pub fn refresh_sprite_filter(&mut self) {
        let l = &self.sprites.list;
        let kinds = [l.show_dc6, l.show_dcc, l.show_cof];
        let key = (l.filter.clone(), kinds, self.session_epoch);
        if l.computed_for.as_ref() == Some(&key) {
            return;
        }
        let filtered = match &self.session {
            Some(s) => filter_sprites(&s.sprites, &key.0, kinds),
            None => Vec::new(),
        };
        self.sprites.list.filtered = filtered;
        self.sprites.list.computed_for = Some(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_and_palette_lists_filter_the_names() {
        let names: Vec<GamePath> = [
            "data\\global\\monsters\\zm\\cof\\zmnuhth.cof",
            "data\\global\\items\\invfoo.DC6",
            "data\\global\\tiles\\x.ds1",
            "data\\global\\monsters\\zm\\tr\\zmtrlitnuhth.dcc",
            "data\\global\\palette\\units\\pal.dat",
            "data\\global\\palette\\act1\\pal.pl2",
        ]
        .iter()
        .map(|s| GamePath::new(s))
        .collect();
        let sprites = sprite_list(names.iter());
        assert_eq!(sprites.len(), 3);
        assert_eq!(palette_list(names.iter()).len(), 1);
        assert_eq!(filter_sprites(&sprites, "ZM", [true; 3]).len(), 2);
        assert_eq!(filter_sprites(&sprites, "", [true, false, false]).len(), 1);
        assert_eq!(
            filter_sprites(&sprites, "zm", [false, false, true]).len(),
            1
        );
    }

    #[test]
    fn natural_order_compares_numbers() {
        let mut v = vec!["f10.png", "f2.png", "F1.png"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["F1.png", "f2.png", "f10.png"]);
    }

    #[test]
    fn sources_name_exports() {
        assert_eq!(
            SpriteSource::Game(GamePath::new("data\\a\\zmtrlitnuhth.dcc")).stem(),
            "zmtrlitnuhth"
        );
        assert_eq!(SpriteSource::Imported("x".into()).stem(), "sprite");
        assert!(
            SpriteSource::File(PathBuf::from("C:\\a.dc6"))
                .game_path()
                .is_none()
        );
    }

    fn state() -> (tempfile::TempDir, AppState) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = crate::paths::Paths::at(tmp.path().join("capybara"));
        (
            tmp,
            AppState::new(paths, crate::config::Config::default(), Vec::new()),
        )
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        let (tmp, mut s) = state();
        assert!(
            s.open_sprite(&GamePath::new("data\\x.dcc")).is_err(),
            "no profile"
        );
        let junk = tmp.path().join("junk.dcc");
        std::fs::write(&junk, b"not a dcc").expect("write");
        assert!(s.open_sprite_file(&junk).is_err());
        let cof = tmp.path().join("zmnuhth.cof");
        std::fs::write(&cof, [1u8, 2, 3]).expect("write");
        assert!(s.open_sprite_file(&cof).is_err());
        assert!(s.open_sprite_file(&tmp.path().join("missing.dc6")).is_err());
        assert!(s.sprites.viewer.error.is_some());
        assert!(s.edit_viewer_sprite(None).is_err(), "nothing to edit");
        assert!(s.sprite_save_game(Target::Viewer, "data\\a.dcc").is_err());
    }

    #[test]
    fn a_loose_sprite_opens_edits_and_guards_unsaved_work() {
        let (tmp, mut s) = state();
        let mut img = IndexedImage::new(4, 4, -2, -4);
        img.pixels[5] = 30;
        img.alpha[5] = 255;
        let sprite = Sprite {
            directions: vec![vec![img.clone(), img]],
        };
        let file = tmp.path().join("thing.dc6");
        std::fs::write(&file, sprite.encode(SpriteFormat::Dc6).expect("enc")).expect("write");
        s.open_sprite_file(&file).expect("open");
        assert_eq!(s.sprites.viewer.frames(), 2);
        s.edit_viewer_sprite(None).expect("edit");
        assert!(s.sprites.editor.is_some());
        let e = s.sprites.editor.as_mut().expect("editor");
        e.press((0.0, 0.0), false, editor::Mods::default())
            .expect("paint");
        e.release((0.0, 0.0)).expect("paint");
        assert!(s.sprites.editor_dirty());
        // Opening another sprite in the editor asks first.
        s.edit_viewer_sprite(None).expect("edit again");
        assert!(s.sprites.pending_editor.is_some());
        s.answer_pending_editor(false);
        assert!(s.sprites.editor_dirty(), "kept the edited one");
        s.edit_viewer_sprite(None).expect("edit again");
        s.answer_pending_editor(true);
        assert!(!s.sprites.editor_dirty(), "replaced");
        // Exports of the editor sprite go where asked.
        let out = tmp.path().join("frames");
        s.sprite_export(Purpose::ExportFrames, &out, Target::Editor)
            .expect("export");
        assert!(out.join("sprite.json").is_file());
    }
}
