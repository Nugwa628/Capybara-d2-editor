//! Entering text: the name a level shows as it is entered and on the automap.
//!
//! Two parts:
//! * the string: Levels `LevelName` and `LevelWarp` name a key of the string
//!   tables, edited here in a [`TblDoc`] (normally `patchstring.tbl`);
//! * the loading-screen image (`Levels.EntryFile` -> `data/local/ui/<lang>/
//!   expansion/<EntryFile>.dc6`) for players who don't use a mod's digital text:
//!   the line drawn with one of the game's fonts, cut into frames of at most 256
//!   pixels. [`render_entry_text`] and [`entry_dc6`] produce it.

use d2_formats::IndexedImage;
use d2_formats::dc6::Dc6;
use d2_formats::tbl::TblDoc;
use d2_formats::txt::TxtDoc;

use crate::level_tables::MAX_LEVEL_STRING;
use crate::lvl_error::{Result, refuse};
use crate::txt_util::{cell, cell_num, data_rows, row_of_record, set};

/// Where the mod's own strings go (read after string.tbl and expansionstring.tbl).
pub const PATCH_STRINGS: &str = "data/local/lng/eng/patchstring.tbl";
/// Folder of the loading-screen images.
pub const ENTRY_IMAGE_DIR: &str = "data/local/ui/eng/expansion/";
/// Frames of the entering image are at most this wide.
pub const FRAME_WIDTH: u32 = 256;

/// The edited level table and strings.
#[derive(Clone, Debug)]
pub struct EntryTextOutcome {
    pub levels: TxtDoc,
    pub strings: TblDoc,
    /// The string key LevelName and LevelWarp now name.
    pub key: String,
    pub changes: Vec<String>,
}

/// Gives level `level_id` the area name `text`.
///
/// The key is `key` when given; else the level's current `LevelName` when no
/// other level names the same key (so editing keeps the game's own key); else a
/// new key `capy_lvl_<id>`. LevelName and LevelWarp both name the key. Doing it
/// again with the same text changes nothing.
pub fn set_area_name(
    levels: &TxtDoc,
    strings: &TblDoc,
    level_id: u32,
    text: &str,
    key: Option<&str>,
) -> Result<EntryTextOutcome> {
    let text = text.trim();
    if text.is_empty() {
        return refuse("Give the level a name.");
    }
    let Some(row) = row_of_record(levels, level_id as usize) else {
        return refuse(format!("Levels.txt has no level {level_id}."));
    };
    let current = cell(levels, row, "LevelName").trim().to_owned();
    let used_elsewhere = |k: &str| {
        data_rows(levels).into_iter().any(|r| {
            r != row
                && ["LevelName", "LevelWarp"]
                    .iter()
                    .any(|c| cell(levels, r, c).trim().eq_ignore_ascii_case(k))
        })
    };
    let key = match key {
        Some(k) => k.trim().to_owned(),
        None if !current.is_empty() && !used_elsewhere(&current) => current,
        None => format!("capy_lvl_{level_id}"),
    };
    if key.is_empty() {
        return refuse("The string key is empty.");
    }
    if key.chars().count() > MAX_LEVEL_STRING {
        return refuse(format!(
            "The key \"{key}\" is longer than the game's {MAX_LEVEL_STRING}-character LevelName limit."
        ));
    }
    let mut levels = levels.clone();
    let mut strings = strings.clone();
    let mut changes = Vec::new();
    for col in ["LevelName", "LevelWarp"] {
        let from = cell(&levels, row, col);
        if set(&mut levels, row, col, &key)? {
            changes.push(format!(
                "Level {level_id}: {col} {} -> {key}",
                if from.is_empty() { "(empty)" } else { &from }
            ));
        }
    }
    let old = strings.get(&key).map(str::to_owned);
    if old.as_deref() != Some(text) {
        strings.set(&key, text)?;
        changes.push(format!(
            "String {key} = \"{text}\"{}",
            old.map_or(String::new(), |o| format!(" (was \"{o}\")"))
        ));
    }
    Ok(EntryTextOutcome {
        levels,
        strings,
        key,
        changes,
    })
}

/// Points a level's `EntryFile` (the loading-screen image name) at `name`.
/// Returns the image's game path.
pub fn set_entry_file(levels: &mut TxtDoc, level_id: u32, name: &str) -> Result<String> {
    let Some(row) = row_of_record(levels, level_id as usize) else {
        return refuse(format!("Levels.txt has no level {level_id}."));
    };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return refuse("An entry file name is letters, digits and underscores.");
    }
    set(levels, row, "EntryFile", name)?;
    Ok(format!("{ENTRY_IMAGE_DIR}{name}.dc6"))
}

/// A suggested EntryFile name: short, letters and digits (the game builds the
/// path from it), unique by `taken`.
pub fn suggest_entry_name(level_name: &str, taken: impl Fn(&str) -> bool) -> String {
    let letters: String = level_name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(10)
        .collect::<String>()
        .to_lowercase();
    let base = if letters.is_empty() {
        "entry".to_owned()
    } else {
        letters
    };
    let mut name = base.clone();
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}{n}");
        n += 1;
    }
    name
}

/// Levels whose `EntryFile` is `name` (case-insensitive), as level ids.
pub fn levels_using_entry_file(levels: &TxtDoc, name: &str) -> Vec<u32> {
    data_rows(levels)
        .into_iter()
        .filter(|&r| {
            cell(levels, r, "EntryFile")
                .trim()
                .eq_ignore_ascii_case(name)
        })
        .map(|r| cell_num(levels, r, "Id") as u32)
        .collect()
}

/// One glyph of a font.
#[derive(Clone, Debug)]
pub struct Glyph {
    pub advance: u32,
    pub image: IndexedImage,
}

/// A game font: its DC6 frames and the `.tbl` that maps characters to frames.
#[derive(Clone, Debug)]
pub struct Font {
    glyphs: std::collections::HashMap<u16, (u32, usize)>,
    frames: Vec<IndexedImage>,
    /// Height of the font's first frame.
    pub height: u32,
}

impl Font {
    /// `dc6`: `data/local/font/latin/fontNN.dc6`; `tbl`: the matching `.tbl`
    /// (`Woo!` header, 14-byte entries: code u16, advance at +3, DC6 frame u16 at +8).
    pub fn load(dc6: &Dc6, tbl: &[u8]) -> Result<Font> {
        if tbl.len() < 12 || &tbl[..4] != b"Woo!" {
            return refuse("Not a Diablo II font table (.tbl).");
        }
        let frames = dc6
            .frames
            .iter()
            .map(|f| f.render())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut glyphs = std::collections::HashMap::new();
        for e in tbl[12..].as_chunks::<14>().0 {
            let code = u16::from_le_bytes([e[0], e[1]]);
            let frame = usize::from(u16::from_le_bytes([e[8], e[9]]));
            glyphs.insert(code, (u32::from(e[3]), frame));
        }
        let height = frames.first().map_or(0, |f| f.height);
        Ok(Font {
            glyphs,
            frames,
            height,
        })
    }

    /// The glyph for a character, if the font has it.
    pub fn glyph(&self, ch: char) -> Option<Glyph> {
        let code = u16::try_from(u32::from(ch)).ok()?;
        let (advance, frame) = *self.glyphs.get(&code)?;
        Some(Glyph {
            advance,
            image: self.frames.get(frame)?.clone(),
        })
    }
}

/// The line of text as one image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryImage {
    pub width: u32,
    pub height: u32,
    /// Palette indices, 0 = transparent, top row first.
    pub pixels: Vec<u8>,
    /// Characters the font has no glyph for (drawn as nothing).
    pub missing: Vec<char>,
}

/// Draws `text` glyph after glyph by each one's advance width.
pub fn render_entry_text(
    height: u32,
    glyph: impl Fn(char) -> Option<Glyph>,
    text: &str,
) -> EntryImage {
    let mut missing = Vec::new();
    let glyphs: Vec<Option<Glyph>> = text
        .chars()
        .map(|c| {
            let g = glyph(c);
            if g.is_none() && !missing.contains(&c) {
                missing.push(c);
            }
            g
        })
        .collect();
    let width = glyphs
        .iter()
        .flatten()
        .map(|g| g.advance)
        .sum::<u32>()
        .max(1);
    let mut pixels = vec![0u8; (width * height) as usize];
    let mut x = 0u32;
    for g in glyphs.iter().flatten() {
        let im = &g.image;
        for y in 0..im.height.min(height) {
            for gx in 0..im.width {
                if x + gx >= width {
                    break;
                }
                let i = (y * im.width + gx) as usize;
                let c = im.pixels.get(i).copied().unwrap_or(0);
                let a = im.alpha.get(i).copied().unwrap_or(255);
                if c != 0 && a != 0 {
                    pixels[(y * width + x + gx) as usize] = c;
                }
            }
        }
        x += g.advance;
    }
    EntryImage {
        width,
        height,
        pixels,
        missing,
    }
}

/// The image cut into frames of [`FRAME_WIDTH`] pixels, left to right (the last narrower).
pub fn entry_frames(img: &EntryImage) -> Vec<IndexedImage> {
    let mut out = Vec::new();
    let mut x0 = 0;
    while x0 < img.width {
        let w = FRAME_WIDTH.min(img.width - x0);
        let mut f = IndexedImage::new(w, img.height, 0, 0);
        for y in 0..img.height {
            for x in 0..w {
                let p = img.pixels[(y * img.width + x0 + x) as usize];
                f.pixels[(y * w + x) as usize] = p;
                f.alpha[(y * w + x) as usize] = if p == 0 { 0 } else { 255 };
            }
        }
        out.push(f);
        x0 += FRAME_WIDTH;
    }
    out
}

/// The image as a DC6 (one direction, one frame per 256 pixels).
pub fn entry_dc6(img: &EntryImage) -> Result<Dc6> {
    Ok(Dc6::from_frames(&[entry_frames(img)])?)
}
