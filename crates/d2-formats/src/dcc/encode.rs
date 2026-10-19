//! The DCC encoder: `Dcc::from_frames` / `Dcc::to_bytes`.
//!
//! # What the encoder writes
//!
//! A version-6 file with one direction block per direction. Per direction:
//!
//! * a header whose seven field widths are the smallest the values need
//!   (bit-width table codes), one frame header per frame (`bottom_up` always
//!   0, so `y offset` is the frame's bottom row) and no optional bytes;
//! * `coded bytes` per frame and `outsize coded` per direction are the length
//!   of the DC6 encoding of the frames ([`crate::dc6::Dc6Frame::from_image`]),
//!   the figures the game uses to size its RLE buffers;
//! * the four optional streams are chosen **per direction by trial**: the
//!   direction is encoded with every combination of the equal-cell stream
//!   (flag bit 1) and the encoding-type + raw-pixel streams (flag bit 0) and
//!   the smallest result is kept. A stream whose size does not fit its 20-bit
//!   field rules that combination out.
//!
//! # Cell strategy
//!
//! The direction is cut into 4x4 cells exactly as the decoder does
//! (`axis_cells`). For every cell of every frame, in decoding order:
//!
//! 1. A cell the direction has not shown yet always gets a pixel-buffer entry
//!    (implicit mask `0xF`).
//! 2. A cell shown before is compared with what the decoder would keep there
//!    (the previous pixels, or a cleared cell when the cell size changed). If
//!    they are equal and the equal-cell stream is on, one `1` bit is written.
//! 3. Otherwise an entry is written. Its four colour slots are chosen from
//!    two candidates: the canonical layout (the cell's non-zero colour codes
//!    in descending order, zero filled, always possible) and the slots of the
//!    previous entry of that cell when they already cover the cell's colours.
//!    For each, the smallest mask of slots that must change is tried as well
//!    as the full mask; the cheapest legal combination (counting mask,
//!    encoding-type bit, code bytes and pixel bits) wins. A code list is legal
//!    when no code equals its predecessor (the decoder stops there) and the
//!    codes of the highest slots are zero (the decoder zero-fills them);
//!    codes are sent as nibble deltas or as raw bytes, whichever is shorter
//!    when the raw streams are on.
//! 4. Pixels follow in the shared pixel-code stream with 0, 1 or 2 bits each.
//!
//! The decoder shows a cell with at most four colours. Cells with more are
//! reduced first ([`Dcc::from_frames_with`]): transparency is kept, then the
//! most frequent colours, and each remaining pixel moves to the nearest kept
//! colour. Everything else is lossless: `parse(to_bytes(x))` returns exactly
//! the frames [`Dcc::from_frames`] stored in `x`.
//!
//! Zero-sized frames are stored as one transparent pixel (the format cannot
//! describe an empty frame).

use super::{
    Dcc, DccBox, DccDirection, FORMAT, MAX_BOX_DIM, MAX_DIRECTIONS, MAX_FRAMES, WIDTH_TABLE,
    axis_cells, invalid,
};
use crate::dc6::Dc6Frame;
use crate::{FormatError, IndexedImage, Result};

/// A stream size must fit the 20-bit fields of the direction header.
const MAX_STREAM_BITS: usize = (1 << 20) - 1;

// ---------------------------------------------------------------------------
// bit writer

/// LSB-first bit writer, the inverse of the decoder's `Bits`.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    len: usize,
}

impl BitWriter {
    fn put(&mut self, value: u64, n: u32) {
        let mut v = value;
        let mut left = n;
        while left > 0 {
            let shift = (self.len % 8) as u32;
            if shift == 0 {
                self.bytes.push(0);
            }
            let take = (8 - shift).min(left);
            let chunk = (v & ((1u64 << take) - 1)) as u8;
            if let Some(last) = self.bytes.last_mut() {
                *last |= chunk << shift;
            }
            v >>= take;
            left -= take;
            self.len += take as usize;
        }
    }

    fn append(&mut self, other: &BitWriter) {
        let full = other.len / 8;
        for &b in other.bytes.iter().take(full) {
            self.put(u64::from(b), 8);
        }
        let rest = (other.len % 8) as u32;
        if rest > 0 {
            let last = other.bytes.get(full).copied().unwrap_or(0);
            self.put(u64::from(last), rest);
        }
    }
}

// ---------------------------------------------------------------------------
// geometry

struct FrameGeo {
    xmin: i64,
    ymin: i64,
    width: i64,
    height: i64,
}

/// One cell of one frame.
#[derive(Clone, Copy)]
struct Cell {
    /// Index into the direction's 4x4 grid.
    slot: usize,
    /// Position of the cell inside the frame image.
    fx: usize,
    fy: usize,
    w: usize,
    h: usize,
}

struct Geometry {
    x0: i64,
    y0: i64,
    width: i64,
    height: i64,
    frames: Vec<FrameGeo>,
    /// `cells[frame]`, in decoding order (rows, then columns).
    cells: Vec<Vec<Cell>>,
    buf_cells: usize,
}

fn geometry(frames: &[IndexedImage]) -> Result<Geometry> {
    let geo: Vec<FrameGeo> = frames
        .iter()
        .map(|f| FrameGeo {
            xmin: i64::from(f.offset_x),
            ymin: i64::from(f.offset_y),
            width: i64::from(f.width),
            height: i64::from(f.height),
        })
        .collect();
    let x0 = geo.iter().map(|f| f.xmin).min().unwrap_or(0);
    let y0 = geo.iter().map(|f| f.ymin).min().unwrap_or(0);
    let x1 = geo.iter().map(|f| f.xmin + f.width - 1).max().unwrap_or(0);
    let y1 = geo.iter().map(|f| f.ymin + f.height - 1).max().unwrap_or(0);
    let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
    if width < 1 || height < 1 || width > MAX_BOX_DIM || height > MAX_BOX_DIM {
        return Err(invalid(
            "direction box",
            0,
            format!("{width}x{height} pixels is outside 1..={MAX_BOX_DIM}"),
        ));
    }
    let buf_cols = 1 + (width - 1) / 4;
    let buf_rows = 1 + (height - 1) / 4;
    let buf_cells = (buf_cols * buf_rows) as usize;
    let mut cells = Vec::with_capacity(geo.len());
    for f in &geo {
        let cols = axis_cells(f.xmin, x0, f.width);
        let rows = axis_cells(f.ymin, y0, f.height);
        let (frame_x, frame_y) = (f.xmin - x0, f.ymin - y0);
        let mut list = Vec::with_capacity(cols.len() * rows.len());
        for (cy, &(ry, ch)) in rows.iter().enumerate() {
            for (cx, &(rx, cw)) in cols.iter().enumerate() {
                let col = frame_x / 4 + cx as i64;
                let row = frame_y / 4 + cy as i64;
                let slot = usize::try_from(col + row * buf_cols)
                    .ok()
                    .filter(|&s| s < buf_cells)
                    .ok_or_else(|| invalid("cell index", 0, "cell outside the direction grid"))?;
                list.push(Cell {
                    slot,
                    fx: (rx - frame_x) as usize,
                    fy: (ry - frame_y) as usize,
                    w: cw as usize,
                    h: ch as usize,
                });
            }
        }
        cells.push(list);
    }
    Ok(Geometry {
        x0,
        y0,
        width,
        height,
        frames: geo,
        cells,
        buf_cells,
    })
}

/// Copies a cell's pixels into a 5x5 buffer (cells are at most 5x5).
fn cell_pixels(img: &IndexedImage, cell: &Cell) -> [u8; 25] {
    let mut px = [0u8; 25];
    let w = img.width as usize;
    for y in 0..cell.h.min(5) {
        for x in 0..cell.w.min(5) {
            let v = img
                .pixels
                .get((cell.fy + y) * w + cell.fx + x)
                .copied()
                .unwrap_or(0);
            if let Some(p) = px.get_mut(y * 5 + x) {
                *p = v;
            }
        }
    }
    px
}

// ---------------------------------------------------------------------------
// normalisation and colour reduction

/// Makes a frame match what the decoder can return: alpha 0/255 with index 0
/// transparent, buffers of the right length, never empty.
fn normalise(img: &IndexedImage) -> Result<IndexedImage> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return Ok(IndexedImage::new(1, 1, img.offset_x, img.offset_y));
    }
    let len = w
        .checked_mul(h)
        .ok_or_else(|| invalid("frame size", 0, "frame is too large"))?;
    if img.pixels.len() != len || img.alpha.len() != len {
        return Err(invalid(
            "frame buffers",
            0,
            format!(
                "{w}x{h} frame has {} pixels and {} alpha values",
                img.pixels.len(),
                img.alpha.len()
            ),
        ));
    }
    let mut out = img.clone();
    for (p, a) in out.pixels.iter_mut().zip(out.alpha.iter_mut()) {
        if *p == 0 || *a == 0 {
            *p = 0;
            *a = 0;
        } else {
            *a = 255;
        }
    }
    Ok(out)
}

/// Limits every cell to four distinct colours (see the module docs).
fn reduce_cells(frames: &mut [IndexedImage], geo: &Geometry, dist: &dyn Fn(u8, u8) -> u32) {
    for (img, cells) in frames.iter_mut().zip(&geo.cells) {
        let w = img.width as usize;
        for cell in cells {
            let px = cell_pixels(img, cell);
            let mut count = [0u16; 256];
            let mut distinct = 0usize;
            for y in 0..cell.h {
                for x in 0..cell.w {
                    let v = usize::from(px.get(y * 5 + x).copied().unwrap_or(0));
                    if let Some(c) = count.get_mut(v) {
                        if *c == 0 {
                            distinct += 1;
                        }
                        *c += 1;
                    }
                }
            }
            if distinct <= 4 {
                continue;
            }
            let keep_zero = count.first().copied().unwrap_or(0) > 0;
            let mut ranked: Vec<u8> = (1..=255u8)
                .filter(|&v| count.get(usize::from(v)).copied().unwrap_or(0) > 0)
                .collect();
            ranked.sort_by_key(|&v| {
                (
                    std::cmp::Reverse(count.get(usize::from(v)).copied().unwrap_or(0)),
                    v,
                )
            });
            ranked.truncate(if keep_zero { 3 } else { 4 });
            for y in 0..cell.h {
                for x in 0..cell.w {
                    let v = px.get(y * 5 + x).copied().unwrap_or(0);
                    if v == 0 || ranked.contains(&v) {
                        continue;
                    }
                    let best = ranked
                        .iter()
                        .copied()
                        .min_by_key(|&k| (dist(v, k), k))
                        .unwrap_or(v);
                    if let Some(p) = img.pixels.get_mut((cell.fy + y) * w + cell.fx + x) {
                        *p = best;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// public entry points

impl Dcc {
    /// Builds a version-6 DCC from `frames[dir][frame]` (see the module docs
    /// for what is stored and how). Cells with more than four colours are
    /// reduced by comparing palette *indices*; use [`Dcc::from_frames_with`]
    /// to supply a real colour distance.
    pub fn from_frames(frames: &[Vec<IndexedImage>]) -> Result<Dcc> {
        Dcc::from_frames_with(frames, &index_distance)
    }

    /// [`Dcc::from_frames`] with `distance(a, b)` (palette indices) deciding
    /// which kept colour a pixel moves to when its 4x4 cell holds more than
    /// four colours.
    pub fn from_frames_with(
        frames: &[Vec<IndexedImage>],
        distance: &dyn Fn(u8, u8) -> u32,
    ) -> Result<Dcc> {
        let per_dir = frames.first().map_or(0, Vec::len);
        if frames.is_empty() || per_dir == 0 {
            return Err(invalid(
                "frames",
                0,
                "a DCC needs at least one direction and one frame",
            ));
        }
        if frames.len() > MAX_DIRECTIONS {
            return Err(invalid(
                "direction count",
                2,
                format!("{} directions (limit {MAX_DIRECTIONS})", frames.len()),
            ));
        }
        if per_dir > MAX_FRAMES as usize {
            return Err(invalid(
                "frames per direction",
                3,
                format!("{per_dir} frames (limit {MAX_FRAMES})"),
            ));
        }
        if let Some((d, dir)) = frames.iter().enumerate().find(|(_, d)| d.len() != per_dir) {
            return Err(invalid(
                "frame count",
                3,
                format!(
                    "direction {d} has {} frames, direction 0 has {per_dir}",
                    dir.len()
                ),
            ));
        }
        let mut directions = Vec::with_capacity(frames.len());
        for dir in frames {
            let mut images = dir.iter().map(normalise).collect::<Result<Vec<_>>>()?;
            let geo = geometry(&images)?;
            reduce_cells(&mut images, &geo, distance);
            directions.push(DccDirection {
                frames: images,
                bbox: DccBox {
                    x: geo.x0 as i32,
                    y: geo.y0 as i32,
                    width: geo.width as u32,
                    height: geo.height as u32,
                },
            });
        }
        Ok(Dcc {
            version: 6,
            frames_per_direction: per_dir as u32,
            directions,
        })
    }

    /// Serialises the file. Fails when the directions do not all hold the
    /// same number of frames, a direction is outside the limits the decoder
    /// enforces (32 directions, 1024 frames, a 2048x2048 box), or a stream
    /// cannot be sized in 20 bits. Frames that break the four-colour cell
    /// rule are reduced first (by palette-index distance).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let dirs = Dcc::from_frames(
            &self
                .directions
                .iter()
                .map(|d| d.frames.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut blobs = Vec::with_capacity(dirs.directions.len());
        let mut total_dc6 = 24 + 4 * dirs.directions.len() * dirs.frames_per_direction as usize;
        for dir in &dirs.directions {
            let coded: Vec<u32> = dir
                .frames
                .iter()
                .map(|f| Dc6Frame::from_image(f).data.len() as u32)
                .collect();
            total_dc6 += dir
                .frames
                .iter()
                .map(|f| Dc6Frame::from_image(f).file_len())
                .sum::<usize>();
            blobs.push(encode_direction(&dir.frames, &coded)?);
        }
        let mut out = vec![0x74, 6, dirs.directions.len() as u8];
        out.extend_from_slice(&dirs.frames_per_direction.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(total_dc6 as u32).to_le_bytes());
        let mut offset = 15 + 4 * blobs.len();
        for b in &blobs {
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += b.len();
        }
        for b in &blobs {
            out.extend_from_slice(b);
        }
        Ok(out)
    }
}

fn index_distance(a: u8, b: u8) -> u32 {
    u32::from(a.abs_diff(b))
}

// ---------------------------------------------------------------------------
// header field widths

/// Smallest width-table code whose width holds `bits` bits.
fn width_code(bits: u32) -> Result<u32> {
    WIDTH_TABLE
        .iter()
        .position(|&w| w >= bits)
        .map(|c| c as u32)
        .ok_or_else(|| invalid("field width", 0, format!("{bits} bits do not fit")))
}

fn unsigned_code(max: u64) -> Result<u32> {
    width_code(64 - max.leading_zeros())
}

fn signed_code(min: i64, max: i64) -> Result<u32> {
    if min == 0 && max == 0 {
        return width_code(0);
    }
    let mut n = 1u32;
    while n < 32 {
        let (lo, hi) = (-(1i64 << (n - 1)), (1i64 << (n - 1)) - 1);
        if min >= lo && max <= hi {
            break;
        }
        n += 1;
    }
    width_code(n)
}

// ---------------------------------------------------------------------------
// cell entries

const POPCOUNT: [usize; 16] = [0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];

/// Bits per pixel the decoder uses for an entry's four colour codes.
fn bits_per_pixel(v: [u8; 4]) -> u32 {
    if v[0] == v[1] {
        0
    } else if v[1] == v[2] {
        1
    } else {
        2
    }
}

/// The pixel code that selects colour `code` in entry `v`, if it can.
fn pixel_code(v: [u8; 4], bpp: u32, code: u8) -> Option<u8> {
    match bpp {
        0 => (code == v[0]).then_some(0),
        1 => {
            if code == v[0] {
                Some(0)
            } else if code == v[1] {
                Some(1)
            } else {
                None
            }
        }
        _ => v.iter().position(|&c| c == code).map(|p| p as u8),
    }
}

/// How one entry's colour codes would be sent for a given slot mask.
struct Plan {
    /// Codes in sending order (`n` of them, then a terminator when `n` is
    /// smaller than the number of changed slots).
    seq: [u8; 4],
    n: usize,
    terminator: bool,
    /// Cost in bits of the nibble-delta and raw forms.
    delta_bits: usize,
    raw_bits: usize,
}

/// Plans the codes for the slots in `mask` to take the values in `target`,
/// or `None` when the decoder cannot be made to produce them.
///
/// The decoder reads codes in sending order and hands them to the changed
/// slots in reverse: the first code sent lands in the `n`-th changed slot
/// (counting from the lowest), the last one in the lowest, and changed slots
/// beyond `n` become 0. A code equal to its predecessor (initially 0) ends
/// the list.
fn plan(target: [u8; 4], mask: u8) -> Option<Plan> {
    let mut values = [0u8; 4];
    let mut k = 0usize;
    for (i, &t) in target.iter().enumerate() {
        if mask & (1 << i) != 0 {
            if let Some(slot) = values.get_mut(k) {
                *slot = t;
            }
            k += 1;
        }
    }
    let n = values
        .iter()
        .take(k)
        .rposition(|&v| v != 0)
        .map_or(0, |p| p + 1);
    let mut seq = [0u8; 4];
    let mut last = 0u8;
    let mut nibbles = 0usize;
    for i in 0..n {
        let q = values.get(n - 1 - i).copied().unwrap_or(0);
        if q == last {
            return None;
        }
        if let Some(s) = seq.get_mut(i) {
            *s = q;
        }
        let disp = usize::from(q.wrapping_sub(last));
        nibbles += 1 + disp / 15;
        last = q;
    }
    let terminator = n < k;
    if terminator {
        nibbles += 1;
    }
    Some(Plan {
        seq,
        n,
        terminator,
        delta_bits: nibbles * 4,
        raw_bits: 8 * (n + usize::from(terminator)),
    })
}

/// The slot values after applying `mask`/`target` to the previous entry.
fn apply(old: [u8; 4], target: [u8; 4], mask: u8) -> [u8; 4] {
    let mut out = old;
    for (i, o) in out.iter_mut().enumerate() {
        if mask & (1 << i) != 0 {
            *o = target.get(i).copied().unwrap_or(0);
        }
    }
    out
}

struct Choice {
    val: [u8; 4],
    mask: u8,
    plan: Plan,
    raw: bool,
    bpp: u32,
    cost: usize,
}

/// Picks the cheapest legal way to show `codes` (`npix` pixels) given the
/// previous entry of the cell (`old`, `None` for a cell shown for the first
/// time, whose mask is implicitly `0xF`).
fn choose(codes: &[u8], npix: usize, old: Option<[u8; 4]>, raw_on: bool) -> Option<Choice> {
    let mut set: Vec<u8> = codes.to_vec();
    set.sort_unstable();
    set.dedup();
    // canonical layout: non-zero codes descending, then zeros
    let mut canon = [0u8; 4];
    let nonzero: Vec<u8> = set.iter().rev().copied().filter(|&c| c != 0).collect();
    for (slot, &c) in canon.iter_mut().zip(&nonzero) {
        *slot = c;
    }
    let mut targets = vec![canon];
    if let Some(o) = old
        && o != canon
    {
        targets.push(o);
    }
    let mut best: Option<Choice> = None;
    for &target in &targets {
        let masks: Vec<u8> = match old {
            None => vec![0xF],
            Some(o) => {
                let minimal = (0..4).fold(0u8, |m, i| m | (u8::from(target[i] != o[i]) << i));
                if minimal == 0xF {
                    vec![0xF]
                } else {
                    vec![minimal, 0xF]
                }
            }
        };
        for mask in masks {
            let val = apply(old.unwrap_or([0; 4]), target, mask);
            let bpp = bits_per_pixel(val);
            if !codes.iter().all(|&c| pixel_code(val, bpp, c).is_some()) {
                continue;
            }
            let Some(plan) = plan(target, mask) else {
                continue;
            };
            let nb_pix = POPCOUNT.get(usize::from(mask)).copied().unwrap_or(0);
            let raw = raw_on && nb_pix > 0 && plan.raw_bits < plan.delta_bits;
            let mut cost = bpp as usize * npix;
            cost += if raw { plan.raw_bits } else { plan.delta_bits };
            if old.is_some() {
                cost += 4;
            }
            if raw_on && nb_pix > 0 {
                cost += 1;
            }
            if best.as_ref().is_none_or(|b| cost < b.cost) {
                best = Some(Choice {
                    val,
                    mask,
                    plan,
                    raw,
                    bpp,
                    cost,
                });
            }
        }
    }
    best
}

// ---------------------------------------------------------------------------
// one direction

#[derive(Clone, Copy, Debug)]
struct Mode {
    equal: bool,
    raw: bool,
}

#[derive(Default)]
struct Streams {
    equal: BitWriter,
    mask: BitWriter,
    enc_type: BitWriter,
    raw: BitWriter,
    /// Stage 1 code nibbles, then stage 2 pixel codes: one shared stream.
    codes: BitWriter,
    pixels: BitWriter,
}

#[derive(Clone, Copy)]
struct Stored {
    w: usize,
    h: usize,
    px: [u8; 25],
}

fn same_cell(a: &[u8; 25], b: &[u8; 25], w: usize, h: usize) -> bool {
    (0..h).all(|y| (0..w).all(|x| a.get(y * 5 + x) == b.get(y * 5 + x)))
}

fn write_streams(
    frames: &[IndexedImage],
    geo: &Geometry,
    code_of: &[u8; 256],
    mode: Mode,
) -> Result<Streams> {
    let mut s = Streams::default();
    let mut stored: Vec<Option<Stored>> = vec![None; geo.buf_cells];
    let mut last_val: Vec<Option<[u8; 4]>> = vec![None; geo.buf_cells];
    for (img, cells) in frames.iter().zip(&geo.cells) {
        for cell in cells {
            let content = cell_pixels(img, cell);
            let prev = stored.get(cell.slot).copied().flatten();
            if let Some(p) = prev
                && mode.equal
            {
                let unchanged = if p.w == cell.w && p.h == cell.h {
                    same_cell(&p.px, &content, cell.w, cell.h)
                } else {
                    // the decoder clears a cell whose size changed
                    same_cell(&[0u8; 25], &content, cell.w, cell.h)
                };
                s.equal.put(u64::from(unchanged), 1);
                if unchanged {
                    if let Some(slot) = stored.get_mut(cell.slot) {
                        *slot = Some(Stored {
                            w: cell.w,
                            h: cell.h,
                            px: content,
                        });
                    }
                    continue;
                }
            }
            let mut codes = Vec::with_capacity(cell.w * cell.h);
            for y in 0..cell.h {
                for x in 0..cell.w {
                    let v = content.get(y * 5 + x).copied().unwrap_or(0);
                    codes.push(code_of.get(usize::from(v)).copied().unwrap_or(0));
                }
            }
            let old = prev.and_then(|_| last_val.get(cell.slot).copied().flatten());
            let choice = choose(&codes, codes.len(), old, mode.raw)
                .ok_or_else(|| invalid("dcc encoder", 0, "no legal entry for a cell"))?;
            if prev.is_some() {
                s.mask.put(u64::from(choice.mask), 4);
            }
            let nb_pix = POPCOUNT.get(usize::from(choice.mask)).copied().unwrap_or(0);
            if mode.raw && nb_pix > 0 {
                s.enc_type.put(u64::from(choice.raw), 1);
            }
            let p = &choice.plan;
            let mut last = 0u8;
            for &q in p.seq.iter().take(p.n) {
                if choice.raw {
                    s.raw.put(u64::from(q), 8);
                } else {
                    let mut d = u32::from(q.wrapping_sub(last));
                    while d >= 15 {
                        s.codes.put(15, 4);
                        d -= 15;
                    }
                    s.codes.put(u64::from(d), 4);
                }
                last = q;
            }
            if p.terminator {
                if choice.raw {
                    s.raw.put(u64::from(last), 8);
                } else {
                    s.codes.put(0, 4);
                }
            }
            if choice.bpp > 0 {
                for &c in &codes {
                    let idx = pixel_code(choice.val, choice.bpp, c).unwrap_or(0);
                    s.pixels.put(u64::from(idx), choice.bpp);
                }
            }
            if let Some(slot) = stored.get_mut(cell.slot) {
                *slot = Some(Stored {
                    w: cell.w,
                    h: cell.h,
                    px: content,
                });
            }
            if let Some(slot) = last_val.get_mut(cell.slot) {
                *slot = Some(choice.val);
            }
        }
    }
    Ok(s)
}

fn encode_direction(frames: &[IndexedImage], coded: &[u32]) -> Result<Vec<u8>> {
    let geo = geometry(frames)?;
    // palette-usage key: code n is the n-th used palette index
    let mut used = [false; 256];
    for img in frames {
        for &p in &img.pixels {
            if let Some(u) = used.get_mut(usize::from(p)) {
                *u = true;
            }
        }
    }
    let mut code_of = [0u8; 256];
    let mut next = 0usize;
    for (v, &u) in used.iter().enumerate() {
        if u {
            if let Some(c) = code_of.get_mut(v) {
                *c = next as u8;
            }
            next += 1;
        }
    }

    // header field widths
    let ymax = |f: &FrameGeo| f.ymin + f.height - 1;
    let range = |get: &dyn Fn(&FrameGeo) -> i64| {
        let vals = geo.frames.iter().map(get);
        let lo = vals.clone().min().unwrap_or(0);
        let hi = vals.max().unwrap_or(0);
        (lo, hi)
    };
    let (w_lo, w_hi) = range(&|f| f.width);
    let (h_lo, h_hi) = range(&|f| f.height);
    let (x_lo, x_hi) = range(&|f| f.xmin);
    let (y_lo, y_hi) = range(&ymax);
    let _ = (w_lo, h_lo);
    let fits = |v: i64| i32::try_from(v).is_ok();
    if !(fits(x_lo) && fits(x_hi) && fits(y_lo) && fits(y_hi)) {
        return Err(invalid("frame offset", 0, "offset does not fit 32 bits"));
    }
    let coded_max = coded.iter().copied().max().unwrap_or(0);
    let codes = [
        0,
        unsigned_code(w_hi as u64)?,
        unsigned_code(h_hi as u64)?,
        signed_code(x_lo, x_hi)?,
        signed_code(y_lo, y_hi)?,
        0,
        unsigned_code(u64::from(coded_max))?,
    ];
    let width_of = |c: u32| WIDTH_TABLE.get(c as usize).copied().unwrap_or(0);
    // the direction's DC6 size: 32-byte frame headers, data, 3 slack bytes
    let outsize: u64 = coded.iter().map(|&c| u64::from(c) + 35).sum();

    let mut best: Option<Vec<u8>> = None;
    for mode in [
        Mode {
            equal: true,
            raw: false,
        },
        Mode {
            equal: true,
            raw: true,
        },
        Mode {
            equal: false,
            raw: false,
        },
        Mode {
            equal: false,
            raw: true,
        },
    ] {
        let s = write_streams(frames, &geo, &code_of, mode)?;
        let sizes = [s.equal.len, s.mask.len, s.enc_type.len, s.raw.len];
        if sizes.iter().any(|&n| n > MAX_STREAM_BITS) {
            continue;
        }
        let mut dir = BitWriter::default();
        dir.put(outsize, 32);
        dir.put(u64::from(mode.equal) << 1 | u64::from(mode.raw), 2);
        for c in codes {
            dir.put(u64::from(c), 4);
        }
        for (f, &cb) in geo.frames.iter().zip(coded) {
            dir.put(0, width_of(codes[0]));
            dir.put(f.width as u64, width_of(codes[1]));
            dir.put(f.height as u64, width_of(codes[2]));
            dir.put(f.xmin as u64, width_of(codes[3]));
            dir.put(ymax(f) as u64, width_of(codes[4]));
            dir.put(0, width_of(codes[5]));
            dir.put(u64::from(cb), width_of(codes[6]));
            dir.put(0, 1); // bottom_up
        }
        if mode.equal {
            dir.put(s.equal.len as u64, 20);
        }
        dir.put(s.mask.len as u64, 20);
        if mode.raw {
            dir.put(s.enc_type.len as u64, 20);
            dir.put(s.raw.len as u64, 20);
        }
        for &u in &used {
            dir.put(u64::from(u), 1);
        }
        if mode.equal {
            dir.append(&s.equal);
        }
        dir.append(&s.mask);
        if mode.raw {
            dir.append(&s.enc_type);
            dir.append(&s.raw);
        }
        dir.append(&s.codes);
        dir.append(&s.pixels);
        if best.as_ref().is_none_or(|b| dir.bytes.len() < b.len()) {
            best = Some(dir.bytes);
        }
    }
    best.ok_or_else(|| FormatError::NotRepresentable {
        format: FORMAT,
        version: 6,
        detail: "a stream is larger than its 20-bit size field allows".to_owned(),
    })
}
