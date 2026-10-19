//! Palette index remapping of whole sprite and tile files, for the "Convert
//! to palette" tool.
//!
//! An [`IndexTable`] says which palette index replaces each old index. It
//! never creates or removes transparency: index 0 stays 0 and no other index
//! may become 0.
//!
//! * **DT1** and **DC6** files are patched in place. The parser locates every
//!   structure (file, tile and frame headers, sub-block headers, frame
//!   tables) and every encoded pixel stream; only the literal pixel bytes of
//!   those streams change. Isometric DT1 blocks are 256 literal bytes (a
//!   block of any other length draws nothing and is left alone); RLE DT1
//!   blocks are `(skip, count, count literals)` pairs; DC6 frame streams use
//!   `0x80` (end of line), `0x81..=0xFF` (transparent run) and
//!   `0x00..=0x7F` (that many literals). Bytes nothing points at (padding,
//!   DC6 slack, trailing data) are copied as they are. Blocks or frames that
//!   share data are patched once.
//! * **DCC** files are decoded, mapped and re-encoded with the DCC encoder;
//!   the result is checked to decode to exactly the mapped frames. A mapping
//!   never adds colours to a 4x4 cell, so the encoder's four-colour cell
//!   limit always holds.
//!
//! Parsing applies the format's own limits (the DT1/DC6 copy budget of file
//! length plus 1 MiB, the DCC pixel limits), so hostile input cannot make the
//! walk or the output larger than those bounds.

use crate::bytes::u32_at;
use crate::dc6::Dc6;
use crate::dcc::{Dcc, DccDirection};
use crate::dt1::{self, BlockFormat};
use crate::{FormatError, IndexedImage, Result};

/// The kind of file a remap or histogram reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpriteKind {
    Dt1,
    Dc6,
    Dcc,
}

impl SpriteKind {
    /// From the file extension of a game or file path (case-insensitive).
    /// Both `\` and `/` separate folders; a dot inside a folder name is not
    /// an extension.
    pub fn from_path(path: &str) -> Option<SpriteKind> {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        let (_, ext) = name.rsplit_once('.')?;
        [
            ("dt1", SpriteKind::Dt1),
            ("dc6", SpriteKind::Dc6),
            ("dcc", SpriteKind::Dcc),
        ]
        .into_iter()
        .find(|(e, _)| ext.eq_ignore_ascii_case(e))
        .map(|(_, kind)| kind)
    }

    fn format(self) -> &'static str {
        match self {
            SpriteKind::Dt1 => "dt1",
            SpriteKind::Dc6 => "dc6",
            SpriteKind::Dcc => "dcc",
        }
    }
}

/// `table[i]` is the new index for old index `i`. `table[0]` must be 0 and
/// `table[i]` must not be 0 for `i != 0`: transparency is never created or
/// removed. Anything else is `FormatError::Invalid`.
pub type IndexTable = [u8; 256];

/// How many pixels use each index. Transparent skips are not pixels; literal
/// index-0 bytes count under 0.
///
/// DT1 and DC6 count the literal bytes of every block or frame stream, once
/// per block or frame that uses them (blocks that share data count twice,
/// literals a frame clips away still count). DCC counts the opaque pixels of
/// the decoded frames, so its index 0 count is always 0.
pub fn index_histogram(kind: SpriteKind, bytes: &[u8]) -> Result<[u64; 256]> {
    let mut hist = [0u64; 256];
    match kind {
        SpriteKind::Dt1 | SpriteKind::Dc6 => {
            walk(kind, bytes, &mut |start, len, role| {
                if role == Role::Pixels {
                    let run = bytes.get(start..start.saturating_add(len)).unwrap_or(&[]);
                    for &b in run {
                        hist[usize::from(b)] += 1;
                    }
                }
            })?;
        }
        SpriteKind::Dcc => {
            let dcc = Dcc::parse(bytes)?;
            for frame in dcc.directions.iter().flat_map(|d| &d.frames) {
                for (&p, &a) in frame.pixels.iter().zip(&frame.alpha) {
                    if a != 0 {
                        hist[usize::from(p)] += 1;
                    }
                }
            }
        }
    }
    Ok(hist)
}

/// The file with every pixel index `i` replaced by `table[i]`.
/// - DT1 and DC6 are patched in place: every byte that is not a pixel
///   literal (headers, tile/frame tables, block headers, RLE control bytes,
///   padding, trailing bytes) is identical to the input, so an identity
///   table returns the input unchanged.
/// - DCC is decoded, mapped and re-encoded with the existing encoder: the
///   decoded frames equal the mapped input frames; the byte layout may
///   change. A mapping never adds colours to a 4x4 cell, so the cell limit
///   still holds.
///
/// Files `parse` rejects (including the six DT1 4.1 files) are errors. So is
/// a damaged DT1/DC6 whose pixel data overlaps one of its own headers or
/// control bytes, when the table would change such a byte.
pub fn remap_indices(kind: SpriteKind, bytes: &[u8], table: &IndexTable) -> Result<Vec<u8>> {
    check_table(table)?;
    match kind {
        SpriteKind::Dt1 | SpriteKind::Dc6 => patch(kind, bytes, table),
        SpriteKind::Dcc => remap_dcc(bytes, table),
    }
}

fn check_table(table: &IndexTable) -> Result<()> {
    let bad = |offset: usize, detail: String| FormatError::Invalid {
        format: "remap",
        what: "index table",
        offset,
        detail,
    };
    if table[0] != 0 {
        return Err(bad(
            0,
            format!("index 0 (transparent) must stay 0, not {}", table[0]),
        ));
    }
    if let Some(i) = (1..256).find(|&i| table[i] == 0) {
        return Err(bad(
            i,
            format!("index {i} maps to 0, which would make it transparent"),
        ));
    }
    Ok(())
}

/// What a byte range of a DT1/DC6 file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Headers, tables and RLE control bytes: never changed.
    Fixed,
    /// Literal pixel indices.
    Pixels,
}

/// Parses `bytes` and reports every structure as `visit(start, len, role)`.
/// All reported ranges lie inside `bytes` (the parsers bounds-checked them).
fn walk(kind: SpriteKind, bytes: &[u8], visit: &mut dyn FnMut(usize, usize, Role)) -> Result<()> {
    match kind {
        SpriteKind::Dt1 => walk_dt1(bytes, visit),
        SpriteKind::Dc6 => walk_dc6(bytes, visit),
        SpriteKind::Dcc => Ok(()),
    }
}

/// Mirrors `dt1::decode_block`: what it draws is what this reports as pixels.
fn walk_dt1(bytes: &[u8], visit: &mut dyn FnMut(usize, usize, Role)) -> Result<()> {
    let (parsed, layout) = dt1::parse_located(bytes)?;
    let (file_header, tile_header, block_header) = dt1::HEADER_LENS;
    visit(0, file_header, Role::Fixed);
    for &at in &layout.tile_headers {
        visit(at, tile_header, Role::Fixed);
    }
    for &at in &layout.block_headers {
        visit(at, block_header, Role::Fixed);
    }
    let blocks = parsed.tiles.iter().flat_map(|t| &t.blocks);
    for (block, &at) in blocks.zip(&layout.block_data) {
        let data = block.data.as_slice();
        match block.format {
            BlockFormat::Isometric => {
                // any other length draws nothing (see `Tile::render`)
                if data.len() == 256 {
                    visit(at, 256, Role::Pixels);
                }
            }
            BlockFormat::Rle => {
                let mut pos = 0usize;
                while let (Some(&skip), Some(&count)) = (data.get(pos), data.get(pos + 1)) {
                    visit(at + pos, 2, Role::Fixed);
                    pos += 2;
                    if skip == 0 && count == 0 {
                        continue;
                    }
                    let end = (pos + usize::from(count)).min(data.len());
                    if end > pos {
                        visit(at + pos, end - pos, Role::Pixels);
                    }
                    pos = end;
                }
            }
        }
    }
    Ok(())
}

/// Mirrors `Dc6Frame::render`'s opcode walk.
fn walk_dc6(bytes: &[u8], visit: &mut dyn FnMut(usize, usize, Role)) -> Result<()> {
    const HEADER_LEN: usize = 24;
    const FRAME_HEADER_LEN: usize = 32;
    let parsed = Dc6::parse(bytes)?;
    visit(0, HEADER_LEN, Role::Fixed);
    visit(HEADER_LEN, 4 * parsed.frames.len(), Role::Fixed);
    for (i, frame) in parsed.frames.iter().enumerate() {
        let table_at = HEADER_LEN + 4 * i;
        // `Dc6::parse` read this very entry, so it exists
        let off = u32_at(bytes, table_at).ok_or(FormatError::Truncated {
            format: "dc6",
            offset: table_at,
            needed: 4,
        })? as usize;
        visit(off, FRAME_HEADER_LEN, Role::Fixed);
        let at = off + FRAME_HEADER_LEN;
        let data = frame.data.as_slice();
        let mut pos = 0usize;
        while let Some(&op) = data.get(pos) {
            visit(at + pos, 1, Role::Fixed);
            pos += 1;
            if op & 0x80 == 0 {
                let end = (pos + usize::from(op)).min(data.len());
                if end > pos {
                    visit(at + pos, end - pos, Role::Pixels);
                }
                pos = end;
            }
        }
    }
    Ok(())
}

fn patch(kind: SpriteKind, bytes: &[u8], table: &IndexTable) -> Result<Vec<u8>> {
    const FIXED: u8 = 1;
    const PIXEL: u8 = 2;
    let mut roles = vec![0u8; bytes.len()];
    walk(kind, bytes, &mut |start, len, role| {
        let bit = match role {
            Role::Fixed => FIXED,
            Role::Pixels => PIXEL,
        };
        if let Some(range) = roles.get_mut(start..start.saturating_add(len)) {
            for r in range {
                *r |= bit;
            }
        }
    })?;
    let mut out = bytes.to_vec();
    for (offset, (byte, &role)) in out.iter_mut().zip(&roles).enumerate() {
        if role & PIXEL == 0 {
            continue;
        }
        let new = table[usize::from(*byte)];
        if new == *byte {
            continue;
        }
        if role & FIXED != 0 {
            return Err(FormatError::Invalid {
                format: kind.format(),
                what: "pixel data",
                offset,
                detail: "a pixel byte is also part of a header or an RLE control code; \
                         remapping it would change the file's structure"
                    .to_owned(),
            });
        }
        *byte = new;
    }
    Ok(out)
}

fn map_image(img: &IndexedImage, table: &IndexTable) -> IndexedImage {
    let mut out = img.clone();
    for p in &mut out.pixels {
        *p = table[usize::from(*p)];
    }
    out
}

fn remap_dcc(bytes: &[u8], table: &IndexTable) -> Result<Vec<u8>> {
    let parsed = Dcc::parse(bytes)?;
    let mapped = Dcc {
        version: parsed.version,
        frames_per_direction: parsed.frames_per_direction,
        directions: parsed
            .directions
            .iter()
            .map(|d| DccDirection {
                frames: d.frames.iter().map(|f| map_image(f, table)).collect(),
                bbox: d.bbox,
            })
            .collect(),
    };
    let out = mapped.to_bytes()?;
    let back = Dcc::parse(&out)?;
    let same = back.directions.len() == mapped.directions.len()
        && back
            .directions
            .iter()
            .zip(&mapped.directions)
            .all(|(a, b)| a.frames == b.frames);
    if !same {
        return Err(FormatError::Invalid {
            format: "dcc",
            what: "re-encoded frames",
            offset: 0,
            detail: "the re-encoded file does not decode to the remapped frames".to_owned(),
        });
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
