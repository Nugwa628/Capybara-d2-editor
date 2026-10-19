//! DT1 tile files (`data\global\tiles\**\*.dt1`). Read-only: there is no
//! writer and there never will be.
//!
//! ```text
//! 0x000 i32   version major (7; the only value the game checks)
//! 0x004 i32   flags (6 in every shipped file; stored, not compared)
//! 0x008 260   zero-filled name buffer
//! 0x10C i32   number of tiles
//! 0x110 i32   offset of the tile header array (276)
//! tile header, 96 bytes each:
//!   +0 i32 direction   +4 i16 roof height   +6 u8 sound   +7 u8 animated
//!   +8 i32 height (<= 0)  +12 i32 width  +20 i32 orientation
//!   +24 i32 main  +28 i32 sub  +32 i32 rarity  +40 [u8;25] sub-tile flags
//!   +72 i32 sub-block header offset  +76 i32 their byte length (unused)
//!   +80 i32 sub-block count
//! sub-block header, 20 bytes each, at `tiles_ptr + 20 * i`:
//!   +0 i16 x  +2 i16 y  +6 u8 grid x  +7 u8 grid y  +8 i16 format
//!   +10 u16 length  +16 i32 data offset (relative to `tiles_ptr`)
//! ```
//!
//! Format 1 sub-blocks are 32x15 isometric diamonds (256 literal bytes);
//! every other format is the RLE scheme `(skip, count, count literals)` with
//! `(0, 0)` ending a row. Palette index 0 is the transparent key: decoded
//! pixels with value 0 get alpha 0 (as the reference editor does).
//!
//! # Coordinates
//!
//! A block's `x`/`y` are in the tile's own anchor space, the same space the
//! game draws in. [`Tile::render`] returns the union of all blocks with
//! `offset_x`/`offset_y` giving the position of pixel (0,0) in that space.
//! (For a floor the anchor is the top-left of the 160x80 tile, for an upper
//! wall it is the bottom of the tile, for a lower wall it is 96 pixels below
//! the top; the renderer picks the anchor per orientation.)
//!
//! # Limits
//!
//! The bytes copied into [`Block::data`] across a whole file are capped at
//! the file length plus 1 MiB, and a rendered tile at 2048x2048 pixels, so
//! hostile input cannot over-allocate. The six `4.1` files shipped in
//! `act1\{barracks,catacomb,cathedrl,court,outdoors}` are an unrelated older
//! layout no tool can read; they fail with [`FormatError::UnsupportedVersion`].

use crate::bytes::{i16_at, i32_at, slice_at, u16_at};
use crate::{FormatError, IndexedImage, Result};

const FORMAT: &str = "dt1";
const HEADER_LEN: usize = 276;
const TILE_HEADER_LEN: usize = 96;
const BLOCK_HEADER_LEN: usize = 20;
/// Rendered tiles may not exceed this many pixels per side.
const MAX_RENDER_DIM: i64 = 2048;
/// Slack on top of the file length for the copied block data.
const DATA_SLACK: usize = 1 << 20;

const ISO_FORMAT: i16 = 1;
const ISO_X_JUMP: [i64; 15] = [14, 12, 10, 8, 6, 4, 2, 0, 2, 4, 6, 8, 10, 12, 14];
const ISO_PIXELS: [usize; 15] = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];
/// File byte `t` of the 5x5 sub-tile flags belongs to grid cell
/// `IDX_TABLE[t]` = `y * 5 + x` (the file stores the grid bottom row first).
const IDX_TABLE: [usize; 25] = [
    20, 21, 22, 23, 24, 15, 16, 17, 18, 19, 10, 11, 12, 13, 14, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4,
];

/// The lookup key of a tile: `(orientation, main index, sub index)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub orientation: u8,
    pub main: u8,
    pub sub: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFormat {
    Isometric,
    Rle,
}

/// One sub-block (a piece of a tile) and its still-encoded pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub x: i16,
    pub y: i16,
    pub grid_x: u8,
    pub grid_y: u8,
    /// The format word as stored; 1 is isometric, anything else RLE.
    pub raw_format: i16,
    pub format: BlockFormat,
    /// The encoded pixel bytes (`length` bytes at `tiles_ptr + data_offset`).
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub direction: i32,
    pub roof_height: i16,
    pub sound: u8,
    pub animated: u8,
    /// The stored height, zero or negative (-80 for a floor, more for walls).
    pub height: i32,
    pub width: i32,
    pub orientation: i32,
    pub main_index: i32,
    pub sub_index: i32,
    pub rarity: i32,
    /// 5x5 collision flags, `y * 5 + x`, already reordered from the file's
    /// bottom-row-first layout.
    pub subtile_flags: [u8; 25],
    pub blocks: Vec<Block>,
}

impl Tile {
    /// `None` when orientation, main or sub index does not fit a byte.
    pub fn key(&self) -> Option<TileKey> {
        Some(TileKey {
            orientation: u8::try_from(self.orientation).ok()?,
            main: u8::try_from(self.main_index).ok()?,
            sub: u8::try_from(self.sub_index).ok()?,
        })
    }

    /// Decodes every block and composes them (later blocks overwrite earlier
    /// ones). The image covers exactly the pixels the blocks write; its
    /// offsets are in block space, see the module docs. Isometric blocks
    /// whose data is not exactly 256 bytes draw nothing, like the game.
    /// Fails only when the blocks span more than 2048 pixels on a side.
    pub fn render(&self) -> Result<IndexedImage> {
        let mut runs: Vec<Run<'_>> = Vec::new();
        for block in &self.blocks {
            decode_block(block, &mut runs);
        }
        compose(&runs)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dt1 {
    /// `(major, flags)`: `(7, 6)` in every readable file.
    pub version: (i32, i32),
    pub tiles: Vec<Tile>,
}

fn invalid(what: &'static str, offset: usize, detail: impl Into<String>) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset,
        detail: detail.into(),
    }
}

fn truncated(offset: usize, needed: usize) -> FormatError {
    FormatError::Truncated {
        format: FORMAT,
        offset,
        needed,
    }
}

impl Dt1 {
    pub fn parse(bytes: &[u8]) -> Result<Dt1> {
        if bytes.len() < HEADER_LEN {
            return Err(truncated(bytes.len(), HEADER_LEN - bytes.len()));
        }
        let major = i32_at(bytes, 0).ok_or_else(|| truncated(0, 4))?;
        let flags = i32_at(bytes, 4).ok_or_else(|| truncated(4, 4))?;
        if major != 7 {
            return Err(FormatError::UnsupportedVersion {
                format: FORMAT,
                version: i64::from(major),
            });
        }
        let count = i32_at(bytes, 268).ok_or_else(|| truncated(268, 4))?;
        let table = i32_at(bytes, 272).ok_or_else(|| truncated(272, 4))?;
        let count = usize::try_from(count)
            .map_err(|_| invalid("tile count", 268, format!("negative count {count}")))?;
        let table = usize::try_from(table)
            .map_err(|_| invalid("tile table offset", 272, format!("negative offset {table}")))?;
        if table > bytes.len() || count > (bytes.len() - table) / TILE_HEADER_LEN {
            return Err(invalid(
                "tile count",
                268,
                format!(
                    "{count} tile headers at {table:#x} do not fit in {} bytes",
                    bytes.len()
                ),
            ));
        }
        let budget = bytes.len().saturating_add(DATA_SLACK);
        let mut copied = 0usize;
        let mut tiles = Vec::with_capacity(count);
        for i in 0..count {
            tiles.push(parse_tile(
                bytes,
                table + i * TILE_HEADER_LEN,
                &mut copied,
                budget,
            )?);
        }
        Ok(Dt1 {
            version: (major, flags),
            tiles,
        })
    }
}

fn parse_tile(bytes: &[u8], at: usize, copied: &mut usize, budget: usize) -> Result<Tile> {
    let h = slice_at(bytes, at, TILE_HEADER_LEN).ok_or_else(|| truncated(at, TILE_HEADER_LEN))?;
    let i32f = |o: usize| i32_at(h, o).unwrap_or(0);
    let mut subtile_flags = [0u8; 25];
    for (t, &dst) in IDX_TABLE.iter().enumerate() {
        if let (Some(slot), Some(&v)) = (subtile_flags.get_mut(dst), h.get(40 + t)) {
            *slot = v;
        }
    }
    let tiles_ptr = i32f(72);
    let tiles_number = i32f(80);
    let ptr = usize::try_from(tiles_ptr)
        .map_err(|_| invalid("sub-block offset", at + 72, format!("negative {tiles_ptr}")))?;
    let number = usize::try_from(tiles_number).map_err(|_| {
        invalid(
            "sub-block count",
            at + 80,
            format!("negative {tiles_number}"),
        )
    })?;
    // each header is 20 bytes inside the file, so this also bounds `number`
    if ptr > bytes.len() || number > (bytes.len() - ptr) / BLOCK_HEADER_LEN {
        return Err(invalid(
            "sub-block count",
            at + 80,
            format!("{number} sub-block headers at {ptr:#x} do not fit"),
        ));
    }
    let mut blocks = Vec::with_capacity(number);
    for s in 0..number {
        let hdr_at = ptr + s * BLOCK_HEADER_LEN;
        let hdr = slice_at(bytes, hdr_at, BLOCK_HEADER_LEN)
            .ok_or_else(|| truncated(hdr_at, BLOCK_HEADER_LEN))?;
        let length = usize::from(u16_at(hdr, 10).unwrap_or(0));
        let offset = i32_at(hdr, 16).unwrap_or(0);
        let data_at = usize::try_from(offset)
            .ok()
            .and_then(|o| o.checked_add(ptr))
            .ok_or_else(|| {
                invalid(
                    "sub-block data offset",
                    hdr_at + 16,
                    format!("bad offset {offset}"),
                )
            })?;
        let data = slice_at(bytes, data_at, length).ok_or_else(|| {
            invalid(
                "sub-block data",
                hdr_at + 16,
                format!("{length} bytes at {data_at:#x} run past the end of the file"),
            )
        })?;
        *copied = copied.saturating_add(length);
        if *copied > budget {
            return Err(invalid(
                "sub-block data",
                hdr_at + 10,
                "sub-block data exceeds the file length by more than the allowed slack",
            ));
        }
        let raw_format = i16_at(hdr, 8).unwrap_or(0);
        blocks.push(Block {
            x: i16_at(hdr, 0).unwrap_or(0),
            y: i16_at(hdr, 2).unwrap_or(0),
            grid_x: hdr.get(6).copied().unwrap_or(0),
            grid_y: hdr.get(7).copied().unwrap_or(0),
            raw_format,
            format: if raw_format == ISO_FORMAT {
                BlockFormat::Isometric
            } else {
                BlockFormat::Rle
            },
            data: data.to_vec(),
        });
    }
    Ok(Tile {
        direction: i32f(0),
        roof_height: i16_at(h, 4).unwrap_or(0),
        sound: h.get(6).copied().unwrap_or(0),
        animated: h.get(7).copied().unwrap_or(0),
        height: i32f(8),
        width: i32f(12),
        orientation: i32f(20),
        main_index: i32f(24),
        sub_index: i32f(28),
        rarity: i32f(32),
        subtile_flags,
        blocks,
    })
}

/// A horizontal run of literal pixels at block-space `(x, y)`.
struct Run<'a> {
    x: i64,
    y: i64,
    pixels: &'a [u8],
}

fn decode_block<'a>(block: &'a Block, runs: &mut Vec<Run<'a>>) {
    let bx = i64::from(block.x);
    let by = i64::from(block.y);
    match block.format {
        BlockFormat::Isometric => {
            if block.data.len() != 256 {
                return;
            }
            let mut rest = block.data.as_slice();
            for (row, (&jump, &n)) in ISO_X_JUMP.iter().zip(&ISO_PIXELS).enumerate() {
                let Some((line, tail)) = rest.split_at_checked(n) else {
                    return;
                };
                runs.push(Run {
                    x: bx + jump,
                    y: by + row as i64,
                    pixels: line,
                });
                rest = tail;
            }
        }
        BlockFormat::Rle => {
            let data = block.data.as_slice();
            let (mut pos, mut x, mut y) = (0usize, 0i64, 0i64);
            while let (Some(&skip), Some(&count)) = (data.get(pos), data.get(pos + 1)) {
                pos += 2;
                if skip == 0 && count == 0 {
                    x = 0;
                    y += 1;
                    continue;
                }
                x += i64::from(skip);
                let end = (pos + usize::from(count)).min(data.len());
                if let Some(line) = data.get(pos..end) {
                    if !line.is_empty() {
                        runs.push(Run {
                            x: bx + x,
                            y: by + y,
                            pixels: line,
                        });
                    }
                    x += i64::from(count);
                }
                pos = end;
            }
        }
    }
}

fn compose(runs: &[Run<'_>]) -> Result<IndexedImage> {
    let mut bounds: Option<(i64, i64, i64, i64)> = None;
    for run in runs {
        let (x1, y1) = (run.x + run.pixels.len() as i64, run.y + 1);
        bounds = Some(match bounds {
            None => (run.x, run.y, x1, y1),
            Some((a, b, c, d)) => (a.min(run.x), b.min(run.y), c.max(x1), d.max(y1)),
        });
    }
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        return Ok(IndexedImage::new(0, 0, 0, 0));
    };
    let (w, h) = (max_x - min_x, max_y - min_y);
    if w > MAX_RENDER_DIM || h > MAX_RENDER_DIM {
        return Err(invalid(
            "tile extent",
            0,
            format!("blocks span {w}x{h} pixels (limit {MAX_RENDER_DIM})"),
        ));
    }
    let (Ok(off_x), Ok(off_y)) = (i32::try_from(min_x), i32::try_from(min_y)) else {
        return Err(invalid("tile extent", 0, "offset out of range"));
    };
    let mut image = IndexedImage::new(w as u32, h as u32, off_x, off_y);
    let width = w as usize;
    for run in runs {
        let row = (run.y - min_y) as usize;
        let start = row * width + (run.x - min_x) as usize;
        let (Some(px), Some(al)) = (
            image.pixels.get_mut(start..start + run.pixels.len()),
            image.alpha.get_mut(start..start + run.pixels.len()),
        ) else {
            continue;
        };
        px.copy_from_slice(run.pixels);
        for (a, &p) in al.iter_mut().zip(run.pixels) {
            *a = if p == 0 { 0 } else { 255 };
        }
    }
    Ok(image)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
