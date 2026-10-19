//! Batch conversion of directory trees, in both directions.
//!
//! * [`BatchDirection::ToPng`]: every `.dc6` / `.dcc` under the input becomes
//!   a folder (named like the file without its extension) of frame PNGs plus
//!   `sprite.json`, at the mirrored path under the output. When a `.dc6` and
//!   a `.dcc` share a stem in one folder, the second keeps its extension in
//!   the folder name (`name.dcc`).
//! * [`BatchDirection::ToSprite`]: every folder that holds a `sprite.json`
//!   becomes one sprite file (`<folder>.dc6` / `.dcc`), restoring the exact
//!   offsets; every other PNG/BMP/GIF/PCX file becomes a one-frame sprite
//!   anchored at its bottom centre. With [`BatchOptions::split`] such a lone
//!   image is cut into tiles of that size, one frame per tile.
//!
//! With `recursive` off only the input folder itself is read (plus, for
//! `ToSprite`, its direct sub-folders that hold a `sprite.json`). Nothing is
//! ever written inside the input folder: an output folder inside it is
//! refused, and any single target that would land in it is reported as a
//! failure instead.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use d2_formats::palette::Palette;

use crate::error::{Result, SpriteError};
use crate::image_io::ImageKind;
use crate::model::{Sprite, SpriteFormat};
use crate::ops::{center_offset, split_frame};
use crate::quantize::QuantizeOptions;
use crate::transfer::{META_FILE, export_frames, import_frames, load_quantized, natural_cmp};

#[derive(Clone, Debug, PartialEq)]
pub enum BatchDirection {
    ToSprite(SpriteFormat),
    ToPng,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchOptions {
    pub recursive: bool,
    pub quantize: QuantizeOptions,
    /// `ToSprite` only: cut lone images into tiles of this size.
    pub split: Option<(u32, u32)>,
    /// Replace targets that already exist (otherwise they are skipped).
    pub overwrite: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchReport {
    pub converted: usize,
    pub skipped: usize,
    pub failed: Vec<(PathBuf, String)>,
    pub log: Vec<String>,
}

enum Item {
    SpriteFile {
        path: PathBuf,
        rel: PathBuf,
        format: SpriteFormat,
    },
    SpriteFolder {
        dir: PathBuf,
        rel: PathBuf,
    },
    LoneImage {
        path: PathBuf,
        rel: PathBuf,
        kind: ImageKind,
    },
}

impl Item {
    fn source(&self) -> &Path {
        match self {
            Item::SpriteFile { path, .. } | Item::LoneImage { path, .. } => path,
            Item::SpriteFolder { dir, .. } => dir,
        }
    }

    fn rel(&self) -> &Path {
        match self {
            Item::SpriteFile { rel, .. }
            | Item::SpriteFolder { rel, .. }
            | Item::LoneImage { rel, .. } => rel,
        }
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `path` made absolute and with `.`/`..` resolved lexically after its
/// deepest existing ancestor has been canonicalised (the path itself need
/// not exist).
fn resolve(path: &Path) -> Result<PathBuf> {
    let abs = std::path::absolute(path).map_err(|e| SpriteError::io(path, e))?;
    let mut existing = abs.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut out = existing
        .canonicalize()
        .map_err(|e| SpriteError::io(&existing, e))?;
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    Ok(out)
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let read = std::fs::read_dir(dir).map_err(|e| SpriteError::io(dir, e))?;
    let mut out: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    out.sort_by(|a, b| {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
        natural_cmp(&name(a).unwrap_or_default(), &name(b).unwrap_or_default())
    });
    Ok(out)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn scan(
    dir: &Path,
    rel: &Path,
    depth: usize,
    recursive: bool,
    to_png: bool,
    items: &mut Vec<Item>,
) -> Result<()> {
    let entries = sorted_entries(dir)?;
    let is_sprite_folder = !to_png && dir.join(META_FILE).is_file();
    if is_sprite_folder {
        items.push(Item::SpriteFolder {
            dir: dir.to_path_buf(),
            rel: rel.to_path_buf(),
        });
    }
    for path in entries {
        let Some(name) = path.file_name() else {
            continue;
        };
        let child_rel = rel.join(name);
        if is_real_dir(&path) {
            let sprite_child = !to_png && path.join(META_FILE).is_file();
            if recursive || (depth == 0 && sprite_child) {
                scan(&path, &child_rel, depth + 1, recursive, to_png, items)?;
            }
            continue;
        }
        if !path.is_file() || is_sprite_folder {
            continue; // the images of a sprite folder belong to its sprite
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if to_png {
            if let Some(format) = SpriteFormat::from_extension(ext) {
                items.push(Item::SpriteFile {
                    path,
                    rel: child_rel,
                    format,
                });
            }
        } else if let Some(kind) = ImageKind::from_extension(ext) {
            items.push(Item::LoneImage {
                path,
                rel: child_rel,
                kind,
            });
        }
    }
    Ok(())
}

/// Converts a tree; see the module docs. `progress(done, total, name)` is
/// called once with `done == 0` and then after every item. `cancel` is
/// checked before every item: when set, the call returns
/// [`SpriteError::Cancelled`] and whatever was already written stays.
pub fn batch_convert(
    input: &Path,
    output: &Path,
    direction: &BatchDirection,
    palette: &Palette,
    opts: &BatchOptions,
    progress: &mut dyn FnMut(usize, usize, &str),
    cancel: &AtomicBool,
) -> Result<BatchReport> {
    if !input.is_dir() {
        return Err(SpriteError::invalid(format!(
            "{} is not a folder",
            input.display()
        )));
    }
    let input_abs = input
        .canonicalize()
        .map_err(|e| SpriteError::io(input, e))?;
    let output_abs = resolve(output)?;
    if output_abs.starts_with(&input_abs) {
        return Err(SpriteError::invalid(
            "the output folder must not be inside the input folder",
        ));
    }
    let to_png = matches!(direction, BatchDirection::ToPng);
    let mut items = Vec::new();
    scan(
        &input_abs,
        Path::new(""),
        0,
        opts.recursive,
        to_png,
        &mut items,
    )?;
    let total = items.len();
    let mut report = BatchReport::default();
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    progress(0, total, "");
    for (n, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(SpriteError::Cancelled);
        }
        let name = text(item.rel());
        let outcome = convert_one(
            item,
            &input_abs,
            &output_abs,
            direction,
            palette,
            opts,
            &mut claimed,
        );
        match outcome {
            Ok(Done::Converted(msg)) => {
                report.converted += 1;
                report.log.push(format!("{name}: {msg}"));
            }
            Ok(Done::Skipped(msg)) => {
                report.skipped += 1;
                report.log.push(format!("{name}: skipped, {msg}"));
            }
            Err(e) => {
                report.log.push(format!("{name}: FAILED, {e}"));
                report
                    .failed
                    .push((item.source().to_path_buf(), e.to_string()));
            }
        }
        progress(n + 1, total, &name);
    }
    Ok(report)
}

enum Done {
    Converted(String),
    Skipped(String),
}

/// A safe relative path built only from normal components.
fn clean_rel(rel: &Path) -> PathBuf {
    rel.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect()
}

fn convert_one(
    item: &Item,
    input_abs: &Path,
    output_abs: &Path,
    direction: &BatchDirection,
    palette: &Palette,
    opts: &BatchOptions,
    claimed: &mut HashSet<PathBuf>,
) -> Result<Done> {
    let rel = clean_rel(item.rel());
    let parent = rel.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem_of = |p: &Path| -> String {
        let n = p.file_name().or_else(|| input_abs.file_name());
        let n = n
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match p.extension() {
            Some(_) if !matches!(item, Item::SpriteFolder { .. }) => Path::new(&n)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(n),
            _ => n,
        }
    };
    let stem = if rel.as_os_str().is_empty() {
        "sprite".to_owned()
    } else {
        stem_of(&rel)
    };
    match (direction, item) {
        (BatchDirection::ToPng, Item::SpriteFile { path, format, .. }) => {
            let mut folder = output_abs.join(&parent).join(&stem);
            if claimed.contains(&folder) {
                folder = output_abs
                    .join(&parent)
                    .join(format!("{stem}.{}", format.extension()));
            }
            guard(&folder, input_abs, claimed)?;
            if folder.join(META_FILE).exists() && !opts.overwrite {
                return Ok(Done::Skipped(format!("{} exists", text(&folder))));
            }
            let bytes = std::fs::read(path).map_err(|e| SpriteError::io(path, e))?;
            let sprite = Sprite::decode(&bytes, *format)?;
            export_frames(&sprite, *format, palette, "", Some(&text(&rel)), &folder)?;
            Ok(Done::Converted(format!(
                "{} directions x {} frames -> {}",
                sprite.directions.len(),
                sprite.frames_per_direction(),
                text(folder.strip_prefix(output_abs).unwrap_or(&folder))
            )))
        }
        (BatchDirection::ToSprite(format), item) => {
            let file = output_abs
                .join(&parent)
                .join(format!("{stem}.{}", format.extension()));
            guard(&file, input_abs, claimed)?;
            if file.exists() && !opts.overwrite {
                return Ok(Done::Skipped(format!("{} exists", text(&file))));
            }
            let sprite = match item {
                Item::SpriteFolder { dir, .. } => import_frames(dir, palette, &opts.quantize)?.0,
                Item::LoneImage { path, kind, .. } => {
                    let mut img = load_quantized(path, *kind, palette, &opts.quantize)?;
                    center_offset(&mut img);
                    let frames = match opts.split {
                        Some((w, h)) => split_frame(&img, w, h),
                        None => vec![img],
                    };
                    Sprite {
                        directions: vec![frames],
                    }
                }
                Item::SpriteFile { .. } => {
                    return Err(SpriteError::invalid("a sprite file in a to-sprite batch"));
                }
            };
            let bytes = sprite.encode_with_palette(*format, palette)?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| SpriteError::io(dir, e))?;
            }
            std::fs::write(&file, &bytes).map_err(|e| SpriteError::io(&file, e))?;
            Ok(Done::Converted(format!(
                "{} directions x {} frames -> {} ({} bytes)",
                sprite.directions.len(),
                sprite.frames_per_direction(),
                text(file.strip_prefix(output_abs).unwrap_or(&file)),
                bytes.len()
            )))
        }
        (BatchDirection::ToPng, _) => Err(SpriteError::invalid("an image file in a to-PNG batch")),
    }
}

/// Refuses targets inside the input folder and targets already produced by
/// another input in this run; records the target.
fn guard(target: &Path, input_abs: &Path, claimed: &mut HashSet<PathBuf>) -> Result<()> {
    if target.starts_with(input_abs) {
        return Err(SpriteError::invalid(format!(
            "{} is inside the input folder",
            target.display()
        )));
    }
    if !claimed.insert(target.to_path_buf()) {
        return Err(SpriteError::invalid(format!(
            "{} is already produced by another input",
            target.display()
        )));
    }
    Ok(())
}
