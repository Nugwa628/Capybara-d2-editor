//! The tile pipeline's seam.
//!
//! Everything that turns pixels into index columns is here and needs no game
//! files: colour, texture, transparency, pHash, mirrored pHash and the 96-px
//! PNG thumbnail ([`record_from_rgba`]). The DT1 half (parse a DT1, render
//! each tile with the act palette, and resolve `used_in` from DS1 cells) is
//! [`run`], which `build` calls through the [`TileRun`] function type so tests
//! can plug in synthetic tiles.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use d2_assets::{GamePath, LayerId, Overlay};
use d2_formats::FormatError;
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::dt1::{Dt1, TileKey};
use d2_formats::palette::{Palette, act_palette_path};
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, GrayImage, ImageEncoder, RgbaImage};
use image_hasher::{HashAlg, HasherConfig};

use crate::error::Result;
use crate::naming::{
    class_name, color_name, descriptor, generated_tile_name, luma as luma_of, texture_class,
};
use crate::par::par_map;

/// Longest side of a stored thumbnail.
pub(crate) const THUMB_MAX_SIDE: u32 = 96;

/// One DS1 as the tile pipeline needs it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ds1Usage {
    pub path: String,
    /// DT1 files this map's tiles resolve against, in load order. Empty
    /// means "unknown": any DT1 holding the key counts.
    pub dt1_order: Vec<GamePath>,
    /// `((orientation, main, sub), cells)`, sorted by key.
    pub keys: Vec<((u8, u8, u8), u32)>,
}

/// One DT1 file to expand into tile assets.
#[derive(Clone, Debug)]
pub(crate) struct Dt1Input {
    pub path: GamePath,
    pub layer: LayerId,
    /// `Name` of the lowest-id LvlType that loads it, or a path-derived place.
    pub place: String,
    /// 0-based act whose palette renders the tiles (the Level `Pal` rule of
    /// the first map that loads this DT1; see `build.rs`).
    pub act: Option<u8>,
}

pub(crate) struct TileCtx<'a> {
    pub overlay: &'a Overlay,
    pub dt1s: &'a [Dt1Input],
    pub ds1s: &'a [Ds1Usage],
    pub cancel: &'a AtomicBool,
}

/// A finished tile, ready to insert.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TileRecord {
    /// `<dt1 path>#<orientation>/<main>/<sub>/<n>`.
    pub key: String,
    pub orientation: u8,
    pub main: u8,
    pub sub: u8,
    pub variant: u32,
    pub rarity: i32,
    pub width: u32,
    pub height: u32,
    pub class: String,
    pub gen_name: String,
    pub color: Option<String>,
    pub color_rgb: Option<u32>,
    pub texture: Option<String>,
    pub transparency: f32,
    pub phash: Option<i64>,
    pub phash_mirror: Option<i64>,
    pub thumb: Option<Vec<u8>>,
}

/// All tiles of one DT1.
pub(crate) struct TileBatch {
    pub dt1: GamePath,
    pub tiles: Vec<TileRecord>,
    /// Problems worth a warning (a tile that would not render).
    pub warnings: Vec<String>,
}

/// DT1 path to its tile keys, each with the asset keys of its variants.
pub(crate) type TileKeys = HashMap<GamePath, HashMap<(u8, u8, u8), Vec<String>>>;

/// A DS1 using a tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsedInRow {
    pub tile_key: String,
    pub ds1: String,
    pub cells: u32,
}

pub(crate) type Sink<'a> = &'a mut dyn FnMut(TileBatch) -> Result<()>;
pub(crate) type Progress<'a> = &'a mut dyn FnMut(u64, u64, &str) -> Result<()>;
pub(crate) type TileRun = fn(&TileCtx<'_>, Progress<'_>, Sink<'_>) -> Result<Vec<UsedInRow>>;

/// DT1 files processed per parallel batch (bounds memory held at once).
const CHUNK: usize = 16;

fn gray_palette() -> Palette {
    let mut colors = [[0u8; 3]; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        let v = u8::try_from(i).unwrap_or(u8::MAX);
        *c = [v, v, v];
    }
    Palette { colors }
}

/// Act palettes 0..=4 from the overlay; a missing one borrows the first that
/// exists (or a grey ramp when none does).
fn load_palettes(overlay: &Overlay) -> Vec<Palette> {
    let found: Vec<Option<Palette>> = (0..5u8)
        .map(|act| {
            let path = GamePath::new(act_palette_path(act));
            overlay
                .read_bytes(&path)
                .ok()
                .flatten()
                .and_then(|bytes| Palette::parse_dat(&bytes).ok())
        })
        .collect();
    let fallback = found
        .iter()
        .flatten()
        .next()
        .cloned()
        .unwrap_or_else(gray_palette);
    found
        .into_iter()
        .map(|p| p.unwrap_or_else(|| fallback.clone()))
        .collect()
}

fn process_dt1(overlay: &Overlay, input: &Dt1Input, palettes: &[Palette]) -> TileBatch {
    let mut batch = TileBatch {
        dt1: input.path.clone(),
        tiles: Vec::new(),
        warnings: Vec::new(),
    };
    let name = input.path.as_str();
    let bytes = match overlay.read_from(input.layer, &input.path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            batch
                .warnings
                .push(format!("{name}: file vanished while indexing"));
            return batch;
        }
        Err(e) => {
            batch.warnings.push(format!("{name}: {e}"));
            return batch;
        }
    };
    let dt1 = match Dt1::parse(&bytes) {
        Ok(dt1) => dt1,
        Err(FormatError::UnsupportedVersion { version, .. }) => {
            batch.warnings.push(format!(
                "{name}: unsupported DT1 version {version}, no tiles indexed"
            ));
            return batch;
        }
        Err(e) => {
            batch.warnings.push(format!("{name}: {e}"));
            return batch;
        }
    };
    let Some(palette) = palettes.get(usize::from(input.act.unwrap_or(0).min(4))) else {
        batch.warnings.push(format!("{name}: no palette"));
        return batch;
    };
    let mut seen: HashMap<TileKey, u32> = HashMap::new();
    let (mut bad_key, mut bad_render) = (0u32, 0u32);
    for tile in &dt1.tiles {
        let Some(key) = tile.key() else {
            bad_key += 1;
            continue;
        };
        let counter = seen.entry(key).or_insert(0);
        let variant = *counter;
        *counter += 1;
        let Ok(image) = tile.render() else {
            bad_render += 1;
            continue;
        };
        let rgba = image.to_rgba(palette);
        let origin = TileOrigin {
            dt1_path: name,
            place: &input.place,
            orientation: key.orientation,
            main: key.main,
            sub: key.sub,
            variant,
            rarity: tile.rarity,
        };
        batch
            .tiles
            .push(record_from_rgba(&origin, &rgba, image.width, image.height));
    }
    if bad_key > 0 {
        batch.warnings.push(format!(
            "{name}: {bad_key} tiles with an index outside 0-255 skipped"
        ));
    }
    if bad_render > 0 {
        batch
            .warnings
            .push(format!("{name}: {bad_render} tiles could not be rendered"));
    }
    batch
}

// ---------------------------------------------------------------- sprites

/// What the index records about a DC6, DCC or COF file.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SpriteRecord {
    /// Union box over every direction and frame (DC6, DCC).
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub directions: Option<u32>,
    /// Frames per direction.
    pub frames: Option<u32>,
    /// COF only.
    pub layers: Option<u32>,
    /// PNG of frame 0 of direction 0 (DC6, DCC).
    pub thumb: Option<Vec<u8>>,
    pub warning: Option<String>,
}

/// Units and most sprites use the Act 1 palette.
pub(crate) fn sprite_palette(overlay: &Overlay) -> Palette {
    load_palettes(overlay)
        .into_iter()
        .next()
        .unwrap_or_else(gray_palette)
}

fn union_box(boxes: impl Iterator<Item = (i64, i64, i64, i64)>) -> Option<(u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    let mut any = false;
    for (bx0, by0, bx1, by1) in boxes {
        any = true;
        x0 = x0.min(bx0);
        y0 = y0.min(by0);
        x1 = x1.max(bx1);
        y1 = y1.max(by1);
    }
    any.then(|| {
        (
            u32::try_from(x1 - x0).unwrap_or(u32::MAX),
            u32::try_from(y1 - y0).unwrap_or(u32::MAX),
        )
    })
}

fn thumb_of(image: &d2_formats::IndexedImage, palette: &Palette) -> Option<Vec<u8>> {
    if image.width == 0 || image.height == 0 {
        return None;
    }
    thumbnail_png(&image.to_rgba(palette), image.width, image.height)
}

/// Decode one sprite file for the index. A file that does not parse is
/// reported through `warning` and leaves the other fields empty.
pub(crate) fn process_sprite(
    overlay: &Overlay,
    path: &GamePath,
    layer: LayerId,
    ext: &str,
    palette: &Palette,
) -> SpriteRecord {
    let name = path.as_str();
    let bytes = match overlay.read_from(layer, path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return SpriteRecord {
                warning: Some(format!("{name}: file vanished while indexing")),
                ..SpriteRecord::default()
            };
        }
        Err(e) => {
            return SpriteRecord {
                warning: Some(format!("{name}: {e}")),
                ..SpriteRecord::default()
            };
        }
    };
    let failed = |e: FormatError| SpriteRecord {
        warning: Some(format!("{name}: {e}")),
        ..SpriteRecord::default()
    };
    match ext {
        "dc6" => match Dc6::parse(&bytes) {
            Ok(dc6) => {
                let size = union_box(dc6.frames.iter().map(|f| {
                    let (x, bottom) = (i64::from(f.offset_x), i64::from(f.offset_y));
                    (
                        x,
                        bottom - i64::from(f.height) + 1,
                        x + i64::from(f.width),
                        bottom + 1,
                    )
                }));
                let thumb = dc6
                    .frame(0, 0)
                    .and_then(|f| f.render().ok())
                    .and_then(|img| thumb_of(&img, palette));
                SpriteRecord {
                    width: size.map(|s| s.0),
                    height: size.map(|s| s.1),
                    directions: Some(dc6.directions),
                    frames: Some(dc6.frames_per_direction),
                    thumb,
                    ..SpriteRecord::default()
                }
            }
            Err(e) => failed(e),
        },
        "dcc" => match Dcc::parse(&bytes) {
            Ok(dcc) => {
                let size = union_box(dcc.directions.iter().map(|d| {
                    let b = d.bbox;
                    (
                        i64::from(b.x),
                        i64::from(b.y),
                        i64::from(b.x) + i64::from(b.width),
                        i64::from(b.y) + i64::from(b.height),
                    )
                }));
                let thumb = dcc
                    .directions
                    .first()
                    .and_then(|d| d.frames.first())
                    .and_then(|img| thumb_of(img, palette));
                SpriteRecord {
                    width: size.map(|s| s.0),
                    height: size.map(|s| s.1),
                    directions: u32::try_from(dcc.directions.len()).ok(),
                    frames: Some(dcc.frames_per_direction),
                    thumb,
                    ..SpriteRecord::default()
                }
            }
            Err(e) => failed(e),
        },
        _ => match Cof::parse(&bytes) {
            Ok(cof) => SpriteRecord {
                directions: Some(u32::from(cof.directions)),
                frames: Some(u32::from(cof.frames_per_direction)),
                layers: u32::try_from(cof.layers.len()).ok(),
                ..SpriteRecord::default()
            },
            Err(e) => failed(e),
        },
    }
}

/// The real pipeline: parse each DT1, render every tile with its act palette,
/// stream the finished tiles to `sink`, then work out which DS1s use which tile.
pub(crate) fn run(
    ctx: &TileCtx<'_>,
    progress: Progress<'_>,
    sink: Sink<'_>,
) -> Result<Vec<UsedInRow>> {
    let palettes = load_palettes(ctx.overlay);
    let total = ctx.dt1s.len() as u64;
    let mut done_before = 0u64;
    let mut tiles_by_dt1: TileKeys = HashMap::new();
    for chunk in ctx.dt1s.chunks(CHUNK) {
        let current = chunk.first().map_or("", |c| c.path.as_str()).to_owned();
        let batches = par_map(
            chunk,
            ctx.cancel,
            |input| process_dt1(ctx.overlay, input, &palettes),
            |d| progress(done_before + d as u64, total, &current),
        )?;
        done_before += chunk.len() as u64;
        for batch in batches {
            let keys = tiles_by_dt1.entry(batch.dt1.clone()).or_default();
            for t in &batch.tiles {
                keys.entry((t.orientation, t.main, t.sub))
                    .or_default()
                    .push(t.key.clone());
            }
            sink(batch)?;
        }
    }
    progress(total, total, "")?;
    // TODO(presets): `d2_assets::presets::MapCatalog` will replace the
    // LvlPrest-based DT1 lists in `Ds1Usage::dt1_order`.
    Ok(resolve_used_in(ctx.ds1s, &tiles_by_dt1))
}

// ---------------------------------------------------------------- pixels

/// What the pixels of a tile say about it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PixelStats {
    pub mean_rgb: [u8; 3],
    /// Mean Sobel gradient magnitude of the luma of the opaque pixels.
    pub edge_density: f32,
    /// Fraction of fully transparent pixels, 0..=1.
    pub transparency: f32,
}

fn luma(r: u8, g: u8, b: u8) -> f32 {
    0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)
}

/// Statistics of an RGBA image (`rgba.len() == w * h * 4`). `None` when the
/// image is empty or has no opaque pixel. A pixel is opaque when alpha >= 128.
///
/// The gradient treats a transparent or out-of-image neighbour as equal to
/// the pixel itself, so the silhouette of a diamond tile is not an "edge".
pub(crate) fn analyze(rgba: &[u8], w: u32, h: u32) -> Option<PixelStats> {
    let (w, h) = (w as usize, h as usize);
    let expected = w.checked_mul(h)?.checked_mul(4)?;
    if w == 0 || h == 0 || rgba.len() < expected {
        return None;
    }
    // Luma plane with a one-pixel NaN border; transparent pixels are NaN too.
    let pw = w + 2;
    let mut lum = vec![f32::NAN; pw * (h + 2)];
    let mut sum = [0u64; 3];
    let mut opaque = 0u64;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let p = &rgba[i..i + 4];
            if p[3] < 128 {
                continue;
            }
            opaque += 1;
            for c in 0..3 {
                sum[c] += u64::from(p[c]);
            }
            lum[(y + 1) * pw + x + 1] = luma(p[0], p[1], p[2]);
        }
    }
    if opaque == 0 {
        return None;
    }
    let mut edge = 0f64;
    for y in 1..=h {
        for x in 1..=w {
            let centre = lum[y * pw + x];
            if centre.is_nan() {
                continue;
            }
            // Missing neighbours count as equal to the pixel itself.
            let l = |dx: usize, dy: usize| -> f32 {
                let v = lum[(y + dy - 1) * pw + x + dx - 1];
                if v.is_nan() { centre } else { v }
            };
            let gx = (l(2, 0) + 2.0 * l(2, 1) + l(2, 2)) - (l(0, 0) + 2.0 * l(0, 1) + l(0, 2));
            let gy = (l(0, 2) + 2.0 * l(1, 2) + l(2, 2)) - (l(0, 0) + 2.0 * l(1, 0) + l(2, 0));
            edge += f64::from((gx * gx + gy * gy).sqrt());
        }
    }
    let mean = |c: usize| u8::try_from((sum[c] + opaque / 2) / opaque).unwrap_or(u8::MAX);
    let total = (w * h) as f64;
    #[allow(clippy::cast_possible_truncation)]
    Some(PixelStats {
        mean_rgb: [mean(0), mean(1), mean(2)],
        edge_density: (edge / opaque as f64) as f32,
        transparency: ((total - opaque as f64) / total) as f32,
    })
}

/// Grey image for hashing at full resolution (so a mirrored copy hashes
/// exactly like the mirror of the original). Transparent pixels take `fill`.
fn gray_for_hash(rgba: &[u8], w: u32, h: u32, fill: [u8; 3]) -> Option<GrayImage> {
    let expected = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if w == 0 || h == 0 || rgba.len() < expected {
        return None;
    }
    let fill_luma = luma(fill[0], fill[1], fill[2]);
    let pixels: Vec<u8> = rgba[..expected]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let l = if p[3] >= 128 {
                luma(p[0], p[1], p[2])
            } else {
                fill_luma
            };
            l.round().clamp(0.0, 255.0) as u8
        })
        .collect();
    GrayImage::from_raw(w, h, pixels)
}

/// Longest side of the grey image that is hashed.
const HASH_SIDE: u32 = 32;

/// Box-filter `gray` down to at most [`HASH_SIDE`] per side (hashing the
/// result is much cheaper than letting the hasher resize a 160-px tile).
fn shrink(gray: &GrayImage) -> GrayImage {
    let (w, h) = gray.dimensions();
    let (tw, th) = (w.min(HASH_SIDE), h.min(HASH_SIDE));
    if (tw, th) == (w, h) {
        return gray.clone();
    }
    let mut acc = vec![0u32; (tw * th) as usize];
    let mut count = vec![0u32; (tw * th) as usize];
    for (x, y, p) in gray.enumerate_pixels() {
        let tx = (u64::from(x) * u64::from(tw) / u64::from(w)) as u32;
        let ty = (u64::from(y) * u64::from(th) / u64::from(h)) as u32;
        let t = (ty * tw + tx) as usize;
        acc[t] += u32::from(p.0[0]);
        count[t] += 1;
    }
    let pixels: Vec<u8> = acc
        .iter()
        .zip(&count)
        .map(|(sum, n)| u8::try_from((sum + n / 2) / (*n).max(1)).unwrap_or(u8::MAX))
        .collect();
    GrayImage::from_raw(tw, th, pixels).unwrap_or_else(|| gray.clone())
}

thread_local! {
    static HASHER: image_hasher::Hasher = HasherConfig::new()
        .hash_size(8, 8)
        .hash_alg(HashAlg::Median)
        .preproc_dct()
        .to_hasher();
}

fn hash_u64(img: &GrayImage) -> i64 {
    let hash = HASHER.with(|hasher| hasher.hash_image(img));
    let mut bytes = [0u8; 8];
    for (dst, src) in bytes.iter_mut().zip(hash.as_bytes()) {
        *dst = *src;
    }
    i64::from_be_bytes(bytes)
}

/// pHash of the tile and of its horizontal mirror. Transparent pixels take
/// the mean colour so the tile's silhouette does not dominate the hash.
pub(crate) fn phash_pair(rgba: &[u8], w: u32, h: u32, fill: [u8; 3]) -> Option<(i64, i64)> {
    let gray = gray_for_hash(rgba, w, h, fill)?;
    // The mirror is flipped at full resolution before shrinking, so a tile
    // that is a pixel-exact mirrored copy gets exactly this hash as its own.
    let mirrored = imageops::flip_horizontal(&gray);
    Some((hash_u64(&shrink(&gray)), hash_u64(&shrink(&mirrored))))
}

/// PNG thumbnail with the longest side at most [`THUMB_MAX_SIDE`]; smaller
/// images are kept at their size.
pub(crate) fn thumbnail_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let expected = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if w == 0 || h == 0 || rgba.len() < expected {
        return None;
    }
    let img = RgbaImage::from_raw(w, h, rgba[..expected].to_vec())?;
    let longest = w.max(h);
    let img = if longest > THUMB_MAX_SIDE {
        let scale = f64::from(THUMB_MAX_SIDE) / f64::from(longest);
        let nw = ((f64::from(w) * scale).round() as u32).max(1);
        let nh = ((f64::from(h) * scale).round() as u32).max(1);
        imageops::resize(&img, nw, nh, FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, PngFilter::Adaptive)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

/// Where a tile came from.
pub(crate) struct TileOrigin<'a> {
    pub dt1_path: &'a str,
    pub place: &'a str,
    pub orientation: u8,
    pub main: u8,
    pub sub: u8,
    /// Index among tiles with the same key in this DT1, in file order.
    pub variant: u32,
    pub rarity: i32,
}

/// Turn a rendered tile (RGBA, already palette-mapped) into its index row.
pub(crate) fn record_from_rgba(origin: &TileOrigin<'_>, rgba: &[u8], w: u32, h: u32) -> TileRecord {
    let stats = analyze(rgba, w, h);
    let (color, color_rgb, texture, transparency) = match stats {
        Some(s) => (
            Some(color_name(s.mean_rgb).to_owned()),
            Some(
                (u32::from(s.mean_rgb[0]) << 16)
                    | (u32::from(s.mean_rgb[1]) << 8)
                    | u32::from(s.mean_rgb[2]),
            ),
            Some(texture_class(s.edge_density).to_owned()),
            s.transparency,
        ),
        None => (None, None, None, 1.0),
    };
    let desc = match (&color, &texture) {
        (Some(c), Some(t)) => descriptor(c, t, stats.map_or(0.0, |s| luma_of(s.mean_rgb))),
        _ => "empty".to_owned(),
    };
    let hashes = stats.and_then(|s| phash_pair(rgba, w, h, s.mean_rgb));
    TileRecord {
        key: format!(
            "{}#{}/{}/{}/{}",
            origin.dt1_path, origin.orientation, origin.main, origin.sub, origin.variant
        ),
        orientation: origin.orientation,
        main: origin.main,
        sub: origin.sub,
        variant: origin.variant,
        rarity: origin.rarity,
        width: w,
        height: h,
        class: class_name(origin.orientation).to_owned(),
        gen_name: generated_tile_name(
            origin.place,
            origin.orientation,
            &desc,
            origin.main,
            origin.sub,
        ),
        color,
        color_rgb,
        texture,
        transparency,
        phash: hashes.map(|h| h.0),
        phash_mirror: hashes.map(|h| h.1),
        thumb: thumbnail_png(rgba, w, h),
    }
}

/// Resolve which tile assets each DS1 uses.
///
/// `tiles_by_dt1` maps a DT1 path to the keys it holds, each with the asset
/// keys of its variants. A cell key resolves against the DS1's own DT1 list
/// (first DT1 in load order that has the key); a DS1 with no list falls back
/// to every DT1 that has the key. All variants of the key get the cell count.
pub(crate) fn resolve_used_in(ds1s: &[Ds1Usage], tiles_by_dt1: &TileKeys) -> Vec<UsedInRow> {
    let mut rows = Vec::new();
    let mut every_dt1: Vec<&GamePath> = tiles_by_dt1.keys().collect();
    every_dt1.sort();
    for ds1 in ds1s {
        for (key, cells) in &ds1.keys {
            let owners: Vec<&GamePath> = if ds1.dt1_order.is_empty() {
                every_dt1.clone()
            } else {
                ds1.dt1_order.iter().collect()
            };
            if ds1.dt1_order.is_empty() {
                for dt1 in owners {
                    if let Some(variants) = tiles_by_dt1.get(dt1).and_then(|m| m.get(key)) {
                        for tile_key in variants {
                            rows.push(UsedInRow {
                                tile_key: tile_key.clone(),
                                ds1: ds1.path.clone(),
                                cells: *cells,
                            });
                        }
                    }
                }
            } else if let Some(variants) = owners
                .into_iter()
                .find_map(|dt1| tiles_by_dt1.get(dt1).and_then(|m| m.get(key)))
            {
                for tile_key in variants {
                    rows.push(UsedInRow {
                        tile_key: tile_key.clone(),
                        ds1: ds1.path.clone(),
                        cells: *cells,
                    });
                }
            }
        }
    }
    rows
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        (0..w * h).flat_map(|_| rgba).collect()
    }

    /// Deterministic per-pixel noise around a base colour.
    fn noisy(w: u32, h: u32, base: [u8; 3], amp: u32) -> Vec<u8> {
        let mut seed = 12345u32;
        let mut out = Vec::new();
        for _ in 0..w * h {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = ((seed >> 24) % (amp * 2 + 1)) as i32 - amp as i32;
            for c in base {
                out.push((i32::from(c) + n).clamp(0, 255) as u8);
            }
            out.push(255);
        }
        out
    }

    #[test]
    fn flat_green_tile() {
        let img = solid(32, 16, [50, 150, 50, 255]);
        let s = analyze(&img, 32, 16).unwrap();
        assert_eq!(s.mean_rgb, [50, 150, 50]);
        assert!(s.edge_density < 0.001);
        assert_eq!(s.transparency, 0.0);
        assert_eq!(texture_class(s.edge_density), "flat");
        assert_eq!(color_name(s.mean_rgb), "green");
    }

    #[test]
    fn noisy_texture_is_classified_above_flat() {
        let img = noisy(32, 16, [50, 150, 50], 12);
        let s = analyze(&img, 32, 16).unwrap();
        assert!(
            s.edge_density >= crate::naming::FLAT_BELOW,
            "edge density {} should clear the flat threshold",
            s.edge_density
        );
        assert_ne!(texture_class(s.edge_density), "flat");
    }

    #[test]
    fn silhouette_is_not_an_edge_and_transparency_counts() {
        // Left half opaque grey, right half transparent.
        let (w, h) = (16u32, 8u32);
        let mut img = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                if x < w / 2 {
                    img.extend([128, 128, 128, 255]);
                } else {
                    img.extend([0, 0, 0, 0]);
                }
            }
        }
        let s = analyze(&img, w, h).unwrap();
        assert!(s.edge_density < 0.001, "{}", s.edge_density);
        assert!((s.transparency - 0.5).abs() < 1e-6);
        assert_eq!(s.mean_rgb, [128, 128, 128]);
    }

    #[test]
    fn hard_edges_are_edgy() {
        // Black/white vertical stripes, one pixel wide.
        let (w, h) = (16u32, 16u32);
        let mut img = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let v = if x % 2 == 0 { 0 } else { 255 };
                img.extend([v, v, v, 255]);
            }
        }
        let s = analyze(&img, w, h).unwrap();
        assert_eq!(texture_class(s.edge_density), "edgy");
    }

    #[test]
    fn degenerate_inputs() {
        assert!(analyze(&[], 0, 0).is_none());
        assert!(analyze(&[0, 0, 0, 0], 1, 1).is_none(), "fully transparent");
        assert!(analyze(&[0u8; 3], 1, 1).is_none(), "short buffer");
        assert!(thumbnail_png(&[], 0, 0).is_none());
        assert!(phash_pair(&[0u8; 3], 2, 2, [0, 0, 0]).is_none());
    }

    #[test]
    fn thumbnail_is_a_small_png() {
        let img = noisy(160, 80, [120, 120, 120], 30);
        let png = thumbnail_png(&img, 160, 80).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (96, 48));
        let tall = thumbnail_png(&noisy(32, 200, [1, 2, 3], 1), 32, 200).unwrap();
        let decoded = image::load_from_memory(&tall).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (15, 96));
        // Small images keep their size.
        let small = thumbnail_png(&solid(10, 6, [1, 2, 3, 255]), 10, 6).unwrap();
        let decoded = image::load_from_memory(&small).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (10, 6));
    }

    /// Left-heavy gradient: not symmetric, so a mirror differs.
    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut img = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 255) / (w - 1)) as u8;
                let v2 = (v as u32 * (y + 1) / h) as u8;
                img.extend([v, v2, 255 - v, 255]);
            }
        }
        img
    }

    fn mirror(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for y in 0..h {
            for x in (0..w).rev() {
                let i = ((y * w + x) * 4) as usize;
                out.extend_from_slice(&rgba[i..i + 4]);
            }
        }
        out
    }

    #[test]
    fn phash_of_a_mirrored_copy_is_the_mirror_hash() {
        let (w, h) = (64u32, 32u32);
        let img = gradient(w, h);
        let flipped = mirror(&img, w, h);
        let (a, a_mirror) = phash_pair(&img, w, h, [100, 100, 100]).unwrap();
        let (b, b_mirror) = phash_pair(&flipped, w, h, [100, 100, 100]).unwrap();
        assert_eq!(
            a_mirror, b,
            "mirror hash of A is the hash of the flipped copy"
        );
        assert_eq!(b_mirror, a);
        assert_ne!(a, a_mirror, "a gradient is not mirror-symmetric");
        // Deterministic.
        assert_eq!(phash_pair(&img, w, h, [100, 100, 100]).unwrap().0, a);
    }

    #[test]
    fn similar_images_hash_close_and_different_ones_far() {
        let (w, h) = (64u32, 32u32);
        let a = noisy(w, h, [120, 120, 120], 60);
        let mut b = a.clone();
        for p in b.as_chunks_mut::<4>().0.iter_mut().step_by(37) {
            p[0] = p[0].saturating_add(6);
        }
        let other = gradient(w, h);
        let ha = phash_pair(&a, w, h, [0, 0, 0]).unwrap().0;
        let hb = phash_pair(&b, w, h, [0, 0, 0]).unwrap().0;
        let ho = phash_pair(&other, w, h, [0, 0, 0]).unwrap().0;
        let d = |x: i64, y: i64| (x ^ y).count_ones();
        assert!(d(ha, hb) <= 4, "{}", d(ha, hb));
        assert!(d(ha, ho) > 10, "{}", d(ha, ho));
    }

    #[test]
    fn record_columns() {
        let img = noisy(32, 16, [50, 150, 50], 14);
        let origin = TileOrigin {
            dt1_path: "data\\global\\tiles\\act1\\town\\floor.dt1",
            place: "Act1 Town",
            orientation: 0,
            main: 5,
            sub: 2,
            variant: 1,
            rarity: 3,
        };
        let r = record_from_rgba(&origin, &img, 32, 16);
        assert_eq!(r.key, "data\\global\\tiles\\act1\\town\\floor.dt1#0/5/2/1");
        assert_eq!(r.class, "Floor");
        assert_eq!(r.color.as_deref(), Some("green"));
        assert_eq!(r.color_rgb.map(|c| c >> 16), Some(50));
        assert!(
            r.gen_name.starts_with("Act1 Town · Floor · "),
            "{}",
            r.gen_name
        );
        assert!(r.gen_name.ends_with(" · 05/02"), "{}", r.gen_name);
        assert!(r.phash.is_some() && r.phash_mirror.is_some());
        assert!(r.thumb.is_some());
        assert_eq!((r.width, r.height, r.rarity, r.variant), (32, 16, 3, 1));
        assert_eq!(r.transparency, 0.0);
    }

    #[test]
    fn empty_tile_record() {
        let img = solid(8, 8, [0, 0, 0, 0]);
        let origin = TileOrigin {
            dt1_path: "a.dt1",
            place: "P",
            orientation: 13,
            main: 0,
            sub: 0,
            variant: 0,
            rarity: 0,
        };
        let r = record_from_rgba(&origin, &img, 8, 8);
        assert_eq!(r.gen_name, "P · Shadow · empty · 00/00");
        assert_eq!((r.color.clone(), r.texture.clone()), (None, None));
        assert_eq!(r.transparency, 1.0);
        assert_eq!(r.phash, None);
    }

    #[test]
    fn used_in_resolution() {
        let a = GamePath::new("a.dt1");
        let b = GamePath::new("b.dt1");
        let mut tiles: TileKeys = HashMap::new();
        tiles.entry(a.clone()).or_default().insert(
            (0, 1, 0),
            vec!["a.dt1#0/1/0/0".into(), "a.dt1#0/1/0/1".into()],
        );
        tiles
            .entry(b.clone())
            .or_default()
            .insert((0, 1, 0), vec!["b.dt1#0/1/0/0".into()]);
        tiles
            .entry(b.clone())
            .or_default()
            .insert((1, 2, 0), vec!["b.dt1#1/2/0/0".into()]);
        let ds1s = vec![
            // Known list: first DT1 with the key wins, all its variants count.
            Ds1Usage {
                path: "m1.ds1".into(),
                dt1_order: vec![a.clone(), b.clone()],
                keys: vec![((0, 1, 0), 7), ((1, 2, 0), 3), ((9, 9, 9), 1)],
            },
            // Unknown list: every DT1 with the key.
            Ds1Usage {
                path: "m2.ds1".into(),
                dt1_order: vec![],
                keys: vec![((0, 1, 0), 2)],
            },
        ];
        let rows = resolve_used_in(&ds1s, &tiles);
        let got: Vec<(String, String, u32)> = rows
            .into_iter()
            .map(|r| (r.tile_key, r.ds1, r.cells))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a.dt1#0/1/0/0".into(), "m1.ds1".into(), 7),
                ("a.dt1#0/1/0/1".into(), "m1.ds1".into(), 7),
                ("b.dt1#1/2/0/0".into(), "m1.ds1".into(), 3),
                ("a.dt1#0/1/0/0".into(), "m2.ds1".into(), 2),
                ("a.dt1#0/1/0/1".into(), "m2.ds1".into(), 2),
                ("b.dt1#0/1/0/0".into(), "m2.ds1".into(), 2),
            ]
        );
    }

    #[test]
    fn huge_dimensions_with_small_buffers_are_rejected() {
        let buf = vec![255u8; 64];
        for (w, h) in [
            (u32::MAX, u32::MAX),
            (u32::MAX, 1),
            (1, u32::MAX),
            (1 << 16, 1 << 16),
            (0, u32::MAX),
            (u32::MAX, 0),
        ] {
            assert!(analyze(&buf, w, h).is_none(), "{w}x{h}");
            assert!(phash_pair(&buf, w, h, [0, 0, 0]).is_none(), "{w}x{h}");
            assert!(thumbnail_png(&buf, w, h).is_none(), "{w}x{h}");
            let origin = TileOrigin {
                dt1_path: "a.dt1",
                place: "P",
                orientation: 0,
                main: 0,
                sub: 0,
                variant: u32::MAX,
                rarity: i32::MIN,
            };
            let r = record_from_rgba(&origin, &buf, w, h);
            assert_eq!((r.phash, r.thumb), (None, None));
        }
    }

    proptest::proptest! {
        /// Tile images arrive with file-derived sizes; the buffer may be
        /// shorter or longer than `w * h * 4`.
        #[test]
        fn pixel_pipeline_never_panics(
            w in proptest::prop_oneof![0u32..80, proptest::num::u32::ANY],
            h in proptest::prop_oneof![0u32..80, proptest::num::u32::ANY],
            extra in -64i64..64,
            seed in proptest::num::u32::ANY,
            fill in proptest::array::uniform3(proptest::num::u8::ANY),
        ) {
            // A buffer near `w * h * 4` when that is small, else a tiny one.
            let exact = u64::from(w).checked_mul(u64::from(h)).and_then(|n| n.checked_mul(4));
            let len = if let Some(exact) = exact.filter(|&n| n <= 80 * 80 * 4) {
                usize::try_from((exact as i64 + extra).max(0)).unwrap_or(0)
            } else {
                usize::try_from(extra.unsigned_abs()).unwrap_or(0)
            };
            let mut s = seed;
            let buf: Vec<u8> = (0..len)
                .map(|_| {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (s >> 24) as u8
                })
                .collect();
            let stats = analyze(&buf, w, h);
            if let Some(st) = stats {
                proptest::prop_assert!((0.0..=1.0).contains(&st.transparency));
            }
            let _ = phash_pair(&buf, w, h, fill);
            let _ = thumbnail_png(&buf, w, h);
            let origin = TileOrigin {
                dt1_path: "x.dt1",
                place: "P",
                orientation: (seed % 256) as u8,
                main: 1,
                sub: 2,
                variant: seed,
                rarity: 0,
            };
            let r = record_from_rgba(&origin, &buf, w, h);
            proptest::prop_assert_eq!((r.width, r.height), (w, h));
        }

        #[test]
        fn shrink_handles_any_small_image(w in 0u32..200, h in 0u32..200, v in proptest::num::u8::ANY) {
            let img = GrayImage::from_pixel(w, h, image::Luma([v]));
            let small = shrink(&img);
            proptest::prop_assert!(small.width() <= HASH_SIDE && small.height() <= HASH_SIDE);
        }

        #[test]
        fn union_box_is_total(
            boxes in proptest::collection::vec(
                (proptest::num::i32::ANY, proptest::num::i32::ANY, proptest::num::u32::ANY, proptest::num::u32::ANY),
                0..8,
            )
        ) {
            let _ = union_box(boxes.iter().map(|&(x, y, w, h)| {
                (
                    i64::from(x),
                    i64::from(y) - i64::from(h) + 1,
                    i64::from(x) + i64::from(w),
                    i64::from(y) + 1,
                )
            }));
        }
    }
}
