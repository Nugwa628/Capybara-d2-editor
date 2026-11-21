//! Creating brand-new DT1 tiles: tile shapes and their drawable area, building
//! a tile from a picture the way shipped tiles of that kind are split into
//! sub-blocks, image import (quantize + place), re-shaping an existing tile,
//! an empty DT1 and free tile numbers, and putting created tiles into a map's
//! own DT1 through the same flow as copied tiles ([`add_tiles_to_map`]).
//! Pure: images and DT1 models in, models out; no I/O, no UI.
//!
//! # Conventions (measured on every shipped tile, `docs/specs/dt1.md` 14)
//!
//! Pictures are in the tile's **anchor space** (the space
//! [`Tile::render`] returns, `IndexedImage::offset_x/offset_y` giving where
//! pixel (0,0) is):
//!
//! | class | orientations | drawable area | sub-blocks |
//! |---|---|---|---|
//! | floor, roof | 0, 15 | the 160x79 diamond at (0,0) | 5x5 diamonds of 32x15 at `(64 + 16(gx - gy), 8(gx + gy))`; a full cell is isometric (format 1), a partial one RLE `0x2005` (15 rows), an empty one is left out |
//! | upper wall, shadow, special | 1-14 | x 0..160, y `-height..0` (the anchor is the wall's base) | 32x32 RLE `0x1001` cells (32 rows), grid 0/0, empty cells left out |
//! | lower wall | 16-19 | x 0..160, y `-96..height - 96` | as upper walls |
//!
//! The north corner halves share one picture: orientation 4 holds x 0..80,
//! orientation 3 x 80..160 ([`build_corner_pair`]).
//!
//! Header fields: floors and roofs store height -128 and width 160; upper walls
//! store `-(height + 32)`, lower walls `-height`, both `width = max block x +
//! 32`. `direction` follows the orientation ([`direction_of`]). Default flags:
//! [`default_flags`]. Rarity 0, sound 0.

use std::collections::BTreeSet;

use capy_sprite::image_io::{ImageKind, import_image};
use capy_sprite::model::RgbaFrame;
use capy_sprite::quantize::{QuantizeOptions, quantize};
use d2_formats::IndexedImage;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::{Block, Dt1, Tile, TilePreserved};
use d2_formats::palette::Palette;
use d2_formats::txt::TxtDoc;

use crate::lvl_error::{Result, refuse};
use crate::tiles_register::{AddTilesInput, AddTilesOutcome, add_tiles_to_map};
use crate::tiles_review::{MAX_MAIN, MAX_SUB, TileId, TilePick};

/// The RLE format word of floor and roof pieces (cells the diamond only
/// partly covers).
pub const FLOOR_RLE_FORMAT: i16 = 0x2005;
/// The RLE format word of every wall, shadow and special sub-block.
pub const WALL_RLE_FORMAT: i16 = 0x1001;
/// Wall sub-blocks are this many pixels square; wall heights are multiples.
pub const WALL_BLOCK: u32 = 32;
/// Tallest wall picture accepted (63 blocks; rendered tiles stop at 2048).
pub const MAX_WALL_HEIGHT: u32 = 2016;
/// The roof height most shipped roofs use.
pub const DEFAULT_ROOF_HEIGHT: i16 = 80;
/// Rows of an RLE floor piece and of an RLE wall block, as shipped.
const FLOOR_RLE_ROWS: usize = 15;
const WALL_RLE_ROWS: usize = 32;
/// Width of a tile picture, and of the floor diamond.
const TILE_WIDTH: i32 = 160;
const FLOOR_HEIGHT: i32 = 79;
/// Stored height of every shipped floor and roof.
const FLOOR_STORED_HEIGHT: i32 = -128;
/// A lower wall's anchor lies this far below its top.
const LOWER_TOP: i32 = -96;
/// Header bytes +36 of a created tile: the value most shipped tiles carry.
const NEW_TILE_UNKNOWN_36: [u8; 4] = [0xff, 0, 0xff, 0];

const ISO_X_JUMP: [i32; 15] = [14, 12, 10, 8, 6, 4, 2, 0, 2, 4, 6, 8, 10, 12, 14];
const ISO_PIXELS: [i32; 15] = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];
/// Positions in one isometric sub-block.
const ISO_LEN: usize = 256;

// ---- shapes ----

/// What kind of tile an orientation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TileClass {
    /// Orientation 0.
    Floor,
    /// Orientation 15.
    Roof,
    /// Orientation 13.
    Shadow,
    /// Orientations 1-9, 12 and 14.
    UpperWall,
    /// Orientations 10 and 11 (markers such as warps and stairs).
    Special,
    /// Orientations 16-19.
    LowerWall,
}

impl TileClass {
    /// `None` outside 0..=19.
    pub fn of_orientation(orientation: i32) -> Option<TileClass> {
        Some(match orientation {
            0 => TileClass::Floor,
            15 => TileClass::Roof,
            13 => TileClass::Shadow,
            10 | 11 => TileClass::Special,
            1..=14 => TileClass::UpperWall,
            16..=19 => TileClass::LowerWall,
            _ => return None,
        })
    }

    /// Floors and roofs are diamonds; everything else is wall-like.
    pub fn is_diamond(self) -> bool {
        matches!(self, TileClass::Floor | TileClass::Roof)
    }
}

/// The shape of a tile: its orientation and, for wall-like tiles, how tall
/// the picture is. Every function here normalizes it first
/// ([`TileShape::normalized`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TileShape {
    pub orientation: u8,
    /// Pixel height of the drawable area of a wall, shadow or special
    /// (rounded up to a multiple of 32). Floors and roofs: ignored (79).
    pub height: u32,
    /// Roofs only: how far up the game lifts the roof (`Tile::roof_height`).
    pub roof_height: i16,
}

impl TileShape {
    pub fn floor() -> TileShape {
        TileShape {
            orientation: 0,
            height: 0,
            roof_height: 0,
        }
    }

    pub fn roof(roof_height: i16) -> TileShape {
        TileShape {
            orientation: 15,
            height: 0,
            roof_height,
        }
    }

    /// A wall, shadow or special of `orientation` with a picture `height`
    /// pixels tall.
    pub fn wall(orientation: u8, height: u32) -> TileShape {
        TileShape {
            orientation,
            height,
            roof_height: 0,
        }
    }

    /// The class, or `None` for an orientation outside 0..=19.
    pub fn class(&self) -> Option<TileClass> {
        TileClass::of_orientation(i32::from(self.orientation))
    }

    /// The shape with its height rounded up to whole 32-pixel blocks (floors
    /// and roofs: 0) and `roof_height` 0 unless it is a roof. Refuses an
    /// orientation outside 0..=19 and a wall height of 0 or over
    /// [`MAX_WALL_HEIGHT`].
    pub fn normalized(self) -> Result<TileShape> {
        let Some(class) = self.class() else {
            return refuse(format!(
                "orientation {} is not a tile shape (0-19)",
                self.orientation
            ));
        };
        let mut out = self;
        if class.is_diamond() {
            out.height = 0;
        } else {
            if self.height == 0 || self.height > MAX_WALL_HEIGHT {
                return refuse(format!(
                    "a wall picture must be 1 to {MAX_WALL_HEIGHT} pixels tall (got {})",
                    self.height
                ));
            }
            out.height = self.height.div_ceil(WALL_BLOCK) * WALL_BLOCK;
        }
        if class != TileClass::Roof {
            out.roof_height = 0;
        }
        Ok(out)
    }

    /// The `Tile::height` this (normalized) shape stores: -128 for floors
    /// and roofs, `-(height + 32)` for upper walls, shadows and specials,
    /// `-height` for lower walls.
    pub fn stored_height(&self) -> i32 {
        let h = i32::try_from(self.height).unwrap_or(i32::MAX);
        match self.class() {
            Some(TileClass::Floor | TileClass::Roof) | None => FLOOR_STORED_HEIGHT,
            Some(TileClass::LowerWall) => -h,
            Some(_) => -(h.saturating_add(32)),
        }
    }

    /// The smallest shape of `tile`'s orientation whose drawable area holds
    /// every block the tile has (the height comes from the header and grows
    /// to cover blocks outside it). `None` for an orientation outside 0..=19.
    pub fn of_tile(tile: &Tile) -> Option<TileShape> {
        let orientation = u8::try_from(tile.orientation).ok()?;
        let class = TileClass::of_orientation(tile.orientation)?;
        if class == TileClass::Floor {
            return Some(TileShape::floor());
        }
        if class == TileClass::Roof {
            return Some(TileShape::roof(tile.roof_height));
        }
        let min_y = tile.blocks.iter().map(|b| i64::from(b.y)).min();
        let max_y = tile.blocks.iter().map(|b| i64::from(b.y)).max();
        let declared = -i64::from(tile.height);
        let needed = if class == TileClass::LowerWall {
            let by_blocks = max_y.map_or(0, |y| y + 32 - i64::from(LOWER_TOP));
            declared.max(by_blocks)
        } else {
            let by_blocks = min_y.map_or(0, |y| -y);
            (declared - 32).max(by_blocks)
        };
        let height = u32::try_from(needed.clamp(32, i64::from(MAX_WALL_HEIGHT))).ok()?;
        TileShape::wall(orientation, height).normalized().ok()
    }
}

/// `Tile::direction` of shipped tiles of each orientation (every shipped
/// tile but 42 floors follows it).
pub fn direction_of(orientation: i32) -> i32 {
    match orientation {
        1 | 5 | 8 | 10 => 1,
        2 | 6 | 9 | 11 => 2,
        7 => 4,
        15 => 5,
        16..=19 => orientation - 10,
        _ => 3,
    }
}

/// The sub-tile collision flags (`y * 5 + x`) a new tile of `orientation`
/// gets: 0x07 (block walking, sight and jumps) on the cells a wall stands
/// on, the most common non-empty pattern of shipped tiles of that
/// orientation; floors, roofs, shadows, lower walls and orientations 12 and
/// 14 (free-standing objects with no common pattern) get none.
pub fn default_flags(orientation: i32) -> [u8; 25] {
    const WALL: u8 = 0x07;
    let mut f = [0u8; 25];
    let column = [0usize, 5, 10, 15, 20];
    let row = [0usize, 1, 2, 3, 4];
    let cells: &[usize] = match orientation {
        1 | 5 | 10 => &column,
        2 | 6 | 11 => &row,
        3 | 4 => &[0, 1, 2, 3, 4, 5, 10, 15, 20],
        7 => &[0],
        8 => &[0, 20],
        9 => &[0, 4],
        _ => &[],
    };
    for &c in cells {
        if let Some(v) = f.get_mut(c) {
            *v = WALL;
        }
    }
    f
}

/// The other half of a north corner wall (3 <-> 4), drawn together with it.
pub fn corner_partner(orientation: i32) -> Option<i32> {
    match orientation {
        3 => Some(4),
        4 => Some(3),
        _ => None,
    }
}

// ---- template ----

/// A rectangle in anchor space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let dx = i64::from(x) - i64::from(self.x);
        let dy = i64::from(y) - i64::from(self.y);
        (0..i64::from(self.width)).contains(&dx) && (0..i64::from(self.height)).contains(&dy)
    }
}

/// One place a sub-block can go, in the order [`build_tile`] writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TemplateCell {
    /// The block origin (`Block::x/y`).
    pub x: i32,
    pub y: i32,
    pub grid_x: u8,
    pub grid_y: u8,
    /// A 32x15 isometric diamond (floors and roofs) rather than a 32x32 square.
    pub diamond: bool,
}

/// The drawable area of a shape: where a picture's pixels can be stored.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TileTemplate {
    /// The normalized shape.
    pub shape: TileShape,
    /// Bounding box of the drawable pixels, in anchor space.
    pub rect: PixelRect,
    pub cells: Vec<TemplateCell>,
}

fn in_diamond(dx: i32, dy: i32) -> bool {
    let row = usize::try_from(dy).ok();
    match (
        row.and_then(|r| ISO_X_JUMP.get(r)),
        row.and_then(|r| ISO_PIXELS.get(r)),
    ) {
        (Some(&jump), Some(&n)) => dx >= jump && dx < jump + n,
        _ => false,
    }
}

/// The drawable area of `shape` (normalized first).
pub fn template(shape: TileShape) -> Result<TileTemplate> {
    let shape = shape.normalized()?;
    let class = shape.class().unwrap_or(TileClass::Floor);
    let h = i32::try_from(shape.height).unwrap_or(0);
    let mut cells = Vec::new();
    let rect = if class.is_diamond() {
        // written last cell first, as shipped
        for gy in (0..5u8).rev() {
            for gx in (0..5u8).rev() {
                cells.push(TemplateCell {
                    x: 64 + 16 * (i32::from(gx) - i32::from(gy)),
                    y: 8 * (i32::from(gx) + i32::from(gy)),
                    grid_x: gx,
                    grid_y: gy,
                    diamond: true,
                });
            }
        }
        PixelRect {
            x: 0,
            y: 0,
            width: TILE_WIDTH as u32,
            height: FLOOR_HEIGHT as u32,
        }
    } else {
        let (x0, x1) = match shape.orientation {
            3 => (80, TILE_WIDTH),
            4 => (0, 80),
            _ => (0, TILE_WIDTH),
        };
        // upper walls top row first, lower walls bottom row first; right to
        // left within a row, as shipped
        let (top, rows): (i32, Vec<i32>) = if class == TileClass::LowerWall {
            (
                LOWER_TOP,
                (0..h / 32).rev().map(|r| LOWER_TOP + 32 * r).collect(),
            )
        } else {
            (-h, (0..h / 32).map(|r| -h + 32 * r).collect())
        };
        for y in rows {
            for x in (0..5).rev().map(|c| c * 32) {
                if x + 32 > x0 && x < x1 {
                    cells.push(TemplateCell {
                        x,
                        y,
                        grid_x: 0,
                        grid_y: 0,
                        diamond: false,
                    });
                }
            }
        }
        PixelRect {
            x: x0,
            y: top,
            width: (x1 - x0) as u32,
            height: shape.height,
        }
    };
    Ok(TileTemplate { shape, rect, cells })
}

impl TileTemplate {
    /// The pixel at anchor-space `(x, y)` can be stored.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        if !self.rect.contains(x, y) {
            return false;
        }
        if !self.shape.class().is_some_and(TileClass::is_diamond) {
            return true;
        }
        self.cells.iter().any(|c| in_diamond(x - c.x, y - c.y))
    }

    /// The drawable area as an image of `rect`: index 255, opaque where a
    /// pixel can be stored (a guide for painting or fitting art).
    pub fn mask(&self) -> IndexedImage {
        let mut img =
            IndexedImage::new(self.rect.width, self.rect.height, self.rect.x, self.rect.y);
        let w = self.rect.width as usize;
        for row in 0..self.rect.height as usize {
            for col in 0..w {
                if self.contains(self.rect.x + col as i32, self.rect.y + row as i32) {
                    let i = row * w + col;
                    if let (Some(p), Some(a)) = (img.pixels.get_mut(i), img.alpha.get_mut(i)) {
                        *p = 255;
                        *a = 255;
                    }
                }
            }
        }
        img
    }

    /// A plain top-left picture placed in anchor space: its pixel (0,0) goes
    /// on the top-left of `rect` (the offsets of `plain` are ignored).
    pub fn to_anchor(&self, plain: &IndexedImage) -> IndexedImage {
        let mut out = plain.clone();
        out.offset_x = self.rect.x;
        out.offset_y = self.rect.y;
        out
    }

    /// An anchor-space picture as a plain picture of exactly `rect` (pixel
    /// (0,0) = the top-left of `rect`, offsets 0); what lies outside is cut.
    pub fn to_plain(&self, anchored: &IndexedImage) -> IndexedImage {
        let mut out = IndexedImage::new(self.rect.width, self.rect.height, 0, 0);
        let w = self.rect.width as usize;
        for row in 0..self.rect.height as usize {
            for col in 0..w {
                let (x, y) = (self.rect.x + col as i32, self.rect.y + row as i32);
                if let Some((v, a)) = pixel_at(anchored, x, y) {
                    let i = row * w + col;
                    if let (Some(p), Some(al)) = (out.pixels.get_mut(i), out.alpha.get_mut(i)) {
                        *p = v;
                        *al = a;
                    }
                }
            }
        }
        out
    }
}

/// `(index, alpha)` of `image` at anchor-space `(x, y)`, if inside it.
fn pixel_at(image: &IndexedImage, x: i32, y: i32) -> Option<(u8, u8)> {
    let col = usize::try_from(i64::from(x) - i64::from(image.offset_x)).ok()?;
    let row = usize::try_from(i64::from(y) - i64::from(image.offset_y)).ok()?;
    if col >= image.width as usize || row >= image.height as usize {
        return None;
    }
    let i = row.checked_mul(image.width as usize)?.checked_add(col)?;
    Some((*image.pixels.get(i)?, *image.alpha.get(i)?))
}

/// The opaque index of `image` at `(x, y)`; index 0 counts as transparent
/// (DT1 decoders draw nothing for it).
fn stored_at(image: &IndexedImage, x: i32, y: i32) -> Option<u8> {
    pixel_at(image, x, y).and_then(|(v, a)| (a != 0 && v != 0).then_some(v))
}

// ---- building ----

/// Header values of a built tile beyond its shape and number.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildOptions {
    /// `None`: [`default_flags`] of the orientation.
    pub flags: Option<[u8; 25]>,
    /// 0 (the shipped default): with no other version of the number, the
    /// game draws this one.
    pub rarity: i32,
    pub sound: u8,
    pub animated: u8,
}

/// A built tile and what of the picture it could not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltTile {
    pub tile: Tile,
    /// Opaque picture pixels outside the drawable area (not stored).
    pub dropped: usize,
    /// Their bounding box in anchor space.
    pub dropped_bounds: Option<PixelRect>,
    /// Opaque pixels of palette index 0 inside the area: index 0 is the
    /// transparent key of DT1s, so they do not show.
    pub transparent_zero: usize,
}

impl BuiltTile {
    /// A warning for the caller to show, when something was not stored.
    pub fn warning(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.dropped > 0 {
            parts.push(format!(
                "{} pixel(s) lie outside the tile's drawable area and were left out",
                self.dropped
            ));
        }
        if self.transparent_zero > 0 {
            parts.push(format!(
                "{} pixel(s) use palette index 0, which DT1 tiles draw as transparent",
                self.transparent_zero
            ));
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

#[derive(Default)]
struct Loss {
    dropped: usize,
    bounds: Option<(i32, i32, i32, i32)>,
    zero: usize,
}

impl Loss {
    fn rect(&self) -> Option<PixelRect> {
        self.bounds.map(|(x0, y0, x1, y1)| PixelRect {
            x: x0,
            y: y0,
            width: u32::try_from(i64::from(x1) - i64::from(x0) + 1).unwrap_or(u32::MAX),
            height: u32::try_from(i64::from(y1) - i64::from(y0) + 1).unwrap_or(u32::MAX),
        })
    }
}

/// Counts the opaque pixels of `image` that `keep` refuses, and the index-0
/// ones it keeps. Walks only the pixels the buffers hold.
fn measure_loss(image: &IndexedImage, keep: impl Fn(i32, i32) -> bool) -> Loss {
    let mut loss = Loss::default();
    let w = image.width as usize;
    if w == 0 {
        return loss;
    }
    let n = (w.saturating_mul(image.height as usize))
        .min(image.alpha.len())
        .min(image.pixels.len());
    for i in 0..n {
        let (Some(&a), Some(&v)) = (image.alpha.get(i), image.pixels.get(i)) else {
            continue;
        };
        if a == 0 {
            continue;
        }
        let (col, row) = ((i % w) as i64, (i / w) as i64);
        let (Ok(x), Ok(y)) = (
            i32::try_from(i64::from(image.offset_x) + col),
            i32::try_from(i64::from(image.offset_y) + row),
        ) else {
            loss.dropped += 1;
            continue;
        };
        if keep(x, y) {
            if v == 0 {
                loss.zero += 1;
            }
            continue;
        }
        loss.dropped += 1;
        loss.bounds = Some(match loss.bounds {
            None => (x, y, x, y),
            Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
        });
    }
    loss
}

/// Sets one pixel of a block-local cell image.
fn put(cell: &mut IndexedImage, i: usize, v: u8) -> bool {
    match (cell.pixels.get_mut(i), cell.alpha.get_mut(i)) {
        (Some(p), Some(a)) => {
            *p = v;
            *a = 255;
            true
        }
        _ => false,
    }
}

/// The blocks of `t` holding `image`'s pixels (anchor space).
fn split_blocks(image: &IndexedImage, t: &TileTemplate) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    for c in &t.cells {
        let (Ok(bx), Ok(by)) = (i16::try_from(c.x), i16::try_from(c.y)) else {
            continue;
        };
        if c.diamond {
            let mut cell = IndexedImage::new(32, 15, c.x, c.y);
            let mut n = 0usize;
            for (row, (&jump, &count)) in ISO_X_JUMP.iter().zip(&ISO_PIXELS).enumerate() {
                for dx in jump..jump + count {
                    if let Some(v) = stored_at(image, c.x + dx, c.y + row as i32)
                        && put(&mut cell, row * 32 + dx as usize, v)
                    {
                        n += 1;
                    }
                }
            }
            if n == ISO_LEN {
                blocks.push(Block::isometric(bx, by, c.grid_x, c.grid_y, &cell));
            } else if n > 0 {
                blocks.push(Block::rle(
                    bx,
                    by,
                    c.grid_x,
                    c.grid_y,
                    FLOOR_RLE_FORMAT,
                    &cell,
                    FLOOR_RLE_ROWS,
                )?);
            }
        } else {
            let mut cell = IndexedImage::new(32, 32, c.x, c.y);
            let mut any = false;
            for row in 0..32i32 {
                for col in 0..32i32 {
                    let (x, y) = (c.x + col, c.y + row);
                    if t.rect.contains(x, y)
                        && let Some(v) = stored_at(image, x, y)
                    {
                        any |= put(&mut cell, (row * 32 + col) as usize, v);
                    }
                }
            }
            if any {
                blocks.push(Block::rle(
                    bx,
                    by,
                    0,
                    0,
                    WALL_RLE_FORMAT,
                    &cell,
                    WALL_RLE_ROWS,
                )?);
            }
        }
    }
    Ok(blocks)
}

/// A tile of `t`'s shape holding `blocks`.
fn header(t: &TileTemplate, main: i32, sub: i32, blocks: Vec<Block>, opts: &BuildOptions) -> Tile {
    let o = i32::from(t.shape.orientation);
    let width = if t.shape.class().is_some_and(TileClass::is_diamond) {
        TILE_WIDTH
    } else {
        blocks
            .iter()
            .map(|b| i32::from(b.x) + 32)
            .max()
            .unwrap_or(0)
    };
    Tile {
        direction: direction_of(o),
        roof_height: t.shape.roof_height,
        sound: opts.sound,
        animated: opts.animated,
        height: t.shape.stored_height(),
        width,
        orientation: o,
        main_index: main,
        sub_index: sub,
        rarity: opts.rarity,
        subtile_flags: opts.flags.unwrap_or_else(|| default_flags(o)),
        blocks,
        preserved: TilePreserved {
            unknown_36: NEW_TILE_UNKNOWN_36,
            ..TilePreserved::default()
        },
    }
}

/// Builds a tile of `shape` numbered `main`/`sub` from `image` (anchor
/// space; [`TileTemplate::to_anchor`] places a plain picture), split into
/// sub-blocks exactly as shipped tiles of that kind are. Opaque pixels
/// outside the drawable area and index-0 pixels are counted in the result,
/// not stored. Refuses a shape [`TileShape::normalized`] refuses.
pub fn build_tile(
    image: &IndexedImage,
    shape: TileShape,
    main: i32,
    sub: i32,
    opts: &BuildOptions,
) -> Result<BuiltTile> {
    let t = template(shape)?;
    let loss = measure_loss(image, |x, y| t.contains(x, y));
    let blocks = split_blocks(image, &t)?;
    Ok(BuiltTile {
        tile: header(&t, main, sub, blocks, opts),
        dropped: loss.dropped,
        dropped_bounds: loss.rect(),
        transparent_zero: loss.zero,
    })
}

/// Both halves of a north corner wall `height` pixels tall from one picture
/// (anchor space, x 0..160): orientation 4 gets x 0..80, orientation 3 x
/// 80..160, both numbered `main`/`sub`. Returns `(orientation 3, orientation
/// 4)`; `dropped` counts pixels outside both halves and is reported on the
/// orientation-3 half only (the other half's is 0).
pub fn build_corner_pair(
    image: &IndexedImage,
    height: u32,
    main: i32,
    sub: i32,
    opts: &BuildOptions,
) -> Result<(BuiltTile, BuiltTile)> {
    let right = template(TileShape::wall(3, height))?;
    let left = template(TileShape::wall(4, height))?;
    let loss = measure_loss(image, |x, y| right.contains(x, y) || left.contains(x, y));
    let zero_in = |t: &TileTemplate| measure_loss(image, |x, y| t.contains(x, y)).zero;
    let r = header(&right, main, sub, split_blocks(image, &right)?, opts);
    let l = header(&left, main, sub, split_blocks(image, &left)?, opts);
    Ok((
        BuiltTile {
            tile: r,
            dropped: loss.dropped,
            dropped_bounds: loss.rect(),
            transparent_zero: zero_in(&right),
        },
        BuiltTile {
            tile: l,
            dropped: 0,
            dropped_bounds: None,
            transparent_zero: zero_in(&left),
        },
    ))
}

/// `tile` re-split into `shape` (a taller or shorter wall, another
/// orientation, floor <-> roof, ...): its pixels stay where they are in
/// anchor space and are kept where the new area holds them (the rest is
/// counted in `dropped`). Number, rarity, sound, animation and preserved
/// header bytes are kept; flags too when the orientation stays, else the new
/// orientation's [`default_flags`].
///
/// Upper walls are anchored at their base and lower walls 96 pixels below
/// their top, so a picture moved between those classes keeps its anchor
/// position, not its top. Fails only for a refused shape or a tile that
/// does not render.
pub fn reshape_tile(tile: &Tile, shape: TileShape) -> Result<BuiltTile> {
    let image = tile.render()?;
    let flags = (i32::from(shape.orientation) == tile.orientation).then_some(tile.subtile_flags);
    let opts = BuildOptions {
        flags,
        rarity: tile.rarity,
        sound: tile.sound,
        animated: tile.animated,
    };
    let mut built = build_tile(&image, shape, tile.main_index, tile.sub_index, &opts)?;
    built.tile.preserved = tile.preserved.clone();
    Ok(built)
}

// ---- image import ----

/// How a picture is put into a template.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Placement {
    /// Unscaled, its top-left on the top-left of the drawable box (a
    /// picture painted on [`TileTemplate::mask`]).
    #[default]
    AsIs,
    /// Unscaled, centred in the drawable box (both axes).
    Center,
    /// Scaled with nearest-neighbour by the largest factor that fits the
    /// drawable box (aspect kept, up or down), then centred.
    Fit,
}

/// Options of [`import_tile_image`].
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImportOptions {
    pub quantize: QuantizeOptions,
    pub placement: Placement,
}

/// Where a `w`x`h` picture goes in `rect`: anchor-space origin and size.
fn placed_rect(w: u32, h: u32, rect: PixelRect, placement: Placement) -> (i32, i32, u32, u32) {
    let (nw, nh) = match placement {
        Placement::Fit if w > 0 && h > 0 => {
            // the largest scale s with w*s <= rw and h*s <= rh
            let (rw, rh) = (u64::from(rect.width), u64::from(rect.height));
            let (w64, h64) = (u64::from(w), u64::from(h));
            if rw * h64 <= rh * w64 {
                (
                    rect.width,
                    u32::try_from((h64 * rw / w64).max(1)).unwrap_or(1),
                )
            } else {
                (
                    u32::try_from((w64 * rh / h64).max(1)).unwrap_or(1),
                    rect.height,
                )
            }
        }
        _ => (w, h),
    };
    let centre = |r: u32, n: u32| i32::try_from((i64::from(r) - i64::from(n)) / 2).unwrap_or(0);
    match placement {
        Placement::AsIs => (rect.x, rect.y, nw, nh),
        Placement::Center | Placement::Fit => (
            rect.x.saturating_add(centre(rect.width, nw)),
            rect.y.saturating_add(centre(rect.height, nh)),
            nw,
            nh,
        ),
    }
}

/// Source coordinate of destination `d` when `src` pixels become `dst`.
fn nearest(d: u32, dst: u32, src: u32) -> usize {
    if dst == 0 {
        return 0;
    }
    ((u64::from(d) * 2 + 1) * u64::from(src) / (u64::from(dst) * 2)) as usize
}

/// A plain RGBA picture placed in `template` (anchor-space offsets). With
/// `AsIs` and `Center` the picture keeps its size (what lies outside the
/// drawable area is reported later by [`build_tile`]); `Fit` never exceeds
/// the drawable box.
pub fn place_rgba(frame: &RgbaFrame, template: &TileTemplate, placement: Placement) -> RgbaFrame {
    let (x, y, w, h) = placed_rect(frame.width, frame.height, template.rect, placement);
    if (w, h) == (frame.width, frame.height) {
        let mut out = frame.clone();
        out.offset_x = x;
        out.offset_y = y;
        return out;
    }
    let mut out = RgbaFrame::new(w, h, x, y);
    let sw = frame.width as usize;
    for row in 0..h {
        let sy = nearest(row, h, frame.height);
        for col in 0..w {
            let sx = nearest(col, w, frame.width);
            let src = sy.saturating_mul(sw).saturating_add(sx).saturating_mul(4);
            let dst = (row as usize * w as usize + col as usize) * 4;
            if let (Some(s), Some(d)) = (
                frame.pixels.get(src..src.saturating_add(4)),
                out.pixels.get_mut(dst..dst + 4),
            ) {
                d.copy_from_slice(s);
            }
        }
    }
    out
}

/// [`place_rgba`] for a palette-indexed picture (its offsets are ignored).
pub fn place_indexed(
    image: &IndexedImage,
    template: &TileTemplate,
    placement: Placement,
) -> IndexedImage {
    let (x, y, w, h) = placed_rect(image.width, image.height, template.rect, placement);
    if (w, h) == (image.width, image.height) {
        let mut out = image.clone();
        out.offset_x = x;
        out.offset_y = y;
        return out;
    }
    let mut out = IndexedImage::new(w, h, x, y);
    let sw = image.width as usize;
    for row in 0..h {
        let sy = nearest(row, h, image.height);
        for col in 0..w {
            let sx = nearest(col, w, image.width);
            let src = sy.saturating_mul(sw).saturating_add(sx);
            let dst = row as usize * w as usize + col as usize;
            if let (Some(&p), Some(&a)) = (image.pixels.get(src), image.alpha.get(src)) {
                if let Some(dp) = out.pixels.get_mut(dst) {
                    *dp = p;
                }
                if let Some(da) = out.alpha.get_mut(dst) {
                    *da = a;
                }
            }
        }
    }
    out
}

/// An image file (PNG, BMP, GIF first frame, PCX) as an anchor-space picture
/// for `shape`: placed (on RGBA, so scaling happens before colour matching)
/// and then quantized to `palette` with `opts.quantize`. Feed the result to
/// [`build_tile`]. Refuses a shape [`TileShape::normalized`] refuses or a
/// file that does not decode.
pub fn import_tile_image(
    bytes: &[u8],
    kind: ImageKind,
    palette: &Palette,
    shape: TileShape,
    opts: &ImportOptions,
) -> Result<IndexedImage> {
    let t = template(shape)?;
    let frame = match import_image(bytes, kind) {
        Ok(f) => f,
        Err(e) => return refuse(format!("the picture cannot be read: {e}")),
    };
    let placed = place_rgba(&frame, &t, opts.placement);
    Ok(quantize(&placed, palette, &opts.quantize))
}

// ---- new DT1s and numbers ----

/// An empty DT1 (version 7.6, no tiles), as every shipped file is.
pub fn new_dt1() -> Dt1 {
    Dt1 {
        version: (7, 6),
        tiles: Vec::new(),
    }
}

/// The orientations a new tile of `orientation` takes a number in: both
/// halves for a north corner, else itself.
pub fn number_orientations(orientation: i32) -> Vec<i32> {
    match corner_partner(orientation) {
        Some(p) => {
            let mut v = vec![orientation, p];
            v.sort_unstable();
            v
        }
        None => vec![orientation],
    }
}

/// The first `(main, sub)` free in every one of `orientations` (see
/// [`number_orientations`]) among `taken`: the lowest free sub of
/// `prefer_main` when given and not full, else the lowest main, then sub.
/// DS1 cells hold main 0-63 and sub 0-255; `None` when all are taken.
pub fn free_tile_number(
    taken: &BTreeSet<TileId>,
    orientations: &[i32],
    prefer_main: Option<i32>,
) -> Option<(i32, i32)> {
    let free = |m: i32, s: i32| {
        orientations
            .iter()
            .all(|&o| !taken.contains(&TileId::new(o, m, s)))
    };
    if let Some(m) = prefer_main.filter(|m| (0..=MAX_MAIN).contains(m))
        && let Some(s) = (0..=MAX_SUB).find(|&s| free(m, s))
    {
        return Some((m, s));
    }
    (0..=MAX_MAIN)
        .flat_map(|m| (0..=MAX_SUB).map(move |s| (m, s)))
        .find(|&(m, s)| free(m, s))
}

// ---- into the map's own DT1 ----

/// The map side of [`add_new_tiles_to_map`]: the fields of [`AddTilesInput`]
/// without the picks.
#[derive(Clone, Copy)]
pub struct MapTarget<'a> {
    pub map_path: &'a str,
    pub ds1: &'a Ds1,
    pub types: &'a TxtDoc,
    pub prest: &'a TxtDoc,
    pub type_id: u32,
    pub prest_def: Option<u32>,
    pub type_dt1s: &'a [(String, &'a Dt1)],
    pub existing_own: Option<&'a Dt1>,
    pub existing_own_path: Option<&'a str>,
}

/// The name created tiles go by in plans and change lists.
pub const NEW_TILES_SOURCE: &str = "new tiles";

/// Puts created tiles into the level type's own DT1 through
/// [`add_tiles_to_map`], as if copied from an in-memory DT1 holding them:
/// numbers clashing with the type's DT1s are renumbered (corner halves,
/// animation frames and versions of one number stay together), the file is
/// registered in LvlTypes and the map's Dt1Mask and listed in the DS1.
/// `outcome.placed` maps each given number to the one it got. Special tiles
/// (10/11) are skipped with a warning, as in the copy flow.
pub fn add_new_tiles_to_map(target: &MapTarget<'_>, tiles: &[Tile]) -> Result<AddTilesOutcome> {
    let source = Dt1 {
        version: (7, 6),
        tiles: tiles.to_vec(),
    };
    let sources = [(NEW_TILES_SOURCE.to_owned(), &source)];
    let picks: Vec<TilePick> = (0..tiles.len())
        .map(|index| TilePick { source: 0, index })
        .collect();
    add_tiles_to_map(&AddTilesInput {
        map_path: target.map_path,
        ds1: target.ds1,
        types: target.types,
        prest: target.prest,
        type_id: target.type_id,
        prest_def: target.prest_def,
        picks: &picks,
        sources: &sources,
        type_dt1s: target.type_dt1s,
        existing_own: target.existing_own,
        existing_own_path: target.existing_own_path,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests;
