//! "Convert to palette" (P7): carries the colours of a DC6, DCC or DT1 from
//! one palette into another. The window shows the plan (every used colour,
//! its match and error), lets the user tune it and compare before/after,
//! and changes data only on "Convert". Nothing here runs automatically.
//!
//! The result is an unsaved converted file: "Save to game path" (saves,
//! CHANGED.txt, snapshot) or "Save As...". DT1 and DC6 are patched in place
//! by `d2_formats::remap` (only pixel bytes change), DCC is re-encoded.
//! The UI is in `crate::ui::sprites::convert_palette`.

pub mod batch;
pub mod settings;
pub mod table;

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use capy_paint::remap::{RemapPlan, candidates, plan as make_plan};
use capy_sprite::{Sprite, SpriteFormat};
use d2_assets::GamePath;
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use super::dt1view::{Dt1View, TileInfo};
use super::palette::PaletteChoice;
use super::palette_memory::{guess_from_path, suggests_all_acts};
use super::viewer::{Loaded, SingleSprite, Tex};
use super::{Focus, SpriteSource};
use crate::state::AppState;
use d2_formats::remap::{SpriteKind, index_histogram, remap_indices};
use settings::ConvertSettings;
use table::Summary;

/// What the colours are converted into.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TargetChoice {
    Palette(PaletteChoice),
    /// Act 1's colours without the act-specific indices 225..=254: the
    /// result looks the same under every act palette.
    AllActs,
}

impl TargetChoice {
    pub fn label(&self) -> String {
        match self {
            TargetChoice::Palette(c) => c.label(),
            TargetChoice::AllActs => "All acts (shared colours only)".to_owned(),
        }
    }

    /// The palette whose colours are matched.
    pub fn palette(&self) -> PaletteChoice {
        match self {
            TargetChoice::Palette(c) => c.clone(),
            TargetChoice::AllActs => PaletteChoice::Act(0),
        }
    }

    pub fn shared_only(&self) -> bool {
        *self == TargetChoice::AllActs
    }

    /// `all-acts` / `shared`, or a palette label.
    pub fn parse(text: &str) -> Option<TargetChoice> {
        match text.trim().to_ascii_lowercase().as_str() {
            "all-acts" | "allacts" | "all" | "shared" => Some(TargetChoice::AllActs),
            other => PaletteChoice::from_label(other).map(TargetChoice::Palette),
        }
    }
}

/// Where the converted file came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputOrigin {
    Game(GamePath),
    File(PathBuf),
    /// A sprite with no file (an import), with the game path it may be
    /// saved to.
    Unsaved {
        label: String,
        game: Option<GamePath>,
    },
}

impl InputOrigin {
    pub fn label(&self) -> String {
        match self {
            InputOrigin::Game(p) => p.as_str().to_owned(),
            InputOrigin::File(f) => f.display().to_string(),
            InputOrigin::Unsaved { label, .. } => label.clone(),
        }
    }

    pub fn game_path(&self) -> Option<&GamePath> {
        match self {
            InputOrigin::Game(p) | InputOrigin::Unsaved { game: Some(p), .. } => Some(p),
            _ => None,
        }
    }

    /// The palette-memory key.
    pub fn memory_key(&self) -> Option<String> {
        match self {
            InputOrigin::Game(p) => Some(p.as_str().to_owned()),
            InputOrigin::File(f) => Some(f.display().to_string()),
            InputOrigin::Unsaved { game, .. } => game.as_ref().map(|g| g.as_str().to_owned()),
        }
    }

    fn file_name(&self) -> String {
        match self {
            InputOrigin::Game(p) => p.file_name().to_owned(),
            InputOrigin::File(f) => f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            InputOrigin::Unsaved { game: Some(g), .. } => g.file_name().to_owned(),
            InputOrigin::Unsaved { .. } => "converted".to_owned(),
        }
    }
}

pub fn kind_label(kind: SpriteKind) -> &'static str {
    match kind {
        SpriteKind::Dt1 => "DT1",
        SpriteKind::Dc6 => "DC6",
        SpriteKind::Dcc => "DCC",
    }
}

fn kind_ext(kind: SpriteKind) -> &'static str {
    match kind {
        SpriteKind::Dt1 => "dt1",
        SpriteKind::Dc6 => "dc6",
        SpriteKind::Dcc => "dcc",
    }
}

fn sprite_format(kind: SpriteKind) -> Option<SpriteFormat> {
    match kind {
        SpriteKind::Dc6 => Some(SpriteFormat::Dc6),
        SpriteKind::Dcc => Some(SpriteFormat::Dcc),
        SpriteKind::Dt1 => None,
    }
}

/// The file being converted, decoded for the preview.
pub struct ConvertInput {
    pub kind: SpriteKind,
    pub bytes: Arc<Vec<u8>>,
    pub origin: InputOrigin,
    pub histogram: [u64; 256],
    /// Frames (direction by direction) or tiles.
    pub items: Vec<IndexedImage>,
    /// DT1 tile keys (empty for sprites).
    pub tiles: Vec<TileInfo>,
    pub directions: usize,
    pub frames: usize,
}

impl ConvertInput {
    pub fn load(kind: SpriteKind, bytes: Vec<u8>, origin: InputOrigin) -> Result<Self, String> {
        let (items, tiles, directions, frames) = match sprite_format(kind) {
            None => {
                let view = Dt1View::load(&bytes)?;
                let n = view.len();
                (view.images, view.tiles, 1, n)
            }
            Some(format) => {
                let sprite = Sprite::decode(&bytes, format).map_err(|e| e.to_string())?;
                let (d, f) = (sprite.directions.len(), sprite.frames_per_direction());
                if d == 0 || f == 0 {
                    return Err("the sprite has no frames".to_owned());
                }
                let items = sprite.directions.into_iter().flatten().collect();
                (items, Vec::new(), d, f)
            }
        };
        let histogram = index_histogram(kind, &bytes).map_err(|e| e.to_string())?;
        Ok(ConvertInput {
            kind,
            bytes: Arc::new(bytes),
            origin,
            histogram,
            items,
            tiles,
            directions,
            frames,
        })
    }

    /// "dir 2 frame 5" or the tile key.
    pub fn item_label(&self, i: usize) -> String {
        match self.tiles.get(i) {
            Some(t) => format!(
                "tile {i}: orientation {} main {} sub {} (#{})",
                t.orientation, t.main, t.sub, t.n
            ),
            None => format!(
                "dir {} frame {}",
                i / self.frames.max(1),
                i % self.frames.max(1)
            ),
        }
    }

    pub fn find_tile(&self, o: i32, m: i32, s: i32, n: Option<u32>) -> Option<usize> {
        self.tiles
            .iter()
            .position(|t| (t.orientation, t.main, t.sub) == (o, m, s) && n.is_none_or(|n| t.n == n))
    }
}

/// The converted file, not saved yet (or saved to `saved`).
pub struct Converted {
    pub kind: SpriteKind,
    pub bytes: Arc<Vec<u8>>,
    pub origin: InputOrigin,
    pub target: TargetChoice,
    pub summary: Summary,
    pub saved: Option<String>,
}

impl Converted {
    pub fn label(&self) -> String {
        format!(
            "(converted to {}) {}",
            self.target.label(),
            self.origin.label()
        )
    }
}

/// The before/after textures of the item on show.
pub struct PreviewTex {
    pub key: u64,
    pub before: Option<Tex>,
    pub after: Option<Tex>,
}

pub struct PaletteConvert {
    pub open: bool,
    pub input: Option<ConvertInput>,
    pub source: PaletteChoice,
    pub target: TargetChoice,
    pub settings: ConvertSettings,
    /// Frame or tile on show.
    pub item: usize,
    pub zoom: u32,
    /// Before and after side by side; else one image and a toggle.
    pub side_by_side: bool,
    pub show_after: bool,
    pub highlight_changed: bool,
    pub highlight_worst: bool,
    /// The plan row under the pointer: its pixels are highlighted.
    pub hovered: Option<u8>,
    /// The override picker is open for this source index.
    pub picker: Option<u8>,
    /// A row highlighted by a script (`convert-hover`), as if hovered.
    pub script_hover: Option<u8>,
    pub result: Option<Converted>,
    pub batch: batch::BatchUi,
    /// The "Pick a DT1" list's filter and whether it is shown.
    pub dt1_filter: String,
    pub dt1_list: bool,
    pub preset_name: String,
    pub error: Option<String>,
    plan_cache: Option<(u64, Arc<RemapPlan>)>,
    pub tex: Option<PreviewTex>,
}

impl Default for PaletteConvert {
    fn default() -> Self {
        PaletteConvert {
            open: false,
            input: None,
            source: PaletteChoice::Act(4),
            target: TargetChoice::Palette(PaletteChoice::Act(0)),
            settings: ConvertSettings::default(),
            item: 0,
            zoom: 3,
            side_by_side: true,
            show_after: true,
            highlight_changed: false,
            highlight_worst: false,
            hovered: None,
            picker: None,
            script_hover: None,
            result: None,
            batch: batch::BatchUi::default(),
            dt1_filter: String::new(),
            dt1_list: false,
            preset_name: String::new(),
            error: None,
            plan_cache: None,
            tex: None,
        }
    }
}

impl PaletteConvert {
    pub fn item_count(&self) -> usize {
        self.input.as_ref().map_or(0, |i| i.items.len())
    }

    pub fn set_item(&mut self, item: usize) {
        self.item = item.min(self.item_count().saturating_sub(1));
    }
}

/// `text` as a game path for a converted file of `kind`: relative, under
/// `data\`, with the kind's extension.
pub fn check_converted_path(text: &str, kind: SpriteKind) -> Result<GamePath, String> {
    let t = text.trim();
    let path = GamePath::new(t);
    let s = path.as_str();
    if t.is_empty() || s.contains(':') || s.starts_with('\\') || s.split('\\').any(|p| p == "..") {
        return Err(format!(
            "{t:?} must be a game path relative to the MPQ root"
        ));
    }
    if !s.starts_with("data\\") {
        return Err(format!("{t:?} must start with data\\"));
    }
    if SpriteKind::from_path(s) != Some(kind) {
        return Err(format!("{t:?} must end in .{}", kind_ext(kind)));
    }
    Ok(path)
}

/// The folders of every game install the profiles read (never written).
pub fn install_dirs(profiles: &[d2_assets::Profile]) -> Vec<PathBuf> {
    use d2_assets::LayerSpec;
    let mut out = Vec::new();
    for p in profiles {
        for l in &p.layers {
            match l {
                LayerSpec::Mpq { path, .. } | LayerSpec::Casc { path } => {
                    if let Some(d) = path.parent() {
                        out.push(d.to_path_buf());
                    }
                }
                LayerSpec::MpqGlob { dir, .. } => out.push(dir.clone()),
                LayerSpec::Folder { path } => out.push(path.clone()),
                LayerSpec::Saves => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Refuses a write into a game install folder.
pub fn check_not_install(path: &Path, installs: &[PathBuf]) -> Result<(), String> {
    let norm = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_start_matches("\\\\?\\")
            .to_ascii_lowercase()
    };
    let target = {
        // The file may not exist yet: compare its folder.
        let dir = path.parent().unwrap_or(path);
        format!("{}\\", norm(dir).trim_end_matches('\\'))
    };
    for d in installs {
        let d = format!("{}\\", norm(d).trim_end_matches('\\'));
        if d.len() > 3 && target.starts_with(&d) {
            return Err(format!(
                "{} is inside the game install {}; game folders are never written",
                path.display(),
                d.trim_end_matches('\\')
            ));
        }
    }
    Ok(())
}

fn hash_plan_key(
    src: &Palette,
    dst: &Palette,
    used: &[u64; 256],
    settings: &impl std::fmt::Debug,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.colors.hash(&mut h);
    dst.colors.hash(&mut h);
    used.hash(&mut h);
    format!("{settings:?}").hash(&mut h);
    h.finish()
}

impl AppState {
    fn convert_profile_overlay(&self) -> Option<Arc<d2_assets::Overlay>> {
        self.sprite_overlay()
    }

    /// The raw source and target palettes.
    pub fn convert_palettes(&mut self) -> Result<(Palette, Palette), String> {
        let overlay = self.convert_profile_overlay();
        let epoch = self.session_epoch;
        let c = &self.sprites.convert;
        let (src, dst) = (c.source.clone(), c.target.palette());
        let store = &mut self.sprites.palette;
        let a = store
            .raw_of(&src, overlay.as_deref(), epoch)
            .ok_or_else(|| format!("palette {} cannot be read", src.label()))?;
        let b = store
            .raw_of(&dst, overlay.as_deref(), epoch)
            .ok_or_else(|| format!("palette {} cannot be read", dst.label()))?;
        Ok((a, b))
    }

    /// The plan for the current input and settings (cached; cheap).
    pub fn convert_plan(&mut self) -> Option<Arc<RemapPlan>> {
        let used = self.sprites.convert.input.as_ref()?.histogram;
        let (src, dst) = match self.convert_palettes() {
            Ok(p) => p,
            Err(e) => {
                self.sprites.convert.error = Some(e);
                return None;
            }
        };
        let c = &mut self.sprites.convert;
        let settings = c.settings.to_remap(c.target.shared_only());
        let key = hash_plan_key(&src, &dst, &used, &settings);
        if let Some((k, p)) = &c.plan_cache
            && *k == key
        {
            return Some(p.clone());
        }
        let p = Arc::new(make_plan(&src, &dst, &used, &settings));
        c.plan_cache = Some((key, p.clone()));
        c.error = None;
        Some(p)
    }

    /// The `k` best targets for `source` (the override picker).
    pub fn convert_candidates(&mut self, source: u8, k: usize) -> Vec<(u8, f32)> {
        let Ok((src, dst)) = self.convert_palettes() else {
            return Vec::new();
        };
        let c = &self.sprites.convert;
        let mut settings = c.settings.to_remap(c.target.shared_only());
        settings.overrides.clear();
        candidates(&src, &dst, source, &settings, k)
    }

    // ───────── opening ─────────

    /// Opens the window on a file read through the profile.
    pub fn convert_open_game(&mut self, path: &GamePath) -> Result<(), String> {
        let result = (|| {
            let kind = SpriteKind::from_path(path.as_str())
                .ok_or_else(|| format!("{path} is not a .dt1, .dc6 or .dcc file"))?;
            let overlay = self.sprite_overlay().ok_or("no profile is open")?;
            let bytes = overlay
                .read_bytes(path)
                .map_err(|e| format!("cannot read {path}: {e}"))?
                .ok_or_else(|| format!("{path} is not in the profile"))?;
            self.convert_open_bytes(kind, bytes, InputOrigin::Game(path.clone()))
        })();
        self.report_convert_open(result)
    }

    /// Opens the window on a loose file.
    pub fn convert_open_file(&mut self, file: &Path) -> Result<(), String> {
        let result = (|| {
            let kind = SpriteKind::from_path(&file.to_string_lossy())
                .ok_or_else(|| format!("{} is not a .dt1, .dc6 or .dcc file", file.display()))?;
            let bytes =
                std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            self.convert_open_bytes(kind, bytes, InputOrigin::File(file.to_path_buf()))
        })();
        self.report_convert_open(result)
    }

    /// Opens the window on what the Viewer shows (a DC6/DCC/DT1).
    pub fn convert_open_viewer(&mut self) -> Result<(), String> {
        let result = (|| {
            let source = self
                .sprites
                .viewer
                .source
                .clone()
                .ok_or("open a DC6, DCC or DT1 in the Viewer first")?;
            match &source {
                SpriteSource::Game(p) => {
                    let p = p.clone();
                    return self.convert_open_game(&p);
                }
                SpriteSource::File(f) => {
                    let f = f.clone();
                    return self.convert_open_file(&f);
                }
                SpriteSource::Converted { .. } => {
                    let r = self
                        .sprites
                        .convert
                        .result
                        .as_ref()
                        .ok_or("the converted file is gone")?;
                    let (kind, bytes) = (r.kind, r.bytes.as_ref().clone());
                    let origin = InputOrigin::Unsaved {
                        label: r.label(),
                        game: r.origin.game_path().cloned(),
                    };
                    return self.convert_open_bytes(kind, bytes, origin);
                }
                SpriteSource::Imported(_) | SpriteSource::Reimported { .. } => {}
            }
            let (sprite, format) = self
                .sprites
                .viewer
                .sprite()
                .ok_or("a COF cannot be converted; convert its layers' DCC files")?;
            let bytes = sprite.encode(format).map_err(|e| e.to_string())?;
            let kind = match format {
                SpriteFormat::Dc6 => SpriteKind::Dc6,
                SpriteFormat::Dcc => SpriteKind::Dcc,
            };
            self.convert_open_bytes(
                kind,
                bytes,
                InputOrigin::Unsaved {
                    label: source.label(),
                    game: source.save_path().cloned(),
                },
            )
        })();
        if let Err(e) = &result {
            self.error(format!("Convert to palette: {e}"));
        }
        result
    }

    /// The Tiles browser's "Convert tile set to palette...": the DT1 of a
    /// tile key (`<dt1>#o/m/s/n`) or a file key, showing that tile.
    pub fn convert_open_tile_key(&mut self, key: &str) {
        let (path, tile) = match crate::browser::brush::parse_tile_key(key) {
            Some((p, t)) => (p.to_owned(), Some(t)),
            None => (key.split('#').next().unwrap_or(key).to_owned(), None),
        };
        if self.convert_open_game(&GamePath::new(&path)).is_err() {
            return;
        }
        if let Some(t) = tile
            && let Some(i) = self.sprites.convert.input.as_ref().and_then(|inp| {
                inp.find_tile(
                    i32::from(t.orientation),
                    i32::from(t.main),
                    i32::from(t.sub),
                    Some(t.n),
                )
                .or_else(|| {
                    inp.find_tile(
                        i32::from(t.orientation),
                        i32::from(t.main),
                        i32::from(t.sub),
                        None,
                    )
                })
            })
        {
            self.sprites.convert.item = i;
        }
    }

    /// Opens the window on the Viewer's or the Editor's sprite with this
    /// source and target (the palette warning's "Convert to palette...").
    pub fn convert_open_target(
        &mut self,
        target: super::Target,
        source: PaletteChoice,
        to: PaletteChoice,
    ) {
        let opened = match target {
            super::Target::Viewer => self.convert_open_viewer(),
            super::Target::Editor => {
                let r = (|| {
                    let d = self
                        .target_data(super::Target::Editor)
                        .ok_or("the editor is empty")?;
                    let bytes = d.sprite.encode(d.format).map_err(|e| e.to_string())?;
                    let kind = match d.format {
                        SpriteFormat::Dc6 => SpriteKind::Dc6,
                        SpriteFormat::Dcc => SpriteKind::Dcc,
                    };
                    let source = d.source.unwrap_or(SpriteSource::Imported("sprite".into()));
                    self.convert_open_bytes(
                        kind,
                        bytes,
                        InputOrigin::Unsaved {
                            label: format!("(editor) {}", source.label()),
                            game: source.save_path().cloned(),
                        },
                    )
                })();
                self.report_convert_open(r)
            }
        };
        if opened.is_ok() {
            let c = &mut self.sprites.convert;
            c.source = source;
            c.target = TargetChoice::Palette(to);
        }
    }

    fn report_convert_open(&mut self, result: Result<(), String>) -> Result<(), String> {
        match &result {
            Ok(()) => {
                let c = &self.sprites.convert;
                if let Some(i) = &c.input {
                    let text = format!(
                        "Convert to palette: {} ({}, {} {}), source {}",
                        i.origin.label(),
                        kind_label(i.kind),
                        i.items.len(),
                        if i.kind == SpriteKind::Dt1 {
                            "tiles"
                        } else {
                            "frames"
                        },
                        c.source.label()
                    );
                    self.info(text);
                }
            }
            Err(e) => {
                self.sprites.convert.error = Some(e.clone());
                self.error(format!("Convert to palette: {e}"));
            }
        }
        result
    }

    /// Installs `bytes` as the input, with the default source palette:
    /// remembered for the file, else its folder's, else guessed from the
    /// path, else the current one.
    pub fn convert_open_bytes(
        &mut self,
        kind: SpriteKind,
        bytes: Vec<u8>,
        origin: InputOrigin,
    ) -> Result<(), String> {
        let input = ConvertInput::load(kind, bytes, origin)?;
        let key = input.origin.memory_key();
        let source = key
            .as_deref()
            .and_then(|k| {
                self.remembered_palette(k)
                    .or_else(|| self.folder_palette(k))
                    .and_then(|l| PaletteChoice::from_label(&l))
                    .or_else(|| guess_from_path(k))
            })
            .unwrap_or_else(|| self.sprites.palette.choice.clone());
        let c = &mut self.sprites.convert;
        c.source = source;
        if c.target.palette() == c.source && !c.target.shared_only() {
            c.target = TargetChoice::Palette(if c.source == PaletteChoice::Act(0) {
                PaletteChoice::Act(4)
            } else {
                PaletteChoice::Act(0)
            });
        }
        c.input = Some(input);
        c.item = 0;
        c.hovered = None;
        c.picker = None;
        c.error = None;
        c.tex = None;
        c.open = true;
        Ok(())
    }

    /// Whether to suggest the "All acts" target for the input.
    pub fn convert_suggests_all_acts(&self) -> bool {
        let c = &self.sprites.convert;
        !c.target.shared_only()
            && c.input
                .as_ref()
                .and_then(|i| i.origin.memory_key())
                .is_some_and(|k| suggests_all_acts(&k))
    }

    /// The user picked the source palette: it is the file's palette.
    pub fn convert_set_source(&mut self, choice: PaletteChoice) {
        self.sprites.convert.source = choice.clone();
        if let Some(k) = self
            .sprites
            .convert
            .input
            .as_ref()
            .and_then(|i| i.origin.memory_key())
        {
            self.remember_palette(&k, &choice);
        }
    }

    // ───────── converting ─────────

    /// The Convert button: applies the plan's table and opens the result as
    /// an unsaved converted file in the Viewer.
    pub fn convert_apply(&mut self) -> Result<(), String> {
        let result = (|| {
            let plan = self.convert_plan().ok_or_else(|| {
                self.sprites
                    .convert
                    .error
                    .clone()
                    .unwrap_or_else(|| "open a file to convert first".to_owned())
            })?;
            let c = &self.sprites.convert;
            let input = c.input.as_ref().ok_or("open a file to convert first")?;
            let bytes = remap_indices(input.kind, &input.bytes, &plan.table)
                .map_err(|e| format!("cannot convert {}: {e}", input.origin.label()))?;
            Ok(Converted {
                kind: input.kind,
                bytes: Arc::new(bytes),
                origin: input.origin.clone(),
                target: c.target.clone(),
                summary: table::summary(&plan),
                saved: None,
            })
        })();
        match result {
            Ok(done) => {
                let s = done.summary;
                let text = format!(
                    "Converted {} to {}: {} colour(s) changed, mean error {:.2}, worst {:.1}",
                    done.origin.label(),
                    done.target.label(),
                    s.changed,
                    s.mean,
                    s.worst
                );
                self.sprites.convert.result = Some(done);
                self.info(text.clone());
                self.notify(text);
                self.open_converted_in_viewer()
            }
            Err(e) => {
                self.error(format!("Convert to palette: {e}"));
                Err(e)
            }
        }
    }

    /// Shows the converted file in the Viewer under the target palette.
    pub fn open_converted_in_viewer(&mut self) -> Result<(), String> {
        let r = self
            .sprites
            .convert
            .result
            .as_ref()
            .ok_or("nothing was converted")?;
        let source = SpriteSource::Converted {
            label: r.label(),
            game: r.origin.game_path().cloned(),
        };
        let loaded = match sprite_format(r.kind) {
            Some(format) => Loaded::Sprite(SingleSprite {
                sprite: Sprite::decode(&r.bytes, format).map_err(|e| e.to_string())?,
                format,
            }),
            None => Loaded::Dt1(Box::new(Dt1View::load(&r.bytes)?)),
        };
        let target = r.target.palette();
        self.sprites.viewer.set_loaded(
            loaded,
            source,
            super::viewer::DEFAULT_FPS,
            "default".to_owned(),
        );
        self.sprites.palette.choice = target;
        self.sync_editor_palette();
        self.sprites.focus = Some(Focus::Viewer);
        Ok(())
    }

    /// "Save to game path" for the converted file: saves, CHANGED.txt, a
    /// snapshot; the target palette is remembered for the path.
    pub fn convert_save_game(&mut self, text: &str) -> Result<PathBuf, String> {
        let result = (|| {
            let r = self
                .sprites
                .convert
                .result
                .as_ref()
                .ok_or("nothing was converted")?;
            let path = check_converted_path(text, r.kind)?;
            let profile = self
                .session
                .as_ref()
                .map(|s| s.profile.id.clone())
                .ok_or("no profile is open")?;
            let report = crate::edit::save::save_game_file(&self.paths, &profile, &path, &r.bytes)?;
            Ok((path, report, r.target.palette()))
        })();
        match result {
            Ok((path, report, palette)) => {
                for w in &report.warnings {
                    self.warn(format!("{path}: {w}"));
                }
                self.info(format!(
                    "Saved the converted {path} ({} bytes) to {}; CHANGED.txt lists {} file(s)",
                    report.bytes,
                    report.path.display(),
                    report.changed_total
                ));
                self.remember_palette(path.as_str(), &palette);
                if let Some(r) = self.sprites.convert.result.as_mut() {
                    r.saved = Some(path.as_str().to_owned());
                }
                if let Some(SpriteSource::Converted { game, .. }) =
                    self.sprites.viewer.source.as_mut()
                {
                    *game = Some(path.clone());
                }
                self.refresh_changed();
                self.history.invalidate();
                self.start_saves_refresh();
                Ok(report.path)
            }
            Err(e) => {
                self.error(format!("Save failed: {e}"));
                Err(e)
            }
        }
    }

    /// "Save As...": the converted bytes to a file the user picked (never
    /// inside a game install).
    pub fn convert_save_as(&mut self, file: &Path) -> Result<PathBuf, String> {
        let result = (|| {
            let r = self
                .sprites
                .convert
                .result
                .as_ref()
                .ok_or("nothing was converted")?;
            let file = super::convert::with_extension(file, kind_ext(r.kind));
            check_not_install(&file, &install_dirs(&self.profiles))?;
            if let Some(dir) = file.parent()
                && !dir.as_os_str().is_empty()
            {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            std::fs::write(&file, r.bytes.as_ref())
                .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
            Ok((file, r.target.palette(), r.bytes.len()))
        })();
        match result {
            Ok((file, palette, n)) => {
                self.info(format!(
                    "Saved the converted file to {} ({n} bytes)",
                    file.display()
                ));
                self.remember_palette(&file.display().to_string(), &palette);
                if let Some(r) = self.sprites.convert.result.as_mut() {
                    r.saved = Some(file.display().to_string());
                }
                Ok(file)
            }
            Err(e) => {
                self.error(format!("Save As failed: {e}"));
                Err(e)
            }
        }
    }

    /// Loads a named preset (settings and overrides).
    pub fn convert_load_preset(&mut self, name: &str) -> Result<(), String> {
        match settings::find_preset(&self.config.palette_presets, name) {
            Some(p) => {
                self.sprites.convert.settings = p.settings.clone().clamped();
                self.sprites.convert.preset_name = p.name.clone();
                Ok(())
            }
            None => Err(format!("no preset named {name:?}")),
        }
    }

    // ───────── batch ─────────

    /// Adds the profile's DT1/DC6/DCC files whose game path contains
    /// `filter` (case-insensitive; empty adds nothing).
    pub fn convert_batch_add_game(&mut self, filter: &str) -> usize {
        let needle = filter.trim().replace('/', "\\").to_ascii_lowercase();
        if needle.is_empty() {
            return 0;
        }
        let items: Vec<batch::BatchItem> = self
            .session
            .as_ref()
            .map(|s| {
                s.sprites
                    .iter()
                    .filter(|p| SpriteKind::from_path(p.as_str()).is_some())
                    .filter(|p| p.as_str().contains(&needle))
                    .map(|p| batch::BatchItem::Game(p.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let n = self.sprites.convert.batch.add(items);
        self.info(format!(
            "Convert batch: added {n} game file(s) matching {needle:?}"
        ));
        n
    }

    /// Starts the batch with the window's source, target and settings.
    pub fn convert_batch_start(&mut self) -> Result<(), String> {
        let result = (|| {
            if self.sprites.convert.batch.task.is_some() {
                return Err("a batch is running".to_owned());
            }
            let (src, dst) = self.convert_palettes()?;
            let c = &self.sprites.convert;
            let b = &c.batch;
            if b.items.is_empty() {
                return Err("add files first".to_owned());
            }
            let output = if b.to_saves {
                batch::BatchOutput::Saves {
                    profile: self
                        .session
                        .as_ref()
                        .map(|s| s.profile.id.clone())
                        .ok_or("saving to the game path needs a profile")?,
                }
            } else {
                batch::BatchOutput::Folder(b.out_dir.clone().ok_or("pick an output folder")?)
            };
            let job = batch::BatchJob {
                items: b.items.clone(),
                src,
                dst,
                settings: c.settings.to_remap(c.target.shared_only()),
                output,
                overlay: self.sprite_overlay(),
                paths: self.paths.clone(),
                installs: install_dirs(&self.profiles),
            };
            let n = job.items.len();
            let text = format!(
                "Convert batch started: {n} file(s), {} -> {}",
                c.source.label(),
                c.target.label()
            );
            let task = batch::PaletteBatchTask::spawn(job)?;
            Ok((task, text))
        })();
        match result {
            Ok((task, text)) => {
                self.sprites.convert.batch.task = Some(task);
                self.sprites.convert.batch.outcome = None;
                self.info(text);
                Ok(())
            }
            Err(e) => {
                self.error(format!("Convert batch: {e}"));
                Err(e)
            }
        }
    }

    /// Batch results, once per frame.
    pub fn poll_convert_batch(&mut self) {
        let Some(report) = self
            .sprites
            .convert
            .batch
            .task
            .as_ref()
            .and_then(batch::PaletteBatchTask::poll)
        else {
            return;
        };
        self.sprites.convert.batch.task = None;
        let lines = batch::report_lines(&report);
        for f in &report.files {
            if let Err(e) = &f.result {
                self.error(format!("Convert batch failed: {}: {e}", f.name));
            }
        }
        for l in lines
            .iter()
            .filter(|l| !l.starts_with("Convert batch failed"))
        {
            self.info(l.clone());
        }
        let target = self.sprites.convert.target.palette();
        let mut saved = 0;
        for f in &report.files {
            if let Ok(d) = &f.result {
                let key = d
                    .game
                    .as_ref()
                    .map_or_else(|| d.written.clone(), |g| g.as_str().to_owned());
                self.remember_palette(&key, &target);
                saved += usize::from(d.game.is_some());
            }
        }
        if saved > 0 {
            self.refresh_changed();
            self.history.invalidate();
            self.start_saves_refresh();
        }
        let last = lines.last().cloned().unwrap_or_default();
        self.notify(last.clone());
        self.sprites.convert.batch.outcome = Some((report.cancelled, last));
    }

    /// Waits for the batch to finish (scripts).
    pub fn convert_batch_running(&self) -> bool {
        self.sprites.convert.batch.task.is_some()
    }

    /// The file name "Save As..." offers.
    pub fn converted_file_name(&self) -> String {
        self.sprites
            .convert
            .result
            .as_ref()
            .map_or_else(|| "converted".to_owned(), |r| r.origin.file_name())
    }

    /// The converted file's game path (where "Save to game path" points).
    pub fn converted_save_text(&self) -> String {
        self.sprites
            .convert
            .result
            .as_ref()
            .map_or_else(String::new, |r| {
                r.origin.game_path().map_or_else(
                    || format!("data\\global\\{}", r.origin.file_name()),
                    |g| g.as_str().to_owned(),
                )
            })
    }
}

#[cfg(test)]
mod tests;
