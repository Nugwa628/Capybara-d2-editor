//! Moving sprites to and from image files: a folder of per-frame PNGs with a
//! `sprite.json` (exact offsets), and a single sprite-sheet PNG.
//!
//! Offsets in `sprite.json` are those of [`IndexedImage`]: the position of
//! the PNG's top-left pixel relative to the anchor, y down. An empty (0x0)
//! frame is exported as a one-pixel transparent PNG, since PNG cannot be
//! empty, and comes back as that one transparent pixel.
//!
//! A sheet is one PNG with a row per direction and a column per frame;
//! every cell is the size of the sprite's union box ([`Sprite::bounds`]).
//! Its [`ExportMeta`] holds, for every cell, the **cell origin** (the union
//! box's top-left) as the offset, so [`import_sheet`] gives back full cells
//! at the original anchor position (frames come back padded to the cell
//! size, which renders identically).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use crate::error::{Result, SpriteError};
use crate::image_io::{ImageKind, export_png, import_image};
use crate::model::{RgbaFrame, Sprite, SpriteFormat};
use crate::ops::center_offset;
use crate::quantize::{QuantizeOptions, quantize, to_rgba};

/// Name of the metadata file written next to exported frames.
pub const META_FILE: &str = "sprite.json";
/// Current `sprite.json` version.
pub const META_VERSION: u32 = 1;
/// Largest sheet, in pixels.
const MAX_SHEET_PIXELS: u64 = 1 << 28;

/// One exported frame.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrameMeta {
    pub dir: u32,
    pub frame: u32,
    /// File name inside the export folder.
    pub file: String,
    pub offset_x: i32,
    pub offset_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportMeta {
    pub version: u32,
    pub format: SpriteFormat,
    pub directions: u32,
    pub frames_per_direction: u32,
    /// Name of the palette the PNGs were rendered with (informational).
    pub palette: String,
    /// The file or game path the sprite came from, if known.
    pub source: Option<String>,
    pub frames: Vec<FrameMeta>,
}

pub(crate) fn frame_file_name(dir: usize, frame: usize) -> String {
    format!("d{dir:02}_f{frame:03}.png")
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|e| SpriteError::io(path, e))
}

fn check_shape(sprite: &Sprite) -> Result<()> {
    let per = sprite.frames_per_direction();
    match sprite.directions.iter().position(|d| d.len() != per) {
        Some(d) => Err(SpriteError::invalid(format!(
            "direction {d} has a different number of frames than direction 0"
        ))),
        None => Ok(()),
    }
}

/// Writes `d{dir:02}_f{frame:03}.png` for every frame plus `sprite.json`
/// into `dir` (created when missing) and returns the metadata.
pub fn export_frames(
    sprite: &Sprite,
    format: SpriteFormat,
    palette: &Palette,
    palette_name: &str,
    source: Option<&str>,
    dir: &Path,
) -> Result<ExportMeta> {
    check_shape(sprite)?;
    std::fs::create_dir_all(dir).map_err(|e| SpriteError::io(dir, e))?;
    let mut frames = Vec::new();
    for (d, images) in sprite.directions.iter().enumerate() {
        for (f, img) in images.iter().enumerate() {
            let rgba = if img.width == 0 || img.height == 0 {
                RgbaFrame::new(1, 1, img.offset_x, img.offset_y)
            } else {
                to_rgba(img, palette)
            };
            let file = frame_file_name(d, f);
            write_file(&dir.join(&file), &export_png(&rgba)?)?;
            frames.push(FrameMeta {
                dir: d as u32,
                frame: f as u32,
                file,
                offset_x: img.offset_x,
                offset_y: img.offset_y,
            });
        }
    }
    let meta = ExportMeta {
        version: META_VERSION,
        format,
        directions: sprite.directions.len() as u32,
        frames_per_direction: sprite.frames_per_direction() as u32,
        palette: palette_name.to_owned(),
        source: source.map(str::to_owned),
        frames,
    };
    let json = serde_json::to_vec_pretty(&meta).map_err(|e| SpriteError::invalid(e.to_string()))?;
    write_file(&dir.join(META_FILE), &json)?;
    Ok(meta)
}

/// Natural order: digit runs compare as numbers (`f2` before `f10`).
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        digits.push(c);
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let ord = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| da.len().cmp(&db.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Every readable image file directly inside `dir`, in natural name order.
pub(crate) fn list_images(dir: &Path) -> Result<Vec<(PathBuf, ImageKind)>> {
    let read = std::fs::read_dir(dir).map_err(|e| SpriteError::io(dir, e))?;
    let mut out: Vec<(PathBuf, ImageKind)> = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| SpriteError::io(dir, e))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let kind = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageKind::from_extension);
        if let Some(kind) = kind {
            out.push((path, kind));
        }
    }
    out.sort_by(|a, b| {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
        natural_cmp(
            &name(&a.0).unwrap_or_default(),
            &name(&b.0).unwrap_or_default(),
        )
    });
    Ok(out)
}

pub(crate) fn load_quantized(
    path: &Path,
    kind: ImageKind,
    palette: &Palette,
    opts: &QuantizeOptions,
) -> Result<IndexedImage> {
    let bytes = std::fs::read(path).map_err(|e| SpriteError::io(path, e))?;
    let rgba = import_image(&bytes, kind).map_err(|e| match e {
        SpriteError::Image(m) => SpriteError::Image(format!("{}: {m}", path.display())),
        other => other,
    })?;
    Ok(quantize(&rgba, palette, opts))
}

/// Reads a `sprite.json` exported folder when `dir` has one (every frame
/// gets its exact offset back), otherwise all images in the folder in
/// natural name order as one direction with [`center_offset`] anchors.
///
/// With a `sprite.json` the frames are known exports that already carry
/// alpha, so colour keying is off for that import:
/// [`QuantizeOptions::transparent_rgb`] is treated as `None` and opaque black
/// stays opaque (it maps to the nearest non-zero index when
/// `protect_transparent` is on). Transparency then comes from alpha only.
/// Without a `sprite.json`, `opts` applies unchanged.
pub fn import_frames(
    dir: &Path,
    palette: &Palette,
    opts: &QuantizeOptions,
) -> Result<(Sprite, Option<ExportMeta>)> {
    let meta_path = dir.join(META_FILE);
    if meta_path.is_file() {
        let json = std::fs::read(&meta_path).map_err(|e| SpriteError::io(&meta_path, e))?;
        let meta: ExportMeta = serde_json::from_slice(&json)
            .map_err(|e| SpriteError::invalid(format!("{}: {e}", meta_path.display())))?;
        let no_key = QuantizeOptions {
            transparent_rgb: None,
            ..opts.clone()
        };
        let sprite = sprite_from_meta(dir, &meta, palette, &no_key)?;
        return Ok((sprite, Some(meta)));
    }
    let files = list_images(dir)?;
    if files.is_empty() {
        return Err(SpriteError::invalid(format!(
            "{} holds no sprite.json and no PNG/BMP/GIF/PCX images",
            dir.display()
        )));
    }
    let mut frames = Vec::with_capacity(files.len());
    for (path, kind) in files {
        let mut img = load_quantized(&path, kind, palette, opts)?;
        center_offset(&mut img);
        frames.push(img);
    }
    Ok((
        Sprite {
            directions: vec![frames],
        },
        None,
    ))
}

fn sprite_from_meta(
    dir: &Path,
    meta: &ExportMeta,
    palette: &Palette,
    opts: &QuantizeOptions,
) -> Result<Sprite> {
    let (dirs, per) = (meta.directions as usize, meta.frames_per_direction as usize);
    if dirs.saturating_mul(per) != meta.frames.len() || dirs > 256 || per > 4096 {
        return Err(SpriteError::invalid(format!(
            "sprite.json lists {} frames for {dirs} directions x {per} frames",
            meta.frames.len()
        )));
    }
    let mut grid: Vec<Vec<Option<IndexedImage>>> = vec![vec![None; per]; dirs];
    for fm in &meta.frames {
        // a file name only: sprite.json must not point outside its folder
        let name = Path::new(&fm.file);
        if name.file_name().map(Path::new) != Some(name) || fm.file.contains(['/', '\\']) {
            return Err(SpriteError::invalid(format!(
                "sprite.json frame file {:?} is not a plain file name",
                fm.file
            )));
        }
        let kind = name
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageKind::from_extension)
            .unwrap_or(ImageKind::Png);
        let slot = grid
            .get_mut(fm.dir as usize)
            .and_then(|d| d.get_mut(fm.frame as usize))
            .ok_or_else(|| {
                SpriteError::invalid(format!(
                    "sprite.json frame {}/{} is out of range",
                    fm.dir, fm.frame
                ))
            })?;
        if slot.is_some() {
            return Err(SpriteError::invalid(format!(
                "sprite.json lists frame {}/{} twice",
                fm.dir, fm.frame
            )));
        }
        let mut img = load_quantized(&dir.join(name), kind, palette, opts)?;
        img.offset_x = fm.offset_x;
        img.offset_y = fm.offset_y;
        *slot = Some(img);
    }
    let directions = grid
        .into_iter()
        .map(|d| {
            d.into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| SpriteError::invalid("sprite.json is missing a frame"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Sprite { directions })
}

/// One PNG: a row per direction, a column per frame, every cell the size of
/// the union box. See the module docs for the returned metadata.
pub fn export_sheet(sprite: &Sprite, palette: &Palette) -> Result<(Vec<u8>, ExportMeta)> {
    check_shape(sprite)?;
    let (min_x, min_y, max_x, max_y) = sprite
        .bounds()
        .ok_or_else(|| SpriteError::invalid("the sprite has no pixels to put on a sheet"))?;
    let (cell_w, cell_h) = ((max_x - min_x) as u32, (max_y - min_y) as u32);
    let (dirs, per) = (
        sprite.directions.len() as u32,
        sprite.frames_per_direction() as u32,
    );
    let (sheet_w, sheet_h) = (
        u64::from(cell_w) * u64::from(per),
        u64::from(cell_h) * u64::from(dirs),
    );
    if sheet_w * sheet_h > MAX_SHEET_PIXELS
        || sheet_w > u64::from(u32::MAX)
        || sheet_h > u64::from(u32::MAX)
    {
        return Err(SpriteError::invalid("the sheet would be too large"));
    }
    let mut sheet = RgbaFrame::new(sheet_w as u32, sheet_h as u32, min_x, min_y);
    let mut frames = Vec::new();
    for (r, images) in sprite.directions.iter().enumerate() {
        for (c, img) in images.iter().enumerate() {
            let rgba = to_rgba(img, palette);
            let ox = c as u32 * cell_w + (img.offset_x - min_x) as u32;
            let oy = r as u32 * cell_h + (img.offset_y - min_y) as u32;
            for y in 0..img.height {
                for x in 0..img.width {
                    if let Some(px) = rgba.get(x, y) {
                        sheet.put(ox + x, oy + y, px);
                    }
                }
            }
            frames.push(FrameMeta {
                dir: r as u32,
                frame: c as u32,
                file: String::new(),
                offset_x: min_x,
                offset_y: min_y,
            });
        }
    }
    let meta = ExportMeta {
        version: META_VERSION,
        format: SpriteFormat::Dc6,
        directions: dirs,
        frames_per_direction: per,
        palette: String::new(),
        source: None,
        frames,
    };
    Ok((export_png(&sheet)?, meta))
}

/// Cuts a sheet PNG into `directions x frames` equal cells and quantizes
/// them. Cell offsets come from `meta` when it has the cell; otherwise each
/// cell gets [`center_offset`]. With `meta` (a sheet from [`export_sheet`]),
/// colour keying is turned off (`transparent_rgb = None`) so opaque black
/// stays opaque, as [`import_frames`] does when `sprite.json` is present.
pub fn import_sheet(
    png: &[u8],
    directions: u32,
    frames: u32,
    palette: &Palette,
    opts: &QuantizeOptions,
    meta: Option<&ExportMeta>,
) -> Result<Sprite> {
    if directions == 0 || frames == 0 {
        return Err(SpriteError::invalid(
            "a sheet needs at least one row and column",
        ));
    }
    let sheet = import_image(png, ImageKind::Png)?;
    if sheet.width % frames != 0 || sheet.height % directions != 0 {
        return Err(SpriteError::invalid(format!(
            "a {}x{} sheet cannot be cut into {directions} rows x {frames} columns",
            sheet.width, sheet.height
        )));
    }
    let (cw, ch) = (sheet.width / frames, sheet.height / directions);
    // A sheet with export metadata came from `export_sheet`, whose alpha is
    // exact: key nothing by colour, as `import_frames` does with sprite.json.
    let no_key;
    let opts = if meta.is_some() {
        no_key = QuantizeOptions {
            transparent_rgb: None,
            ..opts.clone()
        };
        &no_key
    } else {
        opts
    };
    let mut out = Vec::with_capacity(directions as usize);
    for r in 0..directions {
        let mut row = Vec::with_capacity(frames as usize);
        for c in 0..frames {
            let mut cell = RgbaFrame::new(cw, ch, 0, 0);
            for y in 0..ch {
                for x in 0..cw {
                    if let Some(px) = sheet.get(c * cw + x, r * ch + y) {
                        cell.put(x, y, px);
                    }
                }
            }
            let mut img = quantize(&cell, palette, opts);
            let known = meta.and_then(|m| m.frames.iter().find(|f| f.dir == r && f.frame == c));
            match known {
                Some(fm) => {
                    img.offset_x = fm.offset_x;
                    img.offset_y = fm.offset_y;
                }
                None => center_offset(&mut img),
            }
            row.push(img);
        }
        out.push(row);
    }
    Ok(Sprite { directions: out })
}
