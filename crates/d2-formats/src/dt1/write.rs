//! DT1 writer and sub-block pixel encoders.
//!
//! Layout written (the one every shipped 7.6 file uses, checked over the
//! whole corpus): file header, tile headers at 276, then per tile in order
//! its sub-block headers followed by their data in block order. A tile
//! without blocks points at the current end (its region is empty). The name
//! buffer and the sub-block header bytes +4..+6 and +12..+16 are written as
//! zero, as in every shipped file; the length at +10 is a `u16`.

use super::{
    BLOCK_HEADER_LEN, Block, BlockFormat, Dt1, FORMAT, HEADER_LEN, IDX_TABLE, ISO_FORMAT,
    ISO_PIXELS, ISO_X_JUMP, Run, TILE_HEADER_LEN, Tile, compose, decode_block,
};
use crate::{FormatError, IndexedImage, Result};

/// Bytes of an isometric sub-block (15 rows, 4..32 pixels each).
const ISO_LEN: usize = 256;
/// Sub-block data length limit (the reader takes a `u16` length).
const MAX_RLE_LEN: usize = u16::MAX as usize;

fn not_representable(detail: impl Into<String>) -> FormatError {
    FormatError::NotRepresentable {
        format: FORMAT,
        version: 7,
        detail: detail.into(),
    }
}

fn put_i32(out: &mut [u8], at: usize, v: i32) {
    if let Some(dst) = out.get_mut(at..at + 4) {
        dst.copy_from_slice(&v.to_le_bytes());
    }
}

fn put_bytes(out: &mut [u8], at: usize, v: &[u8]) {
    if let Some(dst) = out.get_mut(at..at + v.len()) {
        dst.copy_from_slice(v);
    }
}

fn to_i32(v: usize, what: &str) -> Result<i32> {
    i32::try_from(v).map_err(|_| not_representable(format!("{what} {v} does not fit an i32")))
}

impl Dt1 {
    /// Serializes the model. A model from [`Dt1::parse`] left unchanged comes
    /// back byte for byte (every readable file in both corpora does); any
    /// other model parses back to itself.
    ///
    /// Fails with `UnsupportedVersion` unless `version.0` is 7, and with
    /// `NotRepresentable` when a block holds more than 65535 bytes, a block's
    /// `format` disagrees with its `raw_format`, or the file would pass
    /// 2 GiB.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let (major, flags) = self.version;
        if major != 7 {
            return Err(FormatError::UnsupportedVersion {
                format: FORMAT,
                version: i64::from(major),
            });
        }
        let count = to_i32(self.tiles.len(), "tile count")?;
        let headers_end = self
            .tiles
            .len()
            .checked_mul(TILE_HEADER_LEN)
            .and_then(|n| n.checked_add(HEADER_LEN))
            .ok_or_else(|| not_representable("too many tiles"))?;
        to_i32(headers_end, "tile header size")?;
        let mut out = vec![0u8; headers_end];
        put_i32(&mut out, 0, major);
        put_i32(&mut out, 4, flags);
        put_i32(&mut out, 268, count);
        put_i32(&mut out, 272, HEADER_LEN as i32);
        for (i, tile) in self.tiles.iter().enumerate() {
            let ptr = to_i32(out.len(), "sub-block offset")?;
            let region = encode_region(tile)?;
            let len = to_i32(region.len(), "sub-block region length")?;
            let end = out.len().saturating_add(region.len());
            to_i32(end, "file length")?;
            let header = tile_header(tile, ptr, len)?;
            put_bytes(&mut out, HEADER_LEN + i * TILE_HEADER_LEN, &header);
            out.extend_from_slice(&region);
        }
        Ok(out)
    }
}

fn tile_header(tile: &Tile, ptr: i32, len: i32) -> Result<[u8; TILE_HEADER_LEN]> {
    let mut h = [0u8; TILE_HEADER_LEN];
    put_i32(&mut h, 0, tile.direction);
    put_bytes(&mut h, 4, &tile.roof_height.to_le_bytes());
    h[6] = tile.sound;
    h[7] = tile.animated;
    put_i32(&mut h, 8, tile.height);
    put_i32(&mut h, 12, tile.width);
    put_bytes(&mut h, 16, &tile.preserved.unknown_16);
    put_i32(&mut h, 20, tile.orientation);
    put_i32(&mut h, 24, tile.main_index);
    put_i32(&mut h, 28, tile.sub_index);
    put_i32(&mut h, 32, tile.rarity);
    put_bytes(&mut h, 36, &tile.preserved.unknown_36);
    for (t, &src) in IDX_TABLE.iter().enumerate() {
        if let (Some(dst), Some(&v)) = (h.get_mut(40 + t), tile.subtile_flags.get(src)) {
            *dst = v;
        }
    }
    put_bytes(&mut h, 65, &tile.preserved.unknown_65);
    put_i32(&mut h, 72, ptr);
    put_i32(&mut h, 76, len);
    put_i32(&mut h, 80, to_i32(tile.blocks.len(), "sub-block count")?);
    put_bytes(&mut h, 84, &tile.preserved.runtime_84);
    Ok(h)
}

/// A tile's sub-block headers followed by their data.
fn encode_region(tile: &Tile) -> Result<Vec<u8>> {
    let headers = tile
        .blocks
        .len()
        .checked_mul(BLOCK_HEADER_LEN)
        .ok_or_else(|| not_representable("too many sub-blocks"))?;
    let data: usize = tile.blocks.iter().map(|b| b.data.len()).sum();
    let mut out = vec![0u8; headers];
    out.reserve(data);
    for (i, block) in tile.blocks.iter().enumerate() {
        let expected = if block.raw_format == ISO_FORMAT {
            BlockFormat::Isometric
        } else {
            BlockFormat::Rle
        };
        if block.format != expected {
            return Err(not_representable(format!(
                "sub-block format {:?} disagrees with raw format {:#x}",
                block.format, block.raw_format
            )));
        }
        let length = u16::try_from(block.data.len()).map_err(|_| {
            not_representable(format!(
                "sub-block of {} bytes (limit 65535)",
                block.data.len()
            ))
        })?;
        let offset = to_i32(out.len(), "sub-block data offset")?;
        let at = i * BLOCK_HEADER_LEN;
        put_bytes(&mut out, at, &block.x.to_le_bytes());
        put_bytes(&mut out, at + 2, &block.y.to_le_bytes());
        put_bytes(&mut out, at + 6, &[block.grid_x, block.grid_y]);
        put_bytes(&mut out, at + 8, &block.raw_format.to_le_bytes());
        put_bytes(&mut out, at + 10, &length.to_le_bytes());
        put_i32(&mut out, at + 16, offset);
        out.extend_from_slice(&block.data);
    }
    Ok(out)
}

/// The opaque pixel of `image` at block-space `(x, y)`, if any.
fn opaque_at(image: &IndexedImage, x: i64, y: i64) -> Option<u8> {
    let col = usize::try_from(x - i64::from(image.offset_x)).ok()?;
    let row = usize::try_from(y - i64::from(image.offset_y)).ok()?;
    if col >= image.width as usize || row >= image.height as usize {
        return None;
    }
    let i = row.checked_mul(image.width as usize)?.checked_add(col)?;
    (*image.alpha.get(i)? != 0).then(|| image.pixels.get(i).copied())?
}

impl Block {
    /// Decodes this block alone, like [`super::Tile::render`] does for a
    /// whole tile (offsets in block space, index 0 transparent).
    pub fn render(&self) -> Result<IndexedImage> {
        let mut runs: Vec<Run<'_>> = Vec::new();
        decode_block(self, &mut runs);
        compose(&runs)
    }

    /// An isometric sub-block at `(x, y)`: each of the 256 diamond positions
    /// takes the opaque pixel of `image` there (block space, as
    /// [`Block::render`] returns it), or 0. Pixels outside the diamond are
    /// ignored.
    pub fn isometric(x: i16, y: i16, grid_x: u8, grid_y: u8, image: &IndexedImage) -> Block {
        let mut data = Vec::with_capacity(ISO_LEN);
        for (row, (&jump, &n)) in ISO_X_JUMP.iter().zip(&ISO_PIXELS).enumerate() {
            let by = i64::from(y) + row as i64;
            let bx = i64::from(x) + jump;
            data.extend((0..n as i64).map(|i| opaque_at(image, bx + i, by).unwrap_or(0)));
        }
        Block {
            x,
            y,
            grid_x,
            grid_y,
            raw_format: ISO_FORMAT,
            format: BlockFormat::Isometric,
            data,
        }
    }

    /// An RLE sub-block at `(x, y)` holding the opaque pixels of `image`
    /// (block space). Every row down to the last opaque one, and at least
    /// `min_rows` rows, is terminated with `(0, 0)` (shipped walls use 32,
    /// `0x2005` floor blocks 15). Runs longer than 255 are split, skips
    /// longer than 255 use `(255, 0)` pairs.
    ///
    /// Fails when `raw_format` is 1 (that is the isometric format), when an
    /// opaque pixel lies left of `x` or above `y`, or when the encoding
    /// exceeds 65535 bytes.
    pub fn rle(
        x: i16,
        y: i16,
        grid_x: u8,
        grid_y: u8,
        raw_format: i16,
        image: &IndexedImage,
        min_rows: usize,
    ) -> Result<Block> {
        if raw_format == ISO_FORMAT {
            return Err(not_representable("format 1 is isometric, not RLE"));
        }
        let (bx, by) = (i64::from(x), i64::from(y));
        let (ox, oy) = (i64::from(image.offset_x), i64::from(image.offset_y));
        let width = image.width as usize;
        let mut rows: Vec<Vec<(usize, u8)>> = Vec::new();
        for r in 0..image.height as usize {
            let row_y = oy + r as i64;
            for c in 0..width {
                let Some(v) = opaque_at(image, ox + c as i64, row_y) else {
                    continue;
                };
                let (dx, dy) = (ox + c as i64 - bx, row_y - by);
                let (Ok(dx), Ok(dy)) = (usize::try_from(dx), usize::try_from(dy)) else {
                    return Err(not_representable(format!(
                        "opaque pixel at ({}, {row_y}) lies left of or above the block origin ({x}, {y})",
                        ox + c as i64
                    )));
                };
                // every row costs 2 bytes and every 255 columns of skip 2:
                // anything this far out cannot fit 65535 bytes
                if dy > MAX_RLE_LEN / 2 || dx > MAX_RLE_LEN / 2 * 255 {
                    return Err(not_representable(format!(
                        "opaque pixel {dx}, {dy} away from the block origin"
                    )));
                }
                if rows.len() <= dy {
                    rows.resize_with(dy + 1, Vec::new);
                }
                if let Some(line) = rows.get_mut(dy) {
                    line.push((dx, v));
                }
            }
        }
        if rows.len() < min_rows {
            if min_rows > MAX_RLE_LEN / 2 {
                return Err(not_representable(format!("{min_rows} RLE rows")));
            }
            rows.resize_with(min_rows, Vec::new);
        }
        let mut data = Vec::new();
        for line in &rows {
            encode_rle_row(line, &mut data);
            data.extend_from_slice(&[0, 0]);
            if data.len() > MAX_RLE_LEN {
                return Err(not_representable("RLE sub-block over 65535 bytes"));
            }
        }
        Ok(Block {
            x,
            y,
            grid_x,
            grid_y,
            raw_format,
            format: BlockFormat::Rle,
            data,
        })
    }

    /// This block (position, grid cell and format kept) with its pixels
    /// replaced by the opaque pixels of `image` (block space). RLE blocks
    /// keep at least as many terminated rows as they had. Errors as
    /// [`Block::rle`].
    pub fn with_pixels(&self, image: &IndexedImage) -> Result<Block> {
        match self.format {
            BlockFormat::Isometric => {
                let mut b = Block::isometric(self.x, self.y, self.grid_x, self.grid_y, image);
                b.raw_format = self.raw_format;
                Ok(b)
            }
            BlockFormat::Rle => Block::rle(
                self.x,
                self.y,
                self.grid_x,
                self.grid_y,
                self.raw_format,
                image,
                rle_rows(&self.data),
            ),
        }
    }
}

/// `(skip, count, literals)` runs for one row's `(x, value)` pixels, sorted
/// by `x` (one per column).
fn encode_rle_row(line: &[(usize, u8)], out: &mut Vec<u8>) {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while let Some(&(start, _)) = line.get(i) {
        let mut end = i + 1;
        while line.get(end).is_some_and(|&(x, _)| x == start + (end - i)) {
            end += 1;
        }
        let mut skip = start - cursor;
        while skip > 255 {
            out.extend_from_slice(&[255, 0]);
            skip -= 255;
        }
        for (n, chunk) in line.get(i..end).unwrap_or(&[]).chunks(255).enumerate() {
            let s = if n == 0 { skip as u8 } else { 0 };
            out.push(s);
            out.push(chunk.len() as u8);
            out.extend(chunk.iter().map(|&(_, v)| v));
        }
        cursor = start + (end - i);
        i = end;
    }
}

/// Number of `(0, 0)` row terminators in RLE data, walked as the decoder does.
fn rle_rows(data: &[u8]) -> usize {
    let (mut pos, mut rows) = (0usize, 0usize);
    while let (Some(&skip), Some(&count)) = (data.get(pos), data.get(pos + 1)) {
        pos += 2;
        if skip == 0 && count == 0 {
            rows += 1;
        } else {
            pos = pos.saturating_add(usize::from(count));
        }
    }
    rows
}
