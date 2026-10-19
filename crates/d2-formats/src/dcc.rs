//! DCC sprites (`*.dcc`): the game's bit-packed, cell-delta animation format.
//! Parsing decodes every direction and frame eagerly (the decoder is the
//! two-stage algorithm of `docs/specs/dc6-dcc.md` section 2.3).
//!
//! ```text
//! file:      u8 signature (0x74)  u8 version (6)  u8 directions
//!            u32 frames per direction  u32 tag  u32 final DC6 size
//!            u32 x directions   absolute offset of each direction
//! direction: LSB-first bit stream: u32 coded size, 2-bit compression flag,
//!            seven 4-bit width codes (variable0, width, height, x offset,
//!            y offset, optional bytes, coded bytes), then per frame header
//!            fields of those widths + 1 bottom-up bit; optional bytes;
//!            20-bit sizes of the equal-cell / pixel-mask / encoding-type /
//!            raw-pixel streams; a 256-bit palette-usage key; then the five
//!            streams back to back (the last one, pixel codes and
//!            displacements, runs to the end of the direction).
//! ```
//!
//! # Decoding
//!
//! Each direction is a grid of 4x4 cells over the union box of its frames.
//! Stage 1 walks every frame's cells and, for each cell that changed, pops up
//! to four colour codes into a *pixel buffer* entry: codes are decoded in
//! ascending order and assigned to the changed colour slots in **reverse**
//! order, missing ones zero-filled (`dcc.c:989-1001`; this is the standard
//! DCC algorithm). Stage 2 expands each entry into pixels (0, 1 or 2 bits per
//! pixel depending on how many distinct colours the entry holds). A cell the
//! stream marks "unchanged" keeps the pixels of the same buffer cell from the
//! previous frame when its size is unchanged, and is cleared otherwise.
//! Palette index 0 is the transparent key (alpha 0).
//!
//! The reference editor instead keeps one direction-sized bitmap alive across
//! frames and copies unchanged cells between positions with alpha blending,
//! which leaves stale pixels behind when a transparent source pixel meets old
//! data. On a 1-in-50 sample of both corpora (144,064 frames) the two models
//! agree on 99.85% of frames; the rest differ by 1 to 9 stale pixels at frame
//! edges (the dozen inspected are Assassin animations), where this decoder shows the clean
//! cell. The `corpus_dcc_matches_reference_stage2_model` test re-implements
//! the reference model and measures this.
//!
//! # Frames and boxes
//!
//! Every frame is returned cropped to its own box, with `offset_x` /
//! `offset_y` locating pixel (0,0) relative to the sprite anchor.
//! [`DccDirection::bbox`] is the union of the frame boxes, so every frame
//! lies inside it. `bottom_up` frames are not flipped (nothing in the
//! shipped data sets the bit).
//!
//! # Leniency and limits
//!
//! Signature, version and the unused header words are not checked, and
//! unconsumed bitstream tails are tolerated (the strictness the reference
//! applies to those is available to tests via `parse_checked`). Everything is
//! bounds-checked, and: at most 32 directions and 1024 frames per direction;
//! a direction box of at most 2048x2048; at most 32 Mi decoded pixels per
//! file (the largest shipped file decodes to 7.2 Mi, its largest box is
//! 445x324); at most 1 Mi pixel-buffer entries per direction.

use crate::bytes::u32_at;
use crate::{FormatError, IndexedImage, Result};

const FORMAT: &str = "dcc";
const WIDTH_TABLE: [u32; 16] = [0, 1, 2, 4, 6, 8, 10, 12, 14, 16, 20, 24, 26, 28, 30, 32];
const POPCOUNT: [usize; 16] = [0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];

const MAX_DIRECTIONS: usize = 32;
const MAX_FRAMES: u32 = 1024;
const MAX_BOX_DIM: i64 = 2048;
const MAX_TOTAL_PIXELS: u64 = 32 << 20;
const MAX_PB_ENTRIES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dcc {
    pub version: u8,
    pub frames_per_direction: u32,
    pub directions: Vec<DccDirection>,
}

/// An axis-aligned box relative to the sprite anchor (y down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DccBox {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DccDirection {
    /// One image per frame, each cropped to its own box.
    pub frames: Vec<IndexedImage>,
    /// Union of the frame boxes (non-empty for a decoded direction).
    pub bbox: DccBox,
}

fn truncated(offset: usize, needed: usize) -> FormatError {
    FormatError::Truncated {
        format: FORMAT,
        offset,
        needed,
    }
}

fn invalid(what: &'static str, offset: usize, detail: impl Into<String>) -> FormatError {
    FormatError::Invalid {
        format: FORMAT,
        what,
        offset,
        detail: detail.into(),
    }
}

/// LSB-first bit reader over `data[start_byte..]`, limited to `size` bits.
#[derive(Clone)]
struct Bits<'a> {
    data: &'a [u8],
    /// Absolute bit position of the stream start inside `data`.
    start: usize,
    size: usize,
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], start: usize, size: usize) -> Self {
        Self {
            data,
            start,
            size,
            pos: 0,
        }
    }

    /// Reads `n <= 32` bits, or `None` past the end of the stream.
    fn read(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 || self.pos.checked_add(n as usize)? > self.size {
            return None;
        }
        let mut value = 0u64;
        let mut got = 0u32;
        let mut at = self.start + self.pos;
        while got < n {
            let byte = u64::from(*self.data.get(at / 8)?);
            let shift = (at % 8) as u32;
            let take = (8 - shift).min(n - got);
            let chunk = (byte >> shift) & ((1u64 << take) - 1);
            value |= chunk << got;
            got += take;
            at += take as usize;
        }
        self.pos += n as usize;
        Some(value as u32)
    }

    /// Reads `n` bits and sign-extends from bit `n - 1`.
    fn read_signed(&mut self, n: u32) -> Option<i64> {
        let v = i64::from(self.read(n)?);
        if n > 0 && n < 32 && v & (1 << (n - 1)) != 0 {
            Some(v - (1 << n))
        } else if n == 32 {
            Some(i64::from(v as u32 as i32))
        } else {
            Some(v)
        }
    }

    /// Absolute bit position (within `data`) of the read cursor.
    fn absolute(&self) -> usize {
        self.start + self.pos
    }
}

struct FrameHeader {
    xmin: i64,
    ymin: i64,
    width: i64,
    height: i64,
    optional_bytes: u32,
}

/// One entry of the stage-1 pixel buffer.
struct Entry {
    frame: u32,
    /// Index of the cell inside its frame's grid (`x + y * columns`).
    cell: u32,
    val: [u8; 4],
}

impl Dcc {
    /// Decodes every direction and frame.
    pub fn parse(bytes: &[u8]) -> Result<Dcc> {
        parse_checked(bytes).map(|(dcc, _)| dcc)
    }
}

/// `Dcc::parse` plus the reference's end-of-stream consistency checks as
/// messages (stream not fully consumed, ...). Valid game files produce none.
pub(crate) fn parse_checked(bytes: &[u8]) -> Result<(Dcc, Vec<String>)> {
    let need_header = 15usize;
    if bytes.len() < need_header {
        return Err(truncated(bytes.len(), need_header - bytes.len()));
    }
    let version = bytes.get(1).copied().unwrap_or(0);
    let dir_count = usize::from(bytes.get(2).copied().unwrap_or(0));
    let frames_per_direction = u32_at(bytes, 3).ok_or_else(|| truncated(3, 4))?;
    if dir_count > MAX_DIRECTIONS {
        return Err(invalid(
            "direction count",
            2,
            format!("{dir_count} directions (limit {MAX_DIRECTIONS})"),
        ));
    }
    if frames_per_direction > MAX_FRAMES {
        return Err(invalid(
            "frames per direction",
            3,
            format!("{frames_per_direction} frames (limit {MAX_FRAMES})"),
        ));
    }
    let mut offsets = Vec::with_capacity(dir_count);
    for i in 0..dir_count {
        let at = need_header + 4 * i;
        offsets.push(u32_at(bytes, at).ok_or_else(|| truncated(at, 4))? as usize);
    }
    let mut warnings = Vec::new();
    let mut budget = MAX_TOTAL_PIXELS;
    let mut directions = Vec::with_capacity(dir_count);
    for (d, &start) in offsets.iter().enumerate() {
        let end = match offsets.get(d + 1) {
            Some(&next) if next >= start => next,
            _ => bytes.len(),
        };
        if start > end || end > bytes.len() {
            return Err(invalid(
                "direction offset",
                need_header + 4 * d,
                format!("direction {d} at {start:#x}..{end:#x} is outside the file"),
            ));
        }
        let data = bytes.get(start..end).unwrap_or(&[]);
        let mut local = Vec::new();
        let dir = decode_direction(data, frames_per_direction, &mut budget, &mut local)
            .map_err(|e| with_direction(e, d, start))?;
        warnings.extend(local.into_iter().map(|w| format!("direction {d}: {w}")));
        directions.push(dir);
    }
    Ok((
        Dcc {
            version,
            frames_per_direction,
            directions,
        },
        warnings,
    ))
}

/// Re-bases the offset of an error from a direction-local one to a file one.
fn with_direction(e: FormatError, d: usize, base: usize) -> FormatError {
    match e {
        FormatError::Truncated {
            format,
            offset,
            needed,
        } => FormatError::Truncated {
            format,
            offset: base.saturating_add(offset / 8),
            needed: needed.div_ceil(8),
        },
        FormatError::Invalid {
            format,
            what,
            offset,
            detail,
        } => FormatError::Invalid {
            format,
            what,
            offset: base.saturating_add(offset / 8),
            detail: format!("direction {d}: {detail}"),
        },
        other => other,
    }
}

/// Error helpers for the direction decoder; offsets are *bit* offsets within
/// the direction (`with_direction` converts them).
fn eof(bs: &Bits<'_>, _what: &'static str) -> FormatError {
    truncated(bs.absolute(), 1)
}

/// Sizes of the cells covering one axis of a frame: `(start relative to the
/// direction box, length)`. The first cell ends on the 4-pixel grid, and a
/// final sliver of one pixel is merged into the previous cell.
fn axis_cells(frame_min: i64, dir_min: i64, frame_len: i64) -> Vec<(i64, i64)> {
    let rel = frame_min - dir_min;
    let first = 4 - rel.rem_euclid(4);
    let count = if frame_len - first <= 1 {
        1
    } else {
        let tmp = frame_len - first - 1;
        let mut n = 2 + tmp / 4;
        if tmp % 4 == 0 {
            n -= 1;
        }
        n
    };
    let mut out = Vec::with_capacity(count as usize);
    if count == 1 {
        out.push((rel, frame_len));
        return out;
    }
    let mut start = rel;
    for k in 0..count {
        let len = if k == 0 {
            first
        } else if k == count - 1 {
            frame_len - first - 4 * (count - 2)
        } else {
            4
        };
        out.push((start, len));
        start += len;
    }
    out
}

/// Cell boundaries of one frame: `(start relative to the direction box,
/// length)` per column and per row.
struct FrameGrid {
    cols: Vec<(i64, i64)>,
    rows: Vec<(i64, i64)>,
}

/// Everything stage 1 and stage 2 share for one direction.
struct Direction<'a> {
    headers: Vec<FrameHeader>,
    x0: i64,
    y0: i64,
    width: i64,
    height: i64,
    grids: Vec<FrameGrid>,
    /// Columns of the direction's own 4x4 cell grid.
    buf_cols: i64,
    buf_cells: usize,
    pixel_values: [u8; 256],
    equal_cell: Bits<'a>,
    pixel_mask: Bits<'a>,
    encoding_type: Bits<'a>,
    raw_pixel: Bits<'a>,
    pcd: Bits<'a>,
}

impl Direction<'_> {
    /// Index of the direction-grid cell that frame cell `(cx, cy)` of frame
    /// `f` belongs to.
    fn buffer_slot(&self, f: usize, cx: usize, cy: usize) -> Result<usize> {
        let h = self
            .headers
            .get(f)
            .ok_or_else(|| invalid("frame", self.pcd.absolute(), "frame index out of range"))?;
        let col = (h.xmin - self.x0) / 4 + cx as i64;
        let row = (h.ymin - self.y0) / 4 + cy as i64;
        usize::try_from(col + row * self.buf_cols)
            .ok()
            .filter(|&s| s < self.buf_cells)
            .ok_or_else(|| {
                invalid(
                    "cell index",
                    self.pcd.absolute(),
                    "cell outside the direction grid",
                )
            })
    }
}

fn read_direction<'a>(
    data: &'a [u8],
    frames: u32,
    budget: &mut u64,
) -> Result<Option<Direction<'a>>> {
    let total_bits = data.len() * 8;
    let mut bs = Bits::new(data, 0, total_bits);

    let _outsize_coded = bs.read(32).ok_or_else(|| eof(&bs, "coded size"))?;
    let flag = bs.read(2).ok_or_else(|| eof(&bs, "compression flag"))?;
    let mut widths = [0u32; 7];
    for slot in &mut widths {
        let code = bs.read(4).ok_or_else(|| eof(&bs, "field width"))?;
        *slot = WIDTH_TABLE[(code & 15) as usize];
    }
    let [
        w_var0,
        w_width,
        w_height,
        w_xoff,
        w_yoff,
        w_optional,
        w_coded,
    ] = widths;

    // frame headers
    let mut headers: Vec<FrameHeader> = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for _ in 0..frames {
        let _variable0 = bs.read(w_var0).ok_or_else(|| eof(&bs, "frame header"))?;
        let width = i64::from(bs.read(w_width).ok_or_else(|| eof(&bs, "frame width"))?);
        let height = i64::from(bs.read(w_height).ok_or_else(|| eof(&bs, "frame height"))?);
        let xoff = bs
            .read_signed(w_xoff)
            .ok_or_else(|| eof(&bs, "frame x offset"))?;
        let yoff = bs
            .read_signed(w_yoff)
            .ok_or_else(|| eof(&bs, "frame y offset"))?;
        let optional = bs
            .read(w_optional)
            .ok_or_else(|| eof(&bs, "optional bytes"))?;
        let _coded = bs.read(w_coded).ok_or_else(|| eof(&bs, "coded bytes"))?;
        let bottom_up = bs.read(1).ok_or_else(|| eof(&bs, "bottom-up bit"))? != 0;
        let (ymin, ymax) = if bottom_up {
            (yoff, yoff + height - 1)
        } else {
            (yoff - height + 1, yoff)
        };
        x0 = x0.min(xoff);
        y0 = y0.min(ymin);
        x1 = x1.max(xoff + width - 1);
        y1 = y1.max(ymax);
        headers.push(FrameHeader {
            xmin: xoff,
            ymin,
            width,
            height,
            optional_bytes: optional,
        });
    }
    if headers.is_empty() {
        return Ok(None);
    }
    let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
    if width < 1 || height < 1 || width > MAX_BOX_DIM || height > MAX_BOX_DIM {
        return Err(invalid(
            "direction box",
            bs.absolute(),
            format!("{width}x{height} pixels is outside 1..={MAX_BOX_DIM}"),
        ));
    }
    let frame_pixels: u64 = headers
        .iter()
        .map(|h| (h.width.max(0) as u64) * (h.height.max(0) as u64))
        .sum();
    if frame_pixels > *budget {
        return Err(invalid(
            "decoded size",
            bs.absolute(),
            "decoded pixels exceed the per-file limit",
        ));
    }
    *budget -= frame_pixels;

    // optional bytes: skipped (byte aligned), never used
    let optional_total: u64 = headers.iter().map(|h| u64::from(h.optional_bytes)).sum();
    if optional_total > 0 {
        let aligned = bs.pos.div_ceil(8) * 8;
        bs.pos = aligned
            .checked_add((optional_total as usize).saturating_mul(8))
            .filter(|&e| e <= total_bits)
            .ok_or_else(|| eof(&bs, "optional bytes"))?;
    }

    // stream sizes
    let equal_size = if flag & 2 != 0 {
        bs.read(20).ok_or_else(|| eof(&bs, "equal cell size"))? as usize
    } else {
        0
    };
    let mask_size = bs.read(20).ok_or_else(|| eof(&bs, "pixel mask size"))? as usize;
    let (encoding_size, raw_size) = if flag & 1 != 0 {
        let e = bs.read(20).ok_or_else(|| eof(&bs, "encoding type size"))? as usize;
        let r = bs.read(20).ok_or_else(|| eof(&bs, "raw pixel size"))? as usize;
        (e, r)
    } else {
        (0, 0)
    };

    // palette usage key: code n is the n-th set bit
    let mut pixel_values = [0u8; 256];
    let mut used = 0usize;
    for i in 0..=255u8 {
        if bs.read(1).ok_or_else(|| eof(&bs, "pixel key"))? != 0 {
            if let Some(slot) = pixel_values.get_mut(used) {
                *slot = i;
            }
            used += 1;
        }
    }

    // the five sub-streams follow back to back
    let mut glb = bs.absolute();
    let mut take = |size: usize, what: &'static str| -> Result<Bits<'a>> {
        let start = glb;
        glb = glb
            .checked_add(size)
            .filter(|&e| e <= total_bits)
            .ok_or_else(|| {
                invalid(
                    "bit stream",
                    start,
                    format!("{what} stream of {size} bits leaves the direction"),
                )
            })?;
        Ok(Bits::new(data, start, size))
    };
    let equal_cell = take(equal_size, "equal cell")?;
    let pixel_mask = take(mask_size, "pixel mask")?;
    let encoding_type = take(encoding_size, "encoding type")?;
    let raw_pixel = take(raw_size, "raw pixel")?;
    let pcd = Bits::new(data, glb, total_bits - glb);

    let buf_cols = 1 + (width - 1) / 4;
    let buf_rows = 1 + (height - 1) / 4;
    let grids = headers
        .iter()
        .map(|h| FrameGrid {
            cols: axis_cells(h.xmin, x0, h.width),
            rows: axis_cells(h.ymin, y0, h.height),
        })
        .collect();
    Ok(Some(Direction {
        headers,
        x0,
        y0,
        width,
        height,
        grids,
        buf_cols,
        buf_cells: (buf_cols * buf_rows) as usize,
        pixel_values,
        equal_cell,
        pixel_mask,
        encoding_type,
        raw_pixel,
        pcd,
    }))
}

/// Stage 1: walks every cell of every frame and builds the pixel buffer.
fn stage1(dir: &mut Direction<'_>) -> Result<Vec<Entry>> {
    let early = |what: &'static str, bs: &Bits<'_>| invalid(what, bs.absolute(), "ends early");
    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Vec<Option<usize>> = vec![None; dir.buf_cells];
    for f in 0..dir.grids.len() {
        let (columns, row_count) = dir
            .grids
            .get(f)
            .map_or((0, 0), |g| (g.cols.len(), g.rows.len()));
        for cy in 0..row_count {
            for cx in 0..columns {
                let slot = dir.buffer_slot(f, cx, cy)?;
                let old = current.get(slot).copied().flatten();
                let mask = if old.is_some() {
                    let unchanged = dir.equal_cell.size != 0
                        && dir
                            .equal_cell
                            .read(1)
                            .ok_or_else(|| early("equal cell stream", &dir.equal_cell))?
                            != 0;
                    if unchanged {
                        continue;
                    }
                    dir.pixel_mask
                        .read(4)
                        .ok_or_else(|| early("pixel mask stream", &dir.pixel_mask))?
                        as usize
                } else {
                    0x0F
                };
                let nb_pix = POPCOUNT.get(mask).copied().unwrap_or(0);
                let raw = nb_pix != 0
                    && dir.encoding_type.size != 0
                    && dir
                        .encoding_type
                        .read(1)
                        .ok_or_else(|| early("encoding type stream", &dir.encoding_type))?
                        != 0;
                let mut stack = [0u32; 4];
                let mut last = 0u32;
                let mut decoded = 0usize;
                for stacked in stack.iter_mut().take(nb_pix) {
                    let value = if raw {
                        dir.raw_pixel
                            .read(8)
                            .ok_or_else(|| early("raw pixel stream", &dir.raw_pixel))?
                    } else {
                        let mut v = last;
                        let mut disp = dir
                            .pcd
                            .read(4)
                            .ok_or_else(|| early("pixel code stream", &dir.pcd))?;
                        v = v.wrapping_add(disp);
                        while disp == 15 {
                            disp = dir
                                .pcd
                                .read(4)
                                .ok_or_else(|| early("pixel code stream", &dir.pcd))?;
                            v = v.wrapping_add(disp);
                        }
                        v
                    };
                    if value == last {
                        // equal to the previous code: stop, discarding it
                        break;
                    }
                    *stacked = value;
                    last = value;
                    decoded += 1;
                }
                if entries.len() >= MAX_PB_ENTRIES {
                    return Err(invalid(
                        "pixel buffer",
                        dir.pcd.absolute(),
                        "too many pixel buffer entries",
                    ));
                }
                let old_val = old.and_then(|i| entries.get(i)).map_or([0u8; 4], |e| e.val);
                // changed slots pop the decoded codes in reverse order,
                // zero once the stack is empty; the others keep their code
                let mut val = [0u8; 4];
                let mut next = decoded;
                for (i, v) in val.iter_mut().enumerate() {
                    *v = if mask & (1 << i) != 0 {
                        if next > 0 {
                            next -= 1;
                            stack.get(next).copied().unwrap_or(0) as u8
                        } else {
                            0
                        }
                    } else {
                        old_val.get(i).copied().unwrap_or(0)
                    };
                }
                entries.push(Entry {
                    frame: f as u32,
                    cell: (cx + cy * columns) as u32,
                    val,
                });
                if let Some(c) = current.get_mut(slot) {
                    *c = Some(entries.len() - 1);
                }
            }
        }
    }
    // codes -> palette indices
    for e in &mut entries {
        for v in &mut e.val {
            *v = dir.pixel_values.get(usize::from(*v)).copied().unwrap_or(0);
        }
    }
    Ok(entries)
}

/// The pixels of one buffer cell as last shown (cells are at most 5x5).
#[derive(Clone, Copy)]
struct Stored {
    w: usize,
    h: usize,
    px: [u8; 25],
}

/// Stage 2: expands the entries into pixels and composes the frames.
fn stage2(dir: &mut Direction<'_>, entries: &[Entry]) -> Result<Vec<IndexedImage>> {
    let mut stored: Vec<Option<Stored>> = vec![None; dir.buf_cells];
    let mut cursor = 0usize;
    let mut out_frames = Vec::with_capacity(dir.headers.len());
    for f in 0..dir.headers.len() {
        let (Some(h), Some(grid)) = (dir.headers.get(f), dir.grids.get(f)) else {
            break;
        };
        let (fw, fh) = (h.width.max(0) as usize, h.height.max(0) as usize);
        let ox = i32::try_from(h.xmin).map_err(|_| invalid("frame offset", 0, "out of range"))?;
        let oy = i32::try_from(h.ymin).map_err(|_| invalid("frame offset", 0, "out of range"))?;
        let mut image = IndexedImage::new(fw as u32, fh as u32, ox, oy);
        let (frame_x, frame_y) = (h.xmin - dir.x0, h.ymin - dir.y0);
        let columns = grid.cols.len();
        for (cy, &(ry, ch)) in grid.rows.iter().enumerate() {
            for (cx, &(rx, cw)) in grid.cols.iter().enumerate() {
                let slot = dir.buffer_slot(f, cx, cy)?;
                let (cw, ch) = (cw.max(0) as usize, ch.max(0) as usize);
                let mut content = Stored {
                    w: cw,
                    h: ch,
                    px: [0; 25],
                };
                let hit = entries
                    .get(cursor)
                    .filter(|e| e.frame == f as u32 && e.cell == (cx + cy * columns) as u32);
                if let Some(e) = hit {
                    cursor += 1;
                    let val = e.val;
                    if val[0] == val[1] {
                        content.px = [val[0]; 25];
                    } else {
                        let nb_bit = if val[1] == val[2] { 1 } else { 2 };
                        for y in 0..ch {
                            for x in 0..cw {
                                let code = dir.pcd.read(nb_bit).ok_or_else(|| {
                                    invalid("pixel code stream", dir.pcd.absolute(), "ends early")
                                })?;
                                if let Some(p) = content.px.get_mut(y * 5 + x) {
                                    *p = val.get(code as usize).copied().unwrap_or(0);
                                }
                            }
                        }
                    }
                } else if let Some(Some(prev)) = stored.get(slot)
                    && prev.w == cw
                    && prev.h == ch
                {
                    // "unchanged" cell of the same size keeps its pixels;
                    // any other size starts cleared
                    content = *prev;
                }
                // blit into the frame image: the cell start is relative to
                // the direction box, the image starts at the frame box
                let fx = (rx - frame_x) as usize;
                let fy = (ry - frame_y) as usize;
                for y in 0..ch {
                    for x in 0..cw {
                        let v = content.px.get(y * 5 + x).copied().unwrap_or(0);
                        let i = (fy + y) * fw + fx + x;
                        if let (Some(p), Some(a)) =
                            (image.pixels.get_mut(i), image.alpha.get_mut(i))
                        {
                            *p = v;
                            *a = if v == 0 { 0 } else { 255 };
                        }
                    }
                }
                if let Some(s) = stored.get_mut(slot) {
                    *s = Some(content);
                }
            }
        }
        out_frames.push(image);
    }
    Ok(out_frames)
}

fn decode_direction(
    data: &[u8],
    frames: u32,
    budget: &mut u64,
    warnings: &mut Vec<String>,
) -> Result<DccDirection> {
    let Some(mut dir) = read_direction(data, frames, budget)? else {
        return Ok(DccDirection {
            frames: Vec::new(),
            bbox: DccBox::default(),
        });
    };
    let entries = stage1(&mut dir)?;
    let out_frames = stage2(&mut dir, &entries)?;

    // the reference's end-of-stream consistency checks
    let streams = [
        ("equal cell", &dir.equal_cell),
        ("pixel mask", &dir.pixel_mask),
        ("encoding type", &dir.encoding_type),
        ("raw pixel", &dir.raw_pixel),
    ];
    for (name, bs) in streams {
        if bs.pos != bs.size {
            warnings.push(format!(
                "{name} stream: {} of {} bits read",
                bs.pos, bs.size
            ));
        }
    }
    if dir.pcd.pos + 7 < dir.pcd.size {
        warnings.push(format!(
            "pixel code stream: {} of {} bits read",
            dir.pcd.pos, dir.pcd.size
        ));
    }

    let out_of_range = || invalid("direction box", 0, "out of range");
    Ok(DccDirection {
        frames: out_frames,
        bbox: DccBox {
            x: i32::try_from(dir.x0).map_err(|_| out_of_range())?,
            y: i32::try_from(dir.y0).map_err(|_| out_of_range())?,
            width: dir.width as u32,
            height: dir.height as u32,
        },
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
