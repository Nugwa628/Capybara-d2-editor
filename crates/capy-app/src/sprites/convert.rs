//! The Converter: image import into a new sprite, the export formats, the
//! batch task and "Save to game path".
//!
//! Output rule: "Save to game path" writes `capybara\saves\<profile>\<game
//! path>` (CHANGED.txt + snapshot, like a map save); exports and batch runs
//! write to a folder the user picked in a dialog. Nothing else is written.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use capy_sprite::{
    BatchDirection, BatchOptions, BatchReport, Dither, ExportMeta, ImageKind, QuantizeOptions,
    Sprite, SpriteFormat, batch_convert, center_offset, export_png, import_image, import_sheet,
    quantize, split_frame,
};
use d2_assets::GamePath;
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use crate::edit::save::{SaveReport, save_game_file};
use crate::paths::Paths;

/// Edge of the tiles large frames are split into.
pub const SPLIT_EDGE: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One image per frame, direction by direction.
    Frames,
    /// One image cut into rows (directions) x columns (frames).
    Sheet,
}

/// Colour matching options shared by import and batch.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSettings {
    pub dither: Dither,
    /// Treat `transparent_rgb` as transparent.
    pub transparent_key: bool,
    pub transparent_rgb: [u8; 3],
    pub alpha_threshold: u8,
    /// Opaque pixels never become index 0.
    pub protect: bool,
}

impl Default for MatchSettings {
    fn default() -> Self {
        let d = QuantizeOptions::default();
        MatchSettings {
            dither: d.dither,
            transparent_key: d.transparent_rgb.is_some(),
            transparent_rgb: d.transparent_rgb.unwrap_or([0, 0, 0]),
            alpha_threshold: d.alpha_threshold,
            protect: d.protect_transparent,
        }
    }
}

impl MatchSettings {
    pub fn quantize(&self) -> QuantizeOptions {
        QuantizeOptions {
            dither: self.dither,
            transparent_rgb: self.transparent_key.then_some(self.transparent_rgb),
            alpha_threshold: self.alpha_threshold,
            protect_transparent: self.protect,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportSettings {
    pub files: Vec<PathBuf>,
    pub layout: Layout,
    pub directions: u32,
    /// Frames per direction; 0 = as many as the files allow.
    pub frames: u32,
    pub matching: MatchSettings,
    /// Cut frames larger than 256x256 into 256x256 tiles (row-major).
    pub split: bool,
    pub format: SpriteFormat,
}

impl Default for ImportSettings {
    fn default() -> Self {
        ImportSettings {
            files: Vec::new(),
            layout: Layout::Frames,
            directions: 1,
            frames: 0,
            matching: MatchSettings::default(),
            split: false,
            format: SpriteFormat::Dcc,
        }
    }
}

fn kind_of(path: &Path) -> Result<ImageKind, String> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(ImageKind::from_extension)
        .ok_or_else(|| format!("{} is not a PNG, BMP, GIF or PCX file", path.display()))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// `sprite.json` next to the first file, when there is one.
fn sibling_meta(files: &[PathBuf]) -> Option<ExportMeta> {
    let dir = files.first()?.parent()?;
    let json = std::fs::read(dir.join("sprite.json")).ok()?;
    serde_json::from_slice(&json).ok()
}

/// The sheet's own metadata: `<stem>.json` next to it (what [`export_sheet`]
/// writes).
fn sheet_meta(file: &Path) -> Option<ExportMeta> {
    let json = std::fs::read(file.with_extension("json")).ok()?;
    serde_json::from_slice(&json).ok()
}

/// The `sprite.json` (or sheet `.json`) an import would read, if any.
pub fn import_meta(settings: &ImportSettings) -> Option<ExportMeta> {
    match settings.layout {
        Layout::Frames => sibling_meta(&settings.files),
        Layout::Sheet => settings.files.first().and_then(|f| sheet_meta(f)),
    }
}

/// (directions, frames) for `count` images.
pub fn frame_layout(count: usize, directions: u32, frames: u32) -> Result<(usize, usize), String> {
    let dirs = directions.max(1) as usize;
    if count == 0 {
        return Err("pick at least one image".to_owned());
    }
    let per = if frames == 0 {
        if !count.is_multiple_of(dirs) {
            return Err(format!(
                "{count} images cannot be split into {dirs} directions"
            ));
        }
        count / dirs
    } else {
        frames as usize
    };
    if dirs * per != count {
        return Err(format!(
            "{dirs} directions x {per} frames needs {} images, {count} were picked",
            dirs * per
        ));
    }
    Ok((dirs, per))
}

/// Builds the sprite of an import. Offsets come from a `sprite.json` next to
/// the files (exact re-import of an export) when it names them; otherwise
/// each frame is anchored bottom-centre.
pub fn build_import(
    settings: &ImportSettings,
    palette: &Palette,
) -> Result<(Sprite, Option<ExportMeta>), String> {
    let opts = settings.matching.quantize();
    let meta;
    let mut sprite = match settings.layout {
        Layout::Frames => {
            meta = sibling_meta(&settings.files);
            let (dirs, per) =
                frame_layout(settings.files.len(), settings.directions, settings.frames)?;
            if let Some(dir) = whole_export(&settings.files, meta.as_ref(), (dirs, per)) {
                // Exactly an export's frames in its layout: the library reads
                // it (offsets, and no colour keying, from sprite.json).
                let (sprite, _) = capy_sprite::import_frames(dir, palette, &opts)
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
                return finish_import(sprite, settings, meta);
            }
            let frames = match (meta.is_some(), same_folder(&settings.files)) {
                // Some frames of an export: the library reads them with their
                // sprite.json offsets and no colour key, in export order.
                (true, Some(dir)) => {
                    let names: Vec<String> = settings
                        .files
                        .iter()
                        .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .collect();
                    let (sprite, _) =
                        capy_sprite::import_frames_subset(dir, &names, palette, &opts)
                            .map_err(|e| format!("{}: {e}", dir.display()))?;
                    sprite.directions.into_iter().flatten().collect()
                }
                // Loose images: colour key as set, anchored bottom-centre.
                _ => {
                    let mut frames = Vec::with_capacity(settings.files.len());
                    for file in &settings.files {
                        let rgba = import_image(&read(file)?, kind_of(file)?)
                            .map_err(|e| format!("{}: {e}", file.display()))?;
                        let mut img = quantize(&rgba, palette, &opts);
                        center_offset(&mut img);
                        frames.push(img);
                    }
                    frames
                }
            };
            lay_out(frames, dirs, per)?
        }
        Layout::Sheet => {
            let [file] = settings.files.as_slice() else {
                return Err("a sheet import takes exactly one image".to_owned());
            };
            let bytes = read(file)?;
            let png = match kind_of(file)? {
                ImageKind::Png => bytes,
                kind => {
                    let rgba = import_image(&bytes, kind).map_err(|e| e.to_string())?;
                    export_png(&rgba).map_err(|e| e.to_string())?
                }
            };
            meta = sheet_meta(file);
            import_sheet(
                &png,
                settings.directions.max(1),
                settings.frames.max(1),
                palette,
                &opts,
                meta.as_ref(),
            )
            .map_err(|e| e.to_string())?
        }
    };
    if settings.split {
        sprite = split_large(sprite)?;
    }
    Ok((sprite, meta))
}

/// The one folder all `files` are in.
fn same_folder(files: &[PathBuf]) -> Option<&Path> {
    let dir = files.first()?.parent()?;
    files.iter().all(|f| f.parent() == Some(dir)).then_some(dir)
}

/// `frames` as `dirs` directions of `per` frames, in order.
fn lay_out(frames: Vec<IndexedImage>, dirs: usize, per: usize) -> Result<Sprite, String> {
    if frames.len() != dirs * per {
        return Err(format!(
            "{dirs} directions x {per} frames needs {} images, {} were read",
            dirs * per,
            frames.len()
        ));
    }
    let mut directions: Vec<Vec<IndexedImage>> = vec![Vec::with_capacity(per); dirs];
    for (i, img) in frames.into_iter().enumerate() {
        directions[i / per.max(1)].push(img);
    }
    Ok(Sprite { directions })
}

fn finish_import(
    sprite: Sprite,
    settings: &ImportSettings,
    meta: Option<ExportMeta>,
) -> Result<(Sprite, Option<ExportMeta>), String> {
    let sprite = if settings.split {
        split_large(sprite)?
    } else {
        sprite
    };
    Ok((sprite, meta))
}

/// The folder of `files` when they are exactly the frames its `sprite.json`
/// lists, in its layout `(directions, frames)` (an export picked whole).
fn whole_export<'a>(
    files: &'a [PathBuf],
    meta: Option<&ExportMeta>,
    layout: (usize, usize),
) -> Option<&'a Path> {
    let meta = meta?;
    let dir = files.first()?.parent()?;
    let meta_layout = (
        usize::try_from(meta.directions).ok()?,
        usize::try_from(meta.frames_per_direction).ok()?,
    );
    if layout != meta_layout
        || files.len() != meta.frames.len()
        || files.iter().any(|f| f.parent() != Some(dir))
    {
        return None;
    }
    let mut picked: Vec<String> = files
        .iter()
        .filter_map(|f| {
            f.file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
        })
        .collect();
    let mut listed: Vec<String> = meta
        .frames
        .iter()
        .map(|f| f.file.to_ascii_lowercase())
        .collect();
    picked.sort();
    listed.sort();
    (picked == listed).then_some(dir)
}

/// Replaces frames larger than 256x256 by their 256x256 tiles. Every
/// direction must end up with the same number of frames.
pub fn split_large(sprite: Sprite) -> Result<Sprite, String> {
    let mut directions = Vec::with_capacity(sprite.directions.len());
    for dir in sprite.directions {
        let mut out = Vec::new();
        for img in dir {
            if img.width > SPLIT_EDGE || img.height > SPLIT_EDGE {
                out.extend(split_frame(&img, SPLIT_EDGE, SPLIT_EDGE));
            } else {
                out.push(img);
            }
        }
        directions.push(out);
    }
    let per = directions.first().map_or(0, Vec::len);
    if directions.iter().any(|d| d.len() != per) {
        return Err("splitting gave the directions different frame counts".to_owned());
    }
    Ok(Sprite { directions })
}

/// The format a game path is saved as (its extension).
pub fn format_of(path: &str) -> Option<SpriteFormat> {
    let ext = path.rsplit_once('.')?.1;
    SpriteFormat::from_extension(ext)
}

/// Checks a game path typed by the user: relative, under `data\`, with a
/// `.dc6` or `.dcc` extension.
pub fn check_game_path(text: &str) -> Result<(GamePath, SpriteFormat), String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("type the game path, e.g. data\\global\\items\\invfoo.dc6".to_owned());
    }
    let path = GamePath::new(t);
    let s = path.as_str();
    if s.contains(':') || s.starts_with('\\') || s.split('\\').any(|p| p == "..") {
        return Err(format!(
            "{t:?} must be a game path relative to the MPQ root"
        ));
    }
    if !s.starts_with("data\\") {
        return Err(format!("{t:?} must start with data\\"));
    }
    let format = format_of(s).ok_or_else(|| format!("{t:?} must end in .dc6 or .dcc"))?;
    Ok((path, format))
}

/// Encodes and writes the profile's copy of `game_path` (format from the
/// extension).
pub fn save_to_game(
    paths: &Paths,
    profile_id: &str,
    game_path: &GamePath,
    sprite: &Sprite,
    palette: &Palette,
) -> Result<SaveReport, String> {
    let format = format_of(game_path.as_str())
        .ok_or_else(|| format!("{game_path} is not a .dc6 or .dcc path"))?;
    let bytes = sprite
        .encode_with_palette(format, palette)
        .map_err(|e| format!("cannot encode {game_path}: {e}"))?;
    // What the game will read must decode.
    Sprite::decode(&bytes, format).map_err(|e| format!("the encoded file does not decode: {e}"))?;
    save_game_file(paths, profile_id, game_path, &bytes)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Adds `ext` unless the file already ends in it.
pub fn with_extension(path: &Path, ext: &str) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(ext) => path.to_path_buf(),
        _ => {
            let mut s = path.as_os_str().to_owned();
            s.push(format!(".{ext}"));
            PathBuf::from(s)
        }
    }
}

/// What a "Export to..." produced, for the log.
pub type ExportResult = Result<String, String>;

pub fn export_frames(
    sprite: &Sprite,
    format: SpriteFormat,
    palette: &Palette,
    palette_name: &str,
    source: Option<&str>,
    dir: &Path,
) -> ExportResult {
    let meta = capy_sprite::export_frames(sprite, format, palette, palette_name, source, dir)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Exported {} frames and sprite.json to {}",
        meta.frames.len(),
        dir.display()
    ))
}

pub fn export_sheet(sprite: &Sprite, palette: &Palette, file: &Path) -> ExportResult {
    let file = with_extension(file, "png");
    let (png, meta) = capy_sprite::export_sheet(sprite, palette).map_err(|e| e.to_string())?;
    write(&file, &png)?;
    let json = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
    let meta_file = file.with_extension("json");
    write(&meta_file, &json)?;
    Ok(format!(
        "Exported a {} x {} sheet to {} (offsets in {})",
        meta.directions,
        meta.frames_per_direction,
        file.display(),
        meta_file.display()
    ))
}

pub fn export_gif(
    frames: &[IndexedImage],
    palette: &Palette,
    fps: f32,
    file: &Path,
) -> ExportResult {
    let file = with_extension(file, "gif");
    let delay = (100.0 / fps.max(1.0)).round().clamp(2.0, 6000.0) as u16;
    let gif = capy_paint::animated_gif(frames, palette, delay).map_err(|e| e.to_string())?;
    write(&file, &gif)?;
    Ok(format!(
        "Exported {} frames to {}",
        frames.len(),
        file.display()
    ))
}

pub fn export_indexed_png(img: &IndexedImage, palette: &Palette, file: &Path) -> ExportResult {
    let file = with_extension(file, "png");
    let png = capy_paint::indexed_png(img, palette).map_err(|e| e.to_string())?;
    write(&file, &png)?;
    Ok(format!("Exported an indexed PNG to {}", file.display()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteFormat {
    Gpl,
    Act,
    Jasc,
}

pub fn export_palette(
    palette: &Palette,
    name: &str,
    format: PaletteFormat,
    file: &Path,
) -> ExportResult {
    let (ext, bytes) = match format {
        PaletteFormat::Gpl => ("gpl", capy_paint::palette_gpl(palette, name).into_bytes()),
        PaletteFormat::Act => ("act", capy_paint::palette_act(palette)),
        PaletteFormat::Jasc => ("pal", capy_paint::palette_jasc(palette).into_bytes()),
    };
    let file = with_extension(file, ext);
    write(&file, &bytes)?;
    Ok(format!("Exported the palette to {}", file.display()))
}

pub fn export_sprite_file(
    sprite: &Sprite,
    format: SpriteFormat,
    palette: &Palette,
    file: &Path,
) -> ExportResult {
    let file = with_extension(file, format.extension());
    let bytes = sprite
        .encode_with_palette(format, palette)
        .map_err(|e| e.to_string())?;
    write(&file, &bytes)?;
    Ok(format!(
        "Exported {} ({} bytes)",
        file.display(),
        bytes.len()
    ))
}

// ───────── batch ─────────

#[derive(Clone, Debug, PartialEq)]
pub struct BatchSettings {
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    /// true: images to sprites; false: sprites to PNG frames.
    pub to_sprite: bool,
    pub format: SpriteFormat,
    pub recursive: bool,
    pub overwrite: bool,
    pub split: bool,
    pub matching: MatchSettings,
}

impl Default for BatchSettings {
    fn default() -> Self {
        BatchSettings {
            input: None,
            output: None,
            to_sprite: false,
            format: SpriteFormat::Dcc,
            recursive: true,
            overwrite: false,
            split: false,
            matching: MatchSettings::default(),
        }
    }
}

impl BatchSettings {
    pub fn direction(&self) -> BatchDirection {
        if self.to_sprite {
            BatchDirection::ToSprite(self.format)
        } else {
            BatchDirection::ToPng
        }
    }

    pub fn options(&self) -> BatchOptions {
        BatchOptions {
            recursive: self.recursive,
            quantize: self.matching.quantize(),
            split: self.split.then_some((SPLIT_EDGE, SPLIT_EDGE)),
            overwrite: self.overwrite,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: String,
}

/// A batch conversion on a worker thread.
pub struct BatchTask {
    pub progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    rx: Mutex<Receiver<Result<BatchReport, String>>>,
    pub started: Instant,
}

impl BatchTask {
    pub fn spawn(settings: &BatchSettings, palette: Palette) -> Result<BatchTask, String> {
        let input = settings.input.clone().ok_or("pick an input folder")?;
        let output = settings.output.clone().ok_or("pick an output folder")?;
        let direction = settings.direction();
        let options = settings.options();
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (p, c) = (progress.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("sprite-batch".to_owned())
            .spawn(move || {
                let mut report = |done: usize, total: usize, name: &str| {
                    if let Ok(mut g) = p.lock() {
                        *g = Progress {
                            done,
                            total,
                            current: name.to_owned(),
                        };
                    }
                };
                let result = batch_convert(
                    &input,
                    &output,
                    &direction,
                    &palette,
                    &options,
                    &mut report,
                    &c,
                )
                .map_err(|e| e.to_string());
                let _ = tx.send(result);
            })
            .map_err(|e| format!("cannot start the batch: {e}"))?;
        Ok(BatchTask {
            progress,
            cancel,
            rx: Mutex::new(rx),
            started: Instant::now(),
        })
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// The result once the worker is done.
    pub fn poll(&self) -> Option<Result<BatchReport, String>> {
        self.rx.lock().ok()?.try_recv().ok()
    }
}

/// The one-line ending of a batch for the Converter: "Cancelled ..." when
/// the user stopped it (partial counts), "Done ..." otherwise.
pub fn batch_outcome(report: &BatchReport, secs: f32) -> String {
    let counts = format!(
        "{} converted, {} skipped, {} failed",
        report.converted,
        report.skipped,
        report.failed.len()
    );
    if report.cancelled {
        let how_far = report
            .log
            .last()
            .filter(|l| l.starts_with("cancelled"))
            .map_or_else(String::new, |l| format!(" ({l})"));
        format!("Cancelled after {secs:.1}s{how_far}: {counts}")
    } else {
        format!("Done in {secs:.1}s: {counts}")
    }
}

/// The log lines of a finished batch.
pub fn report_lines(report: &BatchReport) -> Vec<String> {
    let mut out = vec![format!(
        "Batch: {} converted, {} skipped, {} failed",
        report.converted,
        report.skipped,
        report.failed.len()
    )];
    out.extend(report.log.iter().take(200).cloned());
    out.extend(
        report
            .failed
            .iter()
            .take(200)
            .map(|(p, e)| format!("Batch failed: {}: {e}", p.display())),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprites::palette::grey_palette;

    #[test]
    fn a_cancelled_batch_says_cancelled_not_done() {
        let mut report = BatchReport {
            converted: 3,
            log: vec!["cancelled after 3 of 10 items".to_owned()],
            cancelled: true,
            ..BatchReport::default()
        };
        let text = batch_outcome(&report, 1.0);
        assert!(text.starts_with("Cancelled"), "{text}");
        assert!(
            text.contains("3 of 10") && text.contains("3 converted"),
            "{text}"
        );
        report.cancelled = false;
        assert!(batch_outcome(&report, 1.0).starts_with("Done"));
    }

    #[test]
    fn frame_layouts_must_match_the_file_count() {
        assert_eq!(frame_layout(8, 2, 0), Ok((2, 4)));
        assert_eq!(frame_layout(8, 2, 4), Ok((2, 4)));
        assert!(frame_layout(7, 2, 0).is_err());
        assert!(frame_layout(8, 2, 3).is_err());
        assert!(frame_layout(0, 1, 0).is_err());
        assert_eq!(frame_layout(3, 0, 0), Ok((1, 3)), "0 directions means 1");
    }

    #[test]
    fn match_settings_map_to_quantize_options() {
        let mut m = MatchSettings::default();
        assert_eq!(m.quantize(), QuantizeOptions::default());
        m.transparent_key = false;
        m.dither = Dither::FloydSteinberg;
        m.alpha_threshold = 10;
        m.protect = false;
        let q = m.quantize();
        assert_eq!(q.transparent_rgb, None);
        assert_eq!(q.dither, Dither::FloydSteinberg);
        assert_eq!(q.alpha_threshold, 10);
        assert!(!q.protect_transparent);
        let b = BatchSettings {
            split: true,
            to_sprite: true,
            format: SpriteFormat::Dc6,
            ..BatchSettings::default()
        };
        assert_eq!(b.direction(), BatchDirection::ToSprite(SpriteFormat::Dc6));
        assert_eq!(b.options().split, Some((256, 256)));
        assert_eq!(BatchSettings::default().direction(), BatchDirection::ToPng);
    }

    #[test]
    fn game_paths_are_checked() {
        assert_eq!(
            check_game_path(" data\\global\\items\\InvFoo.DC6 ")
                .map(|(p, f)| (p.as_str().to_owned(), f)),
            Ok((
                "data\\global\\items\\invfoo.dc6".to_owned(),
                SpriteFormat::Dc6
            ))
        );
        assert!(check_game_path("").is_err());
        assert!(check_game_path("C:\\x\\a.dcc").is_err());
        assert!(check_game_path("data\\..\\..\\a.dcc").is_err());
        assert!(check_game_path("global\\a.dcc").is_err());
        assert!(check_game_path("data\\a.png").is_err());
        assert_eq!(format_of("x.DCC"), Some(SpriteFormat::Dcc));
    }

    fn rgba_png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut f = capy_sprite::RgbaFrame::new(w, h, 0, 0);
        for px in f.pixels.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&color);
        }
        export_png(&f).expect("png")
    }

    #[test]
    fn import_builds_directions_and_honours_sprite_json() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut files = Vec::new();
        for i in 0..4 {
            let p = tmp.path().join(format!("f{i}.png"));
            std::fs::write(&p, rgba_png(4, 6, [200, 200, 200, 255])).expect("write");
            files.push(p);
        }
        let mut s = ImportSettings {
            files: files.clone(),
            directions: 2,
            ..ImportSettings::default()
        };
        let (sprite, meta) = build_import(&s, &grey_palette()).expect("import");
        assert!(meta.is_none());
        assert_eq!(sprite.directions.len(), 2);
        assert_eq!(sprite.frames_per_direction(), 2);
        // bottom-centre anchor without metadata
        assert!(sprite.directions[0][0].offset_y < 0);
        // with a sprite.json the offsets are exact
        let pal = grey_palette();
        let dir = tmp.path().join("exp");
        let mut shifted = sprite.clone();
        shifted.directions[1][1].offset_x = 17;
        let game = r"data\global\monsters\zz\a.dcc";
        export_frames(&shifted, SpriteFormat::Dcc, &pal, "grey", Some(game), &dir).expect("export");
        let exported: Vec<PathBuf> = [
            "d00_f000.png",
            "d00_f001.png",
            "d01_f000.png",
            "d01_f001.png",
        ]
        .iter()
        .map(|n| dir.join(n))
        .collect();
        s.files = exported.clone();
        let (back, meta) = build_import(&s, &pal).expect("re-import");
        assert_eq!(back, shifted);
        // The save prompt will offer the game path the export recorded.
        let source = crate::sprites::SpriteSource::import("x".into(), meta.as_ref());
        assert_eq!(
            source.save_path().map(|p| p.as_str().to_owned()),
            Some(game.to_owned())
        );
        // Part of an export still gets its exact offsets.
        s.files = exported[2..].to_vec();
        s.directions = 1;
        let (part, _) = build_import(&s, &pal).expect("part");
        assert_eq!(part.directions[0][1], shifted.directions[1][1]);
        s.files = exported;
        s.directions = 2;
        // wrong counts are refused, not a panic
        s.frames = 3;
        assert!(build_import(&s, &pal).is_err());
        s.files = vec![tmp.path().join("missing.png")];
        s.frames = 0;
        s.directions = 1;
        assert!(build_import(&s, &pal).is_err());
    }

    #[test]
    fn large_frames_split_into_tiles() {
        let img = IndexedImage::new(300, 100, 0, 0);
        let sprite = Sprite {
            directions: vec![vec![img]],
        };
        let out = split_large(sprite).expect("split");
        assert_eq!(out.frames_per_direction(), 2);
    }

    #[test]
    fn exports_write_where_asked_only() {
        let tmp = tempfile::tempdir().expect("tmp");
        let pal = grey_palette();
        let mut img = IndexedImage::new(3, 3, -1, -3);
        img.pixels[4] = 9;
        img.alpha[4] = 255;
        let sprite = Sprite {
            directions: vec![vec![img.clone(), img.clone()]],
        };
        export_sheet(&sprite, &pal, &tmp.path().join("sheet")).expect("sheet");
        assert!(tmp.path().join("sheet.png").is_file());
        assert!(tmp.path().join("sheet.json").is_file());
        export_gif(
            &sprite.directions[0],
            &pal,
            25.0,
            &tmp.path().join("anim.gif"),
        )
        .expect("gif");
        export_indexed_png(&img, &pal, &tmp.path().join("one")).expect("png");
        for (f, n) in [
            (PaletteFormat::Gpl, "p.gpl"),
            (PaletteFormat::Act, "p.act"),
            (PaletteFormat::Jasc, "p.pal"),
        ] {
            export_palette(&pal, "grey", f, &tmp.path().join("p")).expect("palette");
            assert!(tmp.path().join(n).is_file(), "{n}");
        }
        export_sprite_file(&sprite, SpriteFormat::Dc6, &pal, &tmp.path().join("s")).expect("dc6");
        let bytes = std::fs::read(tmp.path().join("s.dc6")).expect("read");
        assert_eq!(
            Sprite::decode(&bytes, SpriteFormat::Dc6)
                .expect("decode")
                .frames_per_direction(),
            2
        );
    }

    #[test]
    fn save_to_game_writes_saves_changed_and_snapshot() {
        let tmp = tempfile::tempdir().expect("tmp");
        let paths = Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        let mut img = IndexedImage::new(2, 2, 0, -2);
        img.pixels = vec![5, 6, 7, 8];
        img.alpha = vec![255; 4];
        let sprite = Sprite {
            directions: vec![vec![img]],
        };
        let game = GamePath::new("data\\global\\items\\invtest.dcc");
        let report = save_to_game(&paths, "p", &game, &sprite, &grey_palette()).expect("save");
        let back = Sprite::decode(
            &std::fs::read(&report.path).expect("read"),
            SpriteFormat::Dcc,
        )
        .expect("decode");
        assert_eq!(back, sprite);
        assert_eq!(report.changed_total, 1);
        assert!(report.snapshot.is_some());
        assert!(report.path.starts_with(&paths.saves));
        let bad = GamePath::new("data\\global\\items\\invtest.png");
        assert!(save_to_game(&paths, "p", &bad, &sprite, &grey_palette()).is_err());
    }

    #[test]
    fn batch_runs_and_reports() {
        let tmp = tempfile::tempdir().expect("tmp");
        let input = tmp.path().join("in");
        std::fs::create_dir_all(&input).expect("mkdir");
        let mut img = IndexedImage::new(2, 2, 0, -2);
        img.pixels = vec![5, 6, 7, 8];
        img.alpha = vec![255; 4];
        let sprite = Sprite {
            directions: vec![vec![img]],
        };
        std::fs::write(
            input.join("a.dc6"),
            sprite.encode(SpriteFormat::Dc6).expect("encode"),
        )
        .expect("write");
        let settings = BatchSettings {
            input: Some(input),
            output: Some(tmp.path().join("out")),
            ..BatchSettings::default()
        };
        let task = BatchTask::spawn(&settings, grey_palette()).expect("spawn");
        let report = loop {
            if let Some(r) = task.poll() {
                break r.expect("batch ok");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(report.converted, 1, "{report:?}");
        assert!(report_lines(&report)[0].contains("1 converted"));
        assert!(BatchTask::spawn(&BatchSettings::default(), grey_palette()).is_err());
    }
}
