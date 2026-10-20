//! DT1 manager operations: duplicate, rename planning, recolour and pixel
//! edits (ds1-studio `Dt1Manager.tsx`, `dt1Edit.ts`). Pure: models in, edited
//! models and change lists out.
//!
//! Recolouring rewrites only the pixel literals of the chosen tiles' blocks
//! (control bytes, headers and every other tile stay as they were). Pixel
//! painting goes through `Block::with_pixels`, which re-encodes the block.

use d2_formats::IndexedImage;
use d2_formats::dt1::Dt1;
use d2_formats::palette::Palette;

use crate::tiles_pixels::for_each_pixel_mut;

/// The longest tile path the game holds (relative to `data/global/tiles/`).
pub const MAX_TILE_PATH: usize = 41;
/// File names are best kept this short.
pub const RECOMMENDED_NAME_LENGTH: usize = 12;

/// What changed in one tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileChange {
    /// Tile index in file order.
    pub tile: usize,
    /// Block index, when one block changed.
    pub block: Option<usize>,
    /// Pixels (or bytes) that changed.
    pub pixels: u64,
}

/// A failed manager operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dt1ManagerError {
    NoSuchTile(usize),
    NoSuchBlock {
        tile: usize,
        block: usize,
    },
    /// The re-encoded block could not be stored.
    Format(String),
    /// A name or path problem, in words.
    BadName(String),
}

impl std::fmt::Display for Dt1ManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dt1ManagerError::NoSuchTile(t) => write!(f, "the DT1 has no tile #{t}"),
            Dt1ManagerError::NoSuchBlock { tile, block } => {
                write!(f, "tile #{tile} has no sub-block #{block}")
            }
            Dt1ManagerError::Format(s) | Dt1ManagerError::BadName(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Dt1ManagerError {}

// ───────────── duplicate ─────────────

/// An independent copy of the whole DT1: equal tile for tile.
pub fn duplicate_dt1(dt1: &Dt1) -> Dt1 {
    dt1.clone()
}

/// A new DT1 holding copies of the listed tiles, in the order given.
pub fn copy_tiles(dt1: &Dt1, tiles: &[usize]) -> Result<Dt1, Dt1ManagerError> {
    let mut out = Dt1 {
        version: dt1.version,
        tiles: Vec::with_capacity(tiles.len()),
    };
    for &t in tiles {
        out.tiles.push(
            dt1.tiles
                .get(t)
                .ok_or(Dt1ManagerError::NoSuchTile(t))?
                .clone(),
        );
    }
    Ok(out)
}

/// Appends copies of the listed tiles at the end; returns their new indices.
/// (Appending keeps older indices valid, so map undo can still point at them.)
pub fn append_copies(dt1: &mut Dt1, tiles: &[usize]) -> Result<Vec<usize>, Dt1ManagerError> {
    let copies = copy_tiles(dt1, tiles)?;
    let first = dt1.tiles.len();
    dt1.tiles.extend(copies.tiles);
    Ok((first..dt1.tiles.len()).collect())
}

/// The DT1 without tile `index`; every other tile stays as it was.
pub fn without_tile(dt1: &Dt1, index: usize) -> Result<Dt1, Dt1ManagerError> {
    if index >= dt1.tiles.len() {
        return Err(Dt1ManagerError::NoSuchTile(index));
    }
    let mut out = dt1.clone();
    out.tiles.remove(index);
    Ok(out)
}

// ───────────── rename ─────────────

/// `a\B/c` -> `a/b/c`: the form paths are compared in.
pub fn normalize_path(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

/// Letters, digits and `_` only.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_reserved_device(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(n.as_str(), "con" | "prn" | "aux" | "nul")
        || ((n.starts_with("com") || n.starts_with("lpt"))
            && n.len() == 4
            && n.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
}

/// Longest name that still fits [`MAX_TILE_PATH`] in `folder` (relative to
/// `data/global/tiles`).
pub fn max_custom_name_length(folder: &str) -> usize {
    let f = folder.trim_end_matches('/');
    let dir = if f.is_empty() { 0 } else { f.len() + 1 };
    MAX_TILE_PATH.saturating_sub(dir + ".dt1".len())
}

/// Why a library name cannot be used (`None` when fine).
pub fn custom_name_problem(name: &str, folder: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() {
        return Some("Give the library a name.".into());
    }
    if !is_safe_name(n) {
        return Some("Use only letters, digits and _ (no spaces, dots, dashes or accents): other characters can break the game's tables or archives.".into());
    }
    if is_reserved_device(n) {
        return Some(format!(
            "\"{n}\" is a name Windows reserves for devices; choose another."
        ));
    }
    let max = max_custom_name_length(folder);
    if n.len() > max {
        return Some(format!(
            "At most {max} characters here: the game holds tile paths of up to {MAX_TILE_PATH} characters."
        ));
    }
    None
}

/// A planned rename of a DT1 inside `data/global/tiles/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenamePlan {
    /// Full game path before / after (`data/global/tiles/...`).
    pub from: String,
    pub to: String,
    /// The same, relative to `data/global/tiles/` (what the tables hold).
    pub from_rel: String,
    pub to_rel: String,
}

const TILES_PREFIX: &str = "data/global/tiles/";

/// Plans renaming `path` to the file name `new_stem` (no extension, same
/// folder, `.dt1` kept). Fails when the name is unsafe, too long, or the
/// path is not under `data/global/tiles/`.
pub fn plan_rename(path: &str, new_stem: &str) -> Result<RenamePlan, Dt1ManagerError> {
    let p = path.replace('\\', "/");
    let rel = p
        .get(..TILES_PREFIX.len())
        .filter(|h| h.eq_ignore_ascii_case(TILES_PREFIX))
        .and_then(|_| p.get(TILES_PREFIX.len()..))
        .ok_or_else(|| Dt1ManagerError::BadName(format!("{path} is not under {TILES_PREFIX}")))?;
    let (dir, file) = match rel.rfind('/') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    let ext = file.rfind('.').map_or(".dt1", |i| &file[i..]);
    if let Some(problem) = custom_name_problem(new_stem, dir) {
        return Err(Dt1ManagerError::BadName(problem));
    }
    let to_rel = format!("{dir}{}{ext}", new_stem.trim());
    if normalize_path(&to_rel) == normalize_path(rel) {
        return Err(Dt1ManagerError::BadName("That is the current name.".into()));
    }
    Ok(RenamePlan {
        from: format!("{TILES_PREFIX}{rel}"),
        to: format!("{TILES_PREFIX}{to_rel}"),
        from_rel: rel.to_string(),
        to_rel,
    })
}

/// Points every cell that names `old_rel` (tiles-relative, any case or slash
/// style) at `new_rel`; `0` and empty cells are skipped. Returns the indices
/// of the changed cells. For LvlTypes.txt `File n` and LvlPrest.txt `Filen`.
pub fn rename_references(cells: &mut [String], old_rel: &str, new_rel: &str) -> Vec<usize> {
    let want = normalize_path(old_rel);
    let mut changed = Vec::new();
    for (i, c) in cells.iter_mut().enumerate() {
        if c.is_empty() || c == "0" || normalize_path(c) != want {
            continue;
        }
        *c = new_rel.to_string();
        changed.push(i);
    }
    changed
}

/// A shorter tile path for one over `max` characters: the file name is cut first.
pub fn suggest_short_path(rel: &str, max: usize) -> String {
    if rel.len() <= max {
        return rel.to_string();
    }
    let (dir, file) = match rel.rfind('/') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    let base: String = stem
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect();
    if max >= dir.len() + ext.len() + 3 {
        let room = max - dir.len() - ext.len();
        return format!("{dir}{}{ext}", base.chars().take(room).collect::<String>());
    }
    let last = dir.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    let d2 = if last.is_empty() {
        String::new()
    } else {
        format!("{}/", last.chars().take(12).collect::<String>())
    };
    let room = max.saturating_sub(d2.len() + ext.len()).max(3);
    format!("{d2}{}{ext}", base.chars().take(room).collect::<String>())
}

// ───────────── recolour ─────────────

/// Replaces every pixel literal `v` by `table[v]` in the chosen tiles
/// (`None` = all). Index 0 stays 0 and a non-zero index is never mapped to 0
/// (such table entries keep the pixel). Returns one change per tile whose
/// pixels changed. Only pixel literals are rewritten.
pub fn recolor_tiles(
    dt1: &mut Dt1,
    tiles: Option<&[usize]>,
    table: &[u8; 256],
) -> Result<Vec<TileChange>, Dt1ManagerError> {
    if let Some(&t) = tiles.and_then(|ts| ts.iter().find(|&&t| t >= dt1.tiles.len())) {
        return Err(Dt1ManagerError::NoSuchTile(t));
    }
    let mut safe = *table;
    safe[0] = 0;
    for (i, v) in safe.iter_mut().enumerate().skip(1) {
        if *v == 0 {
            *v = i as u8;
        }
    }
    let mut out = Vec::new();
    for (i, tile) in dt1.tiles.iter_mut().enumerate() {
        if tiles.is_some_and(|ts| !ts.contains(&i)) {
            continue;
        }
        let mut n = 0u64;
        for block in &mut tile.blocks {
            for_each_pixel_mut(block, |v| {
                let new = safe[usize::from(*v)];
                if new != *v {
                    n += 1;
                    *v = new;
                }
            });
        }
        if n > 0 {
            out.push(TileChange {
                tile: i,
                block: None,
                pixels: n,
            });
        }
    }
    Ok(out)
}

/// Hue / brightness style recolouring options (ds1-studio `hueRemap`).
#[derive(Clone, Debug, PartialEq)]
pub struct HueOptions {
    /// Hue rotation in degrees.
    pub hue: f32,
    pub saturation: f32,
    pub brightness: f32,
    /// Tint colour and mix amount `0..=1`.
    pub tint: Option<([u8; 3], f32)>,
    /// Only snap to these palette indices (e.g. colours shared by all acts).
    pub allowed: Option<[bool; 256]>,
    /// Plain RGB distance instead of the perceptual one.
    pub rgb_metric: bool,
}

impl Default for HueOptions {
    fn default() -> Self {
        HueOptions {
            hue: 0.0,
            saturation: 1.0,
            brightness: 1.0,
            tint: None,
            allowed: None,
            rgb_metric: false,
        }
    }
}

fn nearest_index(
    pal: &Palette,
    rgb: [f32; 3],
    allowed: Option<&[bool; 256]>,
    rgb_metric: bool,
) -> u8 {
    let ok = |i: usize| allowed.is_none_or(|a| a[i]);
    let mut best = (1..256).find(|&i| ok(i)).unwrap_or(1);
    let mut best_d = f32::INFINITY;
    for i in 1..256usize {
        if !ok(i) {
            continue;
        }
        let c = pal.colors[i];
        let (pr, pg, pb) = (f32::from(c[0]), f32::from(c[1]), f32::from(c[2]));
        let (dr, dg, db) = (pr - rgb[0], pg - rgb[1], pb - rgb[2]);
        let d = if rgb_metric {
            dr * dr + dg * dg + db * db
        } else {
            let rm = (pr + rgb[0]) / 2.0;
            (2.0 + rm / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rm) / 256.0) * db * db
        };
        if d < best_d {
            best_d = d;
            best = i;
            if d == 0.0 {
                break;
            }
        }
    }
    best as u8
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let mut h = 0.0;
    if d != 0.0 {
        h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h *= 60.0;
        if h < 0.0 {
            h += 360.0;
        }
    }
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [(r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0]
}

fn clamp255(v: f32) -> f32 {
    v.round().clamp(0.0, 255.0)
}

/// A 256-entry index remap from HSV adjustments and an optional tint,
/// re-quantised to the nearest palette colour among indices 1..=255.
/// `table[0] = 0` and no other index maps to 0. With default options every
/// index whose colour is unique maps to itself.
#[allow(clippy::needless_range_loop)]
pub fn hue_remap(palette: &Palette, opts: &HueOptions) -> [u8; 256] {
    let hsv_identity = opts.hue == 0.0 && opts.saturation == 1.0 && opts.brightness == 1.0;
    let mut table = [0u8; 256];
    for i in 1..256usize {
        let c = palette.colors[i];
        let mut rgb = [f32::from(c[0]), f32::from(c[1]), f32::from(c[2])];
        if !hsv_identity {
            let (h, s, v) = rgb_to_hsv(rgb[0], rgb[1], rgb[2]);
            rgb = hsv_to_rgb(
                h + opts.hue,
                (s * opts.saturation).clamp(0.0, 1.0),
                (v * opts.brightness).clamp(0.0, 1.0),
            );
        }
        if let Some((t, amount)) = opts.tint {
            let a = amount.clamp(0.0, 1.0);
            if a > 0.0 {
                for (ch, tc) in rgb.iter_mut().zip(t) {
                    *ch += (f32::from(tc) - *ch) * a;
                }
            }
        }
        let rgb = rgb.map(clamp255);
        table[i] = nearest_index(palette, rgb, opts.allowed.as_ref(), opts.rgb_metric);
    }
    table
}

/// "Replace colour": indices within `tolerance` (RGB distance) of `from` are
/// shifted by `to - from` and re-quantised; all others map to themselves.
#[allow(clippy::needless_range_loop)]
pub fn swap_remap(
    palette: &Palette,
    from: [u8; 3],
    to: [u8; 3],
    tolerance: f32,
    allowed: Option<&[bool; 256]>,
) -> [u8; 256] {
    let mut table = [0u8; 256];
    let tol2 = tolerance * tolerance;
    for i in 1..256usize {
        let c = palette.colors[i];
        let d2: f32 = (0..3)
            .map(|k| (f32::from(c[k]) - f32::from(from[k])).powi(2))
            .sum();
        table[i] = if d2 <= tol2 {
            let rgb = [0, 1, 2]
                .map(|k| clamp255(f32::from(c[k]) + f32::from(to[k]) - f32::from(from[k])));
            nearest_index(palette, rgb, allowed, false)
        } else {
            i as u8
        };
    }
    table
}

/// Every index to itself.
pub fn identity_remap() -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = i as u8;
    }
    t
}

// ───────────── pixel edits ─────────────

/// Replaces one block's pixels with the opaque pixels of `image` (block
/// space, as `Block::render` returns it). The block keeps its position,
/// grid cell and format.
pub fn set_block_pixels(
    dt1: &mut Dt1,
    tile: usize,
    block: usize,
    image: &IndexedImage,
) -> Result<TileChange, Dt1ManagerError> {
    let t = dt1
        .tiles
        .get_mut(tile)
        .ok_or(Dt1ManagerError::NoSuchTile(tile))?;
    let b = t
        .blocks
        .get_mut(block)
        .ok_or(Dt1ManagerError::NoSuchBlock { tile, block })?;
    let before = b
        .render()
        .map_err(|e| Dt1ManagerError::Format(e.to_string()))?;
    let new = b
        .with_pixels(image)
        .map_err(|e| Dt1ManagerError::Format(e.to_string()))?;
    let after = new
        .render()
        .map_err(|e| Dt1ManagerError::Format(e.to_string()))?;
    let pixels = count_differences(&before, &after);
    *b = new;
    Ok(TileChange {
        tile,
        block: Some(block),
        pixels,
    })
}

/// Renders a block, lets `f` paint on it, and stores the result.
pub fn edit_block_pixels(
    dt1: &mut Dt1,
    tile: usize,
    block: usize,
    f: impl FnOnce(&mut IndexedImage),
) -> Result<TileChange, Dt1ManagerError> {
    let b = dt1
        .tiles
        .get(tile)
        .ok_or(Dt1ManagerError::NoSuchTile(tile))?
        .blocks
        .get(block)
        .ok_or(Dt1ManagerError::NoSuchBlock { tile, block })?;
    let mut img = b
        .render()
        .map_err(|e| Dt1ManagerError::Format(e.to_string()))?;
    f(&mut img);
    set_block_pixels(dt1, tile, block, &img)
}

/// Pixels whose opacity or index differ between two renders (images of
/// different extent are compared over their union).
fn count_differences(a: &IndexedImage, b: &IndexedImage) -> u64 {
    let at = |img: &IndexedImage, x: i64, y: i64| -> Option<u8> {
        let (c, r) = (x - i64::from(img.offset_x), y - i64::from(img.offset_y));
        if c < 0 || r < 0 || c >= i64::from(img.width) || r >= i64::from(img.height) {
            return None;
        }
        let i = r as usize * img.width as usize + c as usize;
        (*img.alpha.get(i)? != 0).then(|| img.pixels.get(i).copied())?
    };
    let x0 = i64::from(a.offset_x.min(b.offset_x));
    let y0 = i64::from(a.offset_y.min(b.offset_y));
    let x1 = (i64::from(a.offset_x) + i64::from(a.width))
        .max(i64::from(b.offset_x) + i64::from(b.width));
    let y1 = (i64::from(a.offset_y) + i64::from(a.height))
        .max(i64::from(b.offset_y) + i64::from(b.height));
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if at(a, x, y) != at(b, x, y) {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn rename_plan() {
        let p = plan_rename("data\\global\\tiles\\ACT1\\Barracks\\floor.dt1", "floor2").unwrap();
        assert_eq!(p.to_rel, "ACT1/Barracks/floor2.dt1");
        assert!(plan_rename("data/global/tiles/a/b.dt1", "bad name").is_err());
        assert!(plan_rename("data/global/tiles/a/b.dt1", "con").is_err());
        assert!(plan_rename("data/global/tiles/a/b.dt1", "b").is_err());
        assert!(plan_rename("data/other/b.dt1", "c").is_err());
        let long = "x".repeat(60);
        assert!(plan_rename("data/global/tiles/a/b.dt1", &long).is_err());
    }

    #[test]
    fn references() {
        let mut cells = vec![
            "ACT1\\Barracks\\Floor.dt1".to_string(),
            "0".into(),
            "x".into(),
        ];
        let hit = rename_references(
            &mut cells,
            "act1/barracks/floor.dt1",
            "act1/barracks/f2.dt1",
        );
        assert_eq!(hit, vec![0]);
        assert_eq!(cells[0], "act1/barracks/f2.dt1");
    }

    #[test]
    fn short_path() {
        assert_eq!(suggest_short_path("a/b.dt1", 20), "a/b.dt1");
        let s = suggest_short_path("folder/very long_name-here.dt1", 16);
        assert!(s.len() <= 16, "{s}");
        assert!(s.ends_with(".dt1"));
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn identity_hue_keeps_unique_colours() {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = [i as u8, (i * 7) as u8, (i * 13) as u8];
        }
        let pal = Palette { colors };
        let t = hue_remap(&pal, &HueOptions::default());
        for i in 1..256 {
            assert_eq!(usize::from(t[i]), i);
        }
        assert_eq!(t[0], 0);
        let shifted = hue_remap(
            &pal,
            &HueOptions {
                brightness: 0.5,
                ..HueOptions::default()
            },
        );
        assert!(shifted[200] < 200);
        assert!(shifted[1..].iter().all(|&v| v != 0));
    }
}
