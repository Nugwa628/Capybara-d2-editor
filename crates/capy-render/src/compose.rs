//! The CPU compositor: draw list in, RGBA out.
//!
//! # Zoom
//!
//! DS1Edit never composites at 1:1 and shrinks the result. At `--zoom=1:N` it
//! keeps the 1:1 tile bitmaps and draws each one scaled by `1/N`
//! (`wpreview_draw_bitmap`, `preview.c:241`) at integer positions computed in
//! zoomed units (`tile_w = 160/N`, `tile_h = 80/N`, `zero_line*mul/div`
//! truncating toward zero). The scaled draw point-samples the bitmap, so a
//! destination pixel `k` of a tile whose bitmap starts at destination column
//! `ox` shows source column `N*(k - ox) + N/2`. [`render`] reproduces that
//! per item; [`downscale`] applies the same point sampling to a finished
//! image.
//!
//! # Canvas
//!
//! The canvas is the rectangle the reference computes at the start of
//! `wpreview_draw_tiles_big_screenshot` (3364): the union of every tile's
//! bitmap rectangle `[mx, mx + w/N) x [y1, y1 + h/N)`, over every non-empty
//! cell of the map regardless of layer visibility.

use d2_formats::IndexedImage;
use d2_formats::ds1::Ds1;
use d2_formats::palette::Palette;

use crate::error::{RenderError, Result};
use crate::objects::{MissingObject, ObjectOptions, ObjectSprites};
use crate::order::{
    DrawEntry, DrawItem, DrawPass, MissingTile, ObjectDrawItem, Visibility, draw_list_with_objects,
};
use crate::tileset::{TILE_H, TILE_W, TileClass, TileGeometry, TileSet};

/// Sprite blend modes keyed on COF `trans_b` (`preview.c:291-380`): 25/50/75%
/// alpha, additive, multiply, and `(ONE, INV_SRC_COLOR)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Alpha25,
    Alpha50,
    Alpha75,
    Additive,
    Multiply,
    Screen,
    /// DS1Edit's projected object shadow (`a5_draw_shadow_sprite`,
    /// `ui/compat.h:151`): the sprite as a black silhouette with coverage
    /// `1 - level/32`, so `out = dst * level / 32`. `level` is D2's darkness
    /// level, 10 for opaque layers and 28 for transparent ones
    /// (`preview.c:2259`). The source colour is ignored.
    Darken(u8),
}

impl Blend {
    /// Blends one source colour over a destination colour (both straight RGB,
    /// destination alpha `dst_a`); returns the new `[r, g, b, a]`.
    pub fn apply(self, dst: [u8; 4], src: [u8; 3]) -> [u8; 4] {
        let weight: f32 = match self {
            Blend::Normal => 1.0,
            Blend::Alpha25 => 0.25,
            Blend::Alpha50 => 0.5,
            Blend::Alpha75 => 0.75,
            Blend::Darken(level) => {
                let weight = 1.0 - f32::from(level.min(32)) / 32.0;
                return tinted(dst, [0, 0, 0], weight);
            }
            Blend::Additive => {
                return [
                    dst[0].saturating_add(src[0]),
                    dst[1].saturating_add(src[1]),
                    dst[2].saturating_add(src[2]),
                    255,
                ];
            }
            Blend::Multiply => {
                let m = |s: u8, d: u8| ((u16::from(s) * u16::from(d) + 127) / 255) as u8;
                return [m(src[0], dst[0]), m(src[1], dst[1]), m(src[2], dst[2]), 255];
            }
            Blend::Screen => {
                let m = |s: u8, d: u8| {
                    let inv = 255 - u16::from(s);
                    (u16::from(s) + (u16::from(d) * inv + 127) / 255).min(255) as u8
                };
                return [m(src[0], dst[0]), m(src[1], dst[1]), m(src[2], dst[2]), 255];
            }
        };
        if weight >= 1.0 {
            return [src[0], src[1], src[2], 255];
        }
        tinted(dst, src, weight)
    }
}

/// A draw tinted with `(w, w, w, w)` under the blender `(ALPHA, INVERSE_ALPHA)`.
///
/// The reference tints the sprite with (f, f, f, f) (`preview.c:182`)
/// but its blender is (ALPHA, INVERSE_ALPHA), so the tint scales the
/// colour as well as the coverage: out = d * (1 - f) + s * f * f. That
/// is a straight "over" of colour `s * f` with coverage `f` (measured on
/// the shadow pixels of the oracle: 0.5 d + 0.25 s). Over an opaque
/// pixel the result stays opaque; DS1Edit's own canvas drops to alpha
/// 0.75 there, a blender artefact we do not reproduce.
/// The tint is stored as a byte, so the weight is quantised: 0.5 is 128/255.
fn tinted(dst: [u8; 4], src: [u8; 3], weight: f32) -> [u8; 4] {
    let weight = (weight * 255.0 + 0.5).floor() / 255.0;
    let da = f32::from(dst[3]) / 255.0;
    let out_a = weight + da * (1.0 - weight);
    if out_a <= 0.0 {
        return dst;
    }
    let mix = |s: u8, d: u8| -> u8 {
        let v = (f32::from(s) * weight * weight + f32::from(d) * da * (1.0 - weight)) / out_a;
        v.round().clamp(0.0, 255.0) as u8
    };
    [
        mix(src[0], dst[0]),
        mix(src[1], dst[1]),
        mix(src[2], dst[2]),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// How DT1 shadow tiles are blended: DS1Edit's default shadow mode is 3,
/// "transparent" (`ds1.c:440`, `preview.c:817-830`), a 50% draw.
const SHADOW_BLEND: Blend = Blend::Alpha50;

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    /// Zoom divisor, 1..=16 (`--zoom=1:N`).
    pub zoom_div: u32,
    /// Gamma applied to the palette passed to [`render_rgba`]; 1.0 leaves it
    /// untouched, the reference default is 1.30.
    pub gamma: f32,
    pub visibility: Visibility,
    pub background: [u8; 4],
    /// Animation tick of object sprites (the editor's 25 Hz counter): the
    /// frame shown is `tick * frame_delta / 256 mod frame_count`. `None` shows
    /// frame 0. Only used by [`render_with_objects`].
    pub animation_tick: Option<u32>,
    /// User direction of object and monster sprites (0 = the editor's
    /// default), mapped through the editor's direction tables.
    pub object_direction: u8,
    /// Per-object frame pins, see [`ObjectOptions::frame_overrides`].
    pub object_frames: Option<std::sync::Arc<[Option<u8>]>>,
    /// Per-object user directions, see [`ObjectOptions::direction_overrides`].
    pub object_directions: Option<std::sync::Arc<[Option<u8>]>>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            zoom_div: 1,
            gamma: 1.30,
            visibility: Visibility::default(),
            background: [0, 0, 0, 0],
            animation_tick: None,
            object_direction: 0,
            object_frames: None,
            object_directions: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, straight (non-premultiplied)
    /// alpha.
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// An image filled with one colour.
    pub fn filled(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        let n = (width as usize).saturating_mul(height as usize);
        let mut pixels = Vec::with_capacity(n.saturating_mul(4));
        for _ in 0..n {
            pixels.extend_from_slice(&color);
        }
        RgbaImage {
            width,
            height,
            pixels,
        }
    }

    /// `None` outside the image.
    pub fn get(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        let p = self.pixels.get(i..i + 4)?;
        Some([p[0], p[1], p[2], p[3]])
    }

    pub(crate) fn set(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        if let Some(p) = self.pixels.get_mut(i..i + 4) {
            p.copy_from_slice(&color);
        }
    }

    pub fn to_png(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc
                .write_header()
                .map_err(|e| RenderError::Png(e.to_string()))?;
            writer
                .write_image_data(&self.pixels)
                .map_err(|e| RenderError::Png(e.to_string()))?;
        }
        Ok(out)
    }

    /// Decodes any 8/16-bit grey, RGB, palette or RGBA PNG to RGBA8.
    pub fn from_png(bytes: &[u8]) -> Result<RgbaImage> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| RenderError::Png(e.to_string()))?;
        let mut buf = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or_else(|| RenderError::Png("image too large".to_owned()))?
        ];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| RenderError::Png(e.to_string()))?;
        let data = buf
            .get(..info.buffer_size())
            .ok_or_else(|| RenderError::Png("short frame".to_owned()))?;
        let pixels: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => data.to_vec(),
            png::ColorType::Rgb => data
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => data
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => {
                return Err(RenderError::Png("indexed png was not expanded".to_owned()));
            }
        };
        Ok(RgbaImage {
            width: info.width,
            height: info.height,
            pixels,
        })
    }
}

/// Everything a render produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOutput {
    pub image: RgbaImage,
    pub missing: Vec<MissingTile>,
    /// Objects that drew nothing (empty for [`render`]).
    pub missing_objects: Vec<MissingObject>,
    /// Map-space position (in output pixels, i.e. already divided by the zoom)
    /// of the image's pixel (0, 0): the cell `(x, y)` has its top corner at
    /// `((x - y) * 80 / div, (x + y) * 40 / div) - origin` in the image.
    pub origin: (i32, i32),
    /// Tiles that could not be rendered (skipped).
    pub warnings: Vec<String>,
}

/// Cell top corner in zoomed map space, with the reference's integer maths
/// (`base_x = y * -tile_w / 2`, `mx = base_x + x * tile_w / 2`).
pub(crate) fn cell_corner(x: u32, y: u32, div: i32) -> (i32, i32) {
    let tile_w = TILE_W / div;
    let tile_h = TILE_H / div;
    let x = i32::try_from(x).unwrap_or(i32::MAX / 4);
    let y = i32::try_from(y).unwrap_or(i32::MAX / 4);
    let mx = (y * -tile_w) / 2 + (x * tile_w) / 2;
    let my = (y * tile_h) / 2 + (x * tile_h) / 2;
    (mx, my)
}

/// The canvas rectangle `(min_x, min_y, max_x, max_y)` (max exclusive) in
/// zoomed map space, computed like the reference's first loop
/// (`preview.c:3405-3520`). `None` when no tile has a bitmap.
pub fn reference_bounds(ds1: &Ds1, tiles: &TileSet, div: u32) -> Option<(i32, i32, i32, i32)> {
    let div = i32::try_from(div.clamp(1, 16)).unwrap_or(1);
    let tile_h = TILE_H / div;
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    let mut add = |mx: i32, my: i32, geom: &TileGeometry, wall_like: bool| {
        if geom.is_empty() {
            return;
        }
        let w = i32::try_from(geom.width).unwrap_or(0) / div;
        let h = i32::try_from(geom.height).unwrap_or(0) / div;
        let mut y1 = my - geom.zero_line / div;
        if wall_like {
            y1 += tile_h;
        }
        let (x1, x2, y2) = (mx, mx + w, y1 + h);
        bounds = Some(match bounds {
            None => (x1, y1, x2, y2),
            Some((a, b, c, d)) => (a.min(x1), b.min(y1), c.max(x2), d.max(y2)),
        });
    };
    for y in 0..ds1.height {
        for x in 0..ds1.width {
            let (mx, my) = cell_corner(x, y, div);
            // Shadows: a hidden cell counts as unknown and is skipped.
            if let Some(cell) = ds1.shadow.get(x, y)
                && !cell.is_empty()
                && !cell.hidden()
            {
                let key = d2_formats::dt1::TileKey {
                    orientation: 13,
                    main: cell.main_index(),
                    sub: cell.sub_index(),
                };
                if let Some(g) = tiles.resolve(key).and_then(|r| tiles.geometry(&r))
                    && g.class == TileClass::Shadow
                {
                    add(mx, my, &g, true);
                }
            }
            for layer in &ds1.floors {
                if let Some(cell) = layer.get(x, y) {
                    if cell.is_empty() {
                        continue;
                    }
                    let key = d2_formats::dt1::TileKey {
                        orientation: 0,
                        main: cell.main_index(),
                        sub: cell.sub_index(),
                    };
                    if let Some(g) = tiles.resolve(key).and_then(|r| tiles.geometry(&r))
                        && g.class == TileClass::Floor
                    {
                        add(mx, my, &g, false);
                    }
                }
            }
            for (i, layer) in ds1.walls.iter().enumerate() {
                let Some(cell) = layer.tiles.get(x, y) else {
                    continue;
                };
                if cell.is_empty() {
                    continue;
                }
                let Some(orientation) = ds1.orientation(i, x, y) else {
                    continue;
                };
                let key = d2_formats::dt1::TileKey {
                    orientation,
                    main: cell.main_index(),
                    sub: cell.sub_index(),
                };
                if let Some(g) = tiles.resolve(key).and_then(|r| tiles.geometry(&r))
                    && matches!(
                        g.class,
                        TileClass::UpperWall
                            | TileClass::LowerWall
                            | TileClass::Roof
                            | TileClass::Special
                    )
                {
                    // Roofs use the wall formula here, without roof_y.
                    add(mx, my, &g, true);
                }
            }
        }
    }
    bounds
}

/// Largest canvas the compositor will allocate, in bytes.
const MAX_CANVAS_BYTES: u64 = 1 << 30;

/// Composites the map and returns the image, the missing tiles, the canvas
/// origin and any per-tile warnings. `palette` is the raw act palette; the
/// gamma in `opts` is applied here.
pub fn render(
    ds1: &Ds1,
    tiles: &TileSet,
    palette: &Palette,
    opts: &RenderOptions,
) -> Result<RenderOutput> {
    render_with_objects(ds1, tiles, None, palette, opts)
}

/// [`render`] plus the objects and monsters of the DS1 when `objects` is
/// given and `opts.visibility.objects` is set. The canvas is still the
/// tiles' rectangle, as in the reference (`preview.c:3405-3520`): sprites that
/// overhang it are clipped.
pub fn render_with_objects(
    ds1: &Ds1,
    tiles: &TileSet,
    objects: Option<&ObjectSprites<'_>>,
    palette: &Palette,
    opts: &RenderOptions,
) -> Result<RenderOutput> {
    let div = opts.zoom_div.clamp(1, 16);
    let div_i = i32::try_from(div).unwrap_or(1);
    let object_opts = ObjectOptions {
        direction: opts.object_direction,
        animation_tick: opts.animation_tick,
        frame_overrides: opts.object_frames.clone(),
        direction_overrides: opts.object_directions.clone(),
    };
    let list = draw_list_with_objects(
        ds1,
        tiles,
        &opts.visibility,
        objects.map(|o| (o, &object_opts)),
    )?;
    let missing = list.missing;
    let missing_objects = list.missing_objects;
    let Some((min_x, min_y, max_x, max_y)) = reference_bounds(ds1, tiles, div) else {
        return Ok(RenderOutput {
            image: RgbaImage::filled(1, 1, opts.background),
            missing,
            missing_objects,
            origin: (0, 0),
            warnings: Vec::new(),
        });
    };
    let width = u32::try_from(max_x - min_x).unwrap_or(0);
    let height = u32::try_from(max_y - min_y).unwrap_or(0);
    if u64::from(width) * u64::from(height) * 4 > MAX_CANVAS_BYTES {
        return Err(RenderError::Other(format!(
            "canvas {width}x{height} is too large; raise the zoom divisor"
        )));
    }
    let mut canvas = RgbaImage::filled(width.max(1), height.max(1), opts.background);
    let pal = palette.with_gamma(opts.gamma);
    let mut warnings = Vec::new();
    for entry in &list.entries {
        match entry {
            DrawEntry::Tile(item) => {
                if let Err(e) = draw_item(
                    &mut canvas,
                    item,
                    tiles,
                    &pal,
                    (min_x, min_y),
                    div_i,
                    blend_for(item.pass),
                ) {
                    warnings.push(format!("{:?} at ({}, {}): {e}", item.layer, item.x, item.y));
                }
            }
            DrawEntry::Object(item) => {
                draw_object(&mut canvas, item, &pal, (min_x, min_y), div_i);
            }
        }
    }
    Ok(RenderOutput {
        image: canvas,
        missing,
        missing_objects,
        origin: (min_x, min_y),
        warnings,
    })
}

fn blend_for(pass: DrawPass) -> Blend {
    match pass {
        DrawPass::Shadows => SHADOW_BLEND,
        _ => Blend::Normal,
    }
}

fn draw_item(
    canvas: &mut RgbaImage,
    item: &DrawItem,
    tiles: &TileSet,
    pal: &Palette,
    origin: (i32, i32),
    div: i32,
    blend: Blend,
) -> Result<()> {
    let Some(geom) = tiles.geometry(&item.tile) else {
        return Ok(());
    };
    let image = tiles.image(&item.tile)?;
    let (mx, my) = cell_corner(item.x, item.y, div);
    // Destination position of the bitmap's (0, 0).
    let ox = mx - origin.0;
    let oy = my + geom.bitmap_top(div) - origin.1;
    blit(
        canvas,
        &image,
        (image.offset_x, image.offset_y),
        pal,
        ox,
        oy,
        div,
        blend,
    );
    Ok(())
}

/// Draws one object item. The sprite layers are separate bitmaps in the
/// reference (`preview.c:2091-2185`), each point-sampled at `1/div` with its
/// own truncated offset:
///
/// ```text
/// dx = dx0 - x0 + xoffset / div + lay.off_x / div
/// dy = dy0 - y0 + yoffset / div + lay.off_y / div
/// ```
///
/// (the stretch path, `stretch_sprite = YES`, the default; at 1:1 this is the
/// plain sum). `lay.off_*` is [`FrameLayer::box_origin`]. Shadow items draw
/// only the layers with a shadow level, as black silhouettes.
fn draw_object(
    canvas: &mut RgbaImage,
    item: &ObjectDrawItem,
    pal: &Palette,
    origin: (i32, i32),
    div: i32,
) {
    let (dx0, dy0) = crate::objects::object_anchor(item.x, item.y, div);
    let shadow_pass = item.pass == DrawPass::ObjectShadows;
    for layer in &item.frame.layers {
        let blend = if shadow_pass {
            match layer.shadow {
                Some(level) => Blend::Darken(level),
                None => continue,
            }
        } else {
            layer.blend
        };
        let ox = dx0 - origin.0 + item.frame.offset.0 / div + layer.box_origin.0 / div;
        let oy = dy0 - origin.1 + item.frame.offset.1 / div + layer.box_origin.1 / div;
        // The reference bitmap starts at the direction box corner, so source
        // columns are counted from there.
        let off = (
            layer.image.offset_x - layer.box_origin.0,
            layer.image.offset_y - layer.box_origin.1,
        );
        blit_sprite(canvas, &layer.image, off, pal, ox, oy, div, blend);
    }
}

/// Draws `image` (bitmap-space crop, see [`TileSet::image`]) whose bitmap
/// origin lands on canvas pixel `(ox, oy)`, point-sampling every `div`th
/// source pixel as the reference's scaled draw does.
#[expect(clippy::too_many_arguments)]
fn blit(
    canvas: &mut RgbaImage,
    image: &IndexedImage,
    offset: (i32, i32),
    pal: &Palette,
    ox: i32,
    oy: i32,
    div: i32,
    blend: Blend,
) {
    blit_pixels(canvas, image, offset, pal, ox, oy, div, blend, false);
}

/// [`blit`] for sprites: palette index 0 is transparent as well as alpha 0
/// (`compose_cof_frame`).
#[expect(clippy::too_many_arguments)]
fn blit_sprite(
    canvas: &mut RgbaImage,
    image: &IndexedImage,
    offset: (i32, i32),
    pal: &Palette,
    ox: i32,
    oy: i32,
    div: i32,
    blend: Blend,
) {
    blit_pixels(canvas, image, offset, pal, ox, oy, div, blend, true);
}

#[expect(clippy::too_many_arguments)]
fn blit_pixels(
    canvas: &mut RgbaImage,
    image: &IndexedImage,
    offset: (i32, i32),
    pal: &Palette,
    ox: i32,
    oy: i32,
    div: i32,
    blend: Blend,
    skip_index_zero: bool,
) {
    let div = div.max(1);
    let half = div / 2;
    let (w, h) = (image.width as usize, image.height as usize);
    for j in 0..h {
        let v = offset.1 + i32::try_from(j).unwrap_or(0);
        if v.rem_euclid(div) != half {
            continue;
        }
        let dy = oy + v.div_euclid(div);
        if dy < 0 {
            continue;
        }
        for i in 0..w {
            let u = offset.0 + i32::try_from(i).unwrap_or(0);
            if u.rem_euclid(div) != half {
                continue;
            }
            let dx = ox + u.div_euclid(div);
            if dx < 0 {
                continue;
            }
            let k = j * w + i;
            let alpha = image.alpha.get(k).copied().unwrap_or(0);
            if alpha == 0 {
                continue;
            }
            let index = image.pixels.get(k).copied().unwrap_or(0);
            if skip_index_zero && index == 0 {
                continue;
            }
            let rgb = pal.colors[usize::from(index)];
            let (Ok(dx), Ok(dy)) = (u32::try_from(dx), u32::try_from(dy)) else {
                continue;
            };
            let Some(dst) = canvas.get(dx, dy) else {
                continue;
            };
            canvas.set(dx, dy, blend.apply(dst, rgb));
        }
    }
}

/// Composite at the requested zoom. `palette` is the raw act palette
/// (`pal.dat`); `opts.gamma` is applied here.
pub fn render_rgba(
    ds1: &Ds1,
    tiles: &TileSet,
    palette: &Palette,
    opts: &RenderOptions,
) -> Result<(RgbaImage, Vec<MissingTile>)> {
    let out = render(ds1, tiles, palette, opts)?;
    Ok((out.image, out.missing))
}

/// Point-sampled downscale by `div`: output pixel `(i, j)` is input pixel
/// `(div*i + div/2, div*j + div/2)`, the sample a GPU takes at the centre of a
/// `1/div` scaled draw. The output is `width/div x height/div` (at least 1x1).
pub fn downscale(img: &RgbaImage, div: u32) -> RgbaImage {
    let div = div.max(1);
    if div == 1 {
        return img.clone();
    }
    let w = (img.width / div).max(1);
    let h = (img.height / div).max(1);
    let mut out = RgbaImage::filled(w, h, [0, 0, 0, 0]);
    for j in 0..h {
        for i in 0..w {
            let sx = (i * div + div / 2).min(img.width.saturating_sub(1));
            let sy = (j * div + div / 2).min(img.height.saturating_sub(1));
            if let Some(p) = img.get(sx, sy) {
                out.set(i, j, p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
