//! Corpus round trips: sprite, PNG export, import with `sprite.json`
//! offsets, encode, decode.

use std::path::PathBuf;

use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use crate::image_io::{ImageKind, export_png, import_image};
use crate::model::{RgbaFrame, Sprite, SpriteFormat};
use crate::quantize::{QuantizeOptions, quantize, to_rgba};
use crate::testutil::{act1_palette, extracted, find_files, parallel, sample};
use crate::transfer::{export_frames, export_sheet, import_frames, import_sheet};

/// Keeps every palette colour (no colour keying), protects index 0.
fn exact() -> QuantizeOptions {
    QuantizeOptions {
        transparent_rgb: None,
        ..QuantizeOptions::default()
    }
}

fn corpus(ext: &str, every: usize) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for p in sample(find_files(&root, ext), every) {
            let name = format!("{profile}:{}", p.strip_prefix(&root).unwrap().display());
            out.push((name, p));
        }
    }
    out
}

/// Frames compared as the decoders return them; a 0x0 frame comes back from
/// a PNG export as one transparent pixel, which counts as equal.
fn same_sprite(a: &Sprite, b: &Sprite) -> Result<(), String> {
    if a.directions.len() != b.directions.len()
        || a.frames_per_direction() != b.frames_per_direction()
    {
        return Err("shape changed".to_owned());
    }
    for (d, (da, db)) in a.directions.iter().zip(&b.directions).enumerate() {
        for (f, (x, y)) in da.iter().zip(db).enumerate() {
            let x_empty = x.width == 0 || x.height == 0;
            let ok = if x_empty {
                y.alpha.iter().all(|&v| v == 0)
                    && (x.offset_x, x.offset_y) == (y.offset_x, y.offset_y)
            } else {
                x == y
            };
            if !ok {
                return Err(format!(
                    "frame {d}/{f} differs ({}x{} at {},{} vs {}x{} at {},{})",
                    x.width,
                    x.height,
                    x.offset_x,
                    x.offset_y,
                    y.width,
                    y.height,
                    y.offset_x,
                    y.offset_y
                ));
            }
        }
    }
    Ok(())
}

/// The whole conversion in memory: PNG per frame, import, encode, decode.
fn in_memory_round_trip(
    original: &Sprite,
    format: SpriteFormat,
    pal: &Palette,
) -> Result<Sprite, String> {
    let mut dirs = Vec::new();
    for dir in &original.directions {
        let mut frames = Vec::new();
        for img in dir {
            let rgba = if img.width == 0 || img.height == 0 {
                RgbaFrame::new(1, 1, 0, 0)
            } else {
                to_rgba(img, pal)
            };
            let png = export_png(&rgba).map_err(|e| e.to_string())?;
            let back = import_image(&png, ImageKind::Png).map_err(|e| e.to_string())?;
            let mut q = quantize(&back, pal, &exact());
            q.offset_x = img.offset_x;
            q.offset_y = img.offset_y;
            frames.push(q);
        }
        dirs.push(frames);
    }
    let sprite = Sprite { directions: dirs };
    let bytes = sprite
        .encode_with_palette(format, pal)
        .map_err(|e| e.to_string())?;
    Sprite::decode(&bytes, format).map_err(|e| e.to_string())
}

fn run_corpus(ext: &str, format: SpriteFormat, every: usize) {
    let Some(pal) = act1_palette() else {
        println!("skipped: no corpus");
        return;
    };
    let files = corpus(ext, every);
    let results = parallel(&files, |(name, path)| {
        let bytes = std::fs::read(path).unwrap();
        let original = match Sprite::decode(&bytes, format) {
            Ok(s) => s,
            Err(e) => return (0usize, Err(format!("{name}: decode: {e}"))),
        };
        let frames = original.directions.iter().map(Vec::len).sum();
        let r = in_memory_round_trip(&original, format, &pal)
            .and_then(|back| same_sprite(&original, &back))
            .map_err(|e| format!("{name}: {e}"));
        (frames, r)
    });
    let sprites = results.len();
    let frames: usize = results.iter().map(|r| r.0).sum();
    let failures: Vec<&String> = results.iter().filter_map(|r| r.1.as_ref().err()).collect();
    println!(
        "{ext} png round trip: {sprites} sprites, {frames} frames, {} exact, {} failures",
        sprites - failures.len(),
        failures.len()
    );
    for f in failures.iter().take(15) {
        println!("  {f}");
    }
    assert!(
        sprites >= 20 || files.len() < 20,
        "sample too small: {sprites}"
    );
    assert!(
        failures.is_empty(),
        "{} failures, first: {:?}",
        failures.len(),
        failures.first()
    );
}

#[test]
fn dc6_png_round_trip_is_exact() {
    run_corpus("dc6", SpriteFormat::Dc6, 20);
}

#[test]
fn dcc_png_round_trip_is_exact() {
    run_corpus("dcc", SpriteFormat::Dcc, 150);
}

/// The real thing, through the file system: export_frames, import_frames with
/// `sprite.json`, encode, decode, for at least 20 sprites of each format.
fn run_fs(ext: &str, format: SpriteFormat, wanted: usize) {
    let Some(pal) = act1_palette() else {
        println!("skipped: no corpus");
        return;
    };
    let mut files = corpus(ext, 1);
    files.retain(|(_, p)| std::fs::metadata(p).is_ok_and(|m| m.len() < 60_000));
    let step = (files.len() / wanted).max(1);
    let files: Vec<_> = files.into_iter().step_by(step).take(wanted).collect();
    let results = parallel(&files, |(name, path)| -> Result<usize, String> {
        let bytes = std::fs::read(path).unwrap();
        let original = Sprite::decode(&bytes, format).map_err(|e| format!("{name}: {e}"))?;
        let tmp = tempfile::tempdir().unwrap();
        let meta = export_frames(&original, format, &pal, "act1", Some(name), tmp.path())
            .map_err(|e| format!("{name}: export: {e}"))?;
        let (imported, got) = import_frames(tmp.path(), &pal, &exact())
            .map_err(|e| format!("{name}: import: {e}"))?;
        if got.as_ref() != Some(&meta) {
            return Err(format!("{name}: sprite.json differs"));
        }
        let encoded = imported
            .encode_with_palette(format, &pal)
            .map_err(|e| format!("{name}: {e}"))?;
        let decoded =
            Sprite::decode(&encoded, format).map_err(|e| format!("{name}: re-decode: {e}"))?;
        same_sprite(&original, &decoded).map_err(|e| format!("{name}: {e}"))?;
        // the sheet carries the same picture
        let (png, smeta) =
            export_sheet(&original, &pal).map_err(|e| format!("{name}: sheet: {e}"))?;
        let sheet = import_sheet(
            &png,
            original.directions.len() as u32,
            original.frames_per_direction() as u32,
            &pal,
            &exact(),
            Some(&smeta),
        )
        .map_err(|e| format!("{name}: import sheet: {e}"))?;
        check_sheet_matches(&original, &sheet).map_err(|e| format!("{name}: sheet: {e}"))?;
        Ok(original.directions.iter().map(Vec::len).sum())
    });
    let failures: Vec<&String> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    let frames: usize = results.iter().filter_map(|r| r.as_ref().ok()).sum();
    println!(
        "{ext} file round trip: {} sprites ({frames} frames), {} failures",
        results.len(),
        failures.len()
    );
    for f in failures.iter().take(10) {
        println!("  {f}");
    }
    assert!(
        results.len() >= wanted.min(20),
        "only {} sprites",
        results.len()
    );
    assert!(failures.is_empty(), "{:?}", failures.first());
}

fn visible(img: &IndexedImage) -> std::collections::BTreeMap<(i32, i32), u8> {
    let mut out = std::collections::BTreeMap::new();
    for y in 0..img.height as usize {
        for x in 0..img.width as usize {
            let i = y * img.width as usize + x;
            if img.alpha[i] != 0 && img.pixels[i] != 0 {
                out.insert(
                    (img.offset_x + x as i32, img.offset_y + y as i32),
                    img.pixels[i],
                );
            }
        }
    }
    out
}

fn check_sheet_matches(a: &Sprite, b: &Sprite) -> Result<(), String> {
    for (d, (da, db)) in a.directions.iter().zip(&b.directions).enumerate() {
        for (f, (x, y)) in da.iter().zip(db).enumerate() {
            if visible(x) != visible(y) {
                return Err(format!("frame {d}/{f} shows different pixels"));
            }
        }
    }
    Ok(())
}

#[test]
fn dc6_file_round_trip_through_sprite_json() {
    run_fs("dc6", SpriteFormat::Dc6, 30);
}

#[test]
fn dcc_file_round_trip_through_sprite_json() {
    run_fs("dcc", SpriteFormat::Dcc, 30);
}
