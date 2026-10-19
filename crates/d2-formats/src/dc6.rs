//! DC6 sprites (`*.dc6`): palette-indexed frames with a per-scanline RLE.
//!
//! ```text
//! 0x00 i32 version (6)   0x04 u32 flags (1)   0x08 u32 encoding (0)
//! 0x0C u32 termination (unused)  0x10 u32 directions  0x14 u32 frames per direction
//! 0x18 u32 x directions*frames  absolute frame offsets, direction-major
//! frame header, 32 bytes: i32 flip, i32 width, i32 height, i32 offset x,
//!   i32 offset y, u32 unknown, u32 next block (unused), u32 length,
//!   then `length` RLE bytes (+3 slack bytes that are never read)
//! ```
//!
//! RLE opcodes: `0x80` ends the scanline, `0x81..=0xFF` skips `op & 0x7F`
//! transparent pixels, `0x00..=0x7F` copies that many literal pixels. Rows
//! are stored bottom-up unless the frame's `flip` flag is set, in which case
//! they run top-down (the inventory backdrops `inv*.dc6`; no other shipped
//! file sets it). The reference editor never reads the flag.
//!
//! `offset_x` is the frame's left column and `offset_y` its **bottom** row
//! (inclusive), both relative to the sprite anchor, y pointing down. The
//! rendered image's own `offset_y` is therefore `offset_y - height + 1`.
//! Palette index 0 is the transparent key (alpha 0).
//!
//! Loaders are lenient (version is the only header value checked) but every
//! range is bounds-checked, and the frame data copied out of a file is capped
//! at the file length plus 1 MiB.

use crate::bytes::{i32_at, slice_at, u32_at};
use crate::{FormatError, IndexedImage, Result};

const FORMAT: &str = "dc6";
const HEADER_LEN: usize = 24;
const FRAME_HEADER_LEN: usize = 32;
/// A frame may be at most this many pixels per side when rendered.
const MAX_FRAME_DIM: u32 = 4096;
const MAX_FRAME_PIXELS: u64 = 4096 * 4096;
const DATA_SLACK: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dc6 {
    pub version: i32,
    pub flags: u32,
    pub encoding: u32,
    pub directions: u32,
    pub frames_per_direction: u32,
    /// `frames[dir * frames_per_direction + frame]`.
    pub frames: Vec<Dc6Frame>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dc6Frame {
    /// The frame's `flip` dword was non-zero: rows are stored top-down.
    pub flipped: bool,
    pub width: u32,
    pub height: u32,
    /// Left column relative to the anchor.
    pub offset_x: i32,
    /// Bottom row (inclusive) relative to the anchor, y down.
    pub offset_y: i32,
    /// The RLE stream (`length` bytes, without the 3 slack bytes).
    pub data: Vec<u8>,
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

impl Dc6 {
    pub fn parse(bytes: &[u8]) -> Result<Dc6> {
        if bytes.len() < HEADER_LEN {
            return Err(truncated(bytes.len(), HEADER_LEN - bytes.len()));
        }
        let field = |o: usize| u32_at(bytes, o).ok_or_else(|| truncated(o, 4));
        let version = field(0)? as i32;
        if version != 6 {
            return Err(FormatError::UnsupportedVersion {
                format: FORMAT,
                version: i64::from(version),
            });
        }
        let (flags, encoding) = (field(4)?, field(8)?);
        let (directions, frames_per_direction) = (field(16)?, field(20)?);
        let total = u64::from(directions) * u64::from(frames_per_direction);
        let table_bytes = total.saturating_mul(4);
        if table_bytes > (bytes.len() - HEADER_LEN) as u64 {
            return Err(invalid(
                "frame table",
                16,
                format!("{directions} directions x {frames_per_direction} frames do not fit"),
            ));
        }
        let total = total as usize;
        let budget = bytes.len().saturating_add(DATA_SLACK);
        let mut copied = 0usize;
        let mut frames = Vec::with_capacity(total);
        for i in 0..total {
            let table_at = HEADER_LEN + 4 * i;
            let off = field(table_at)? as usize;
            let hdr = slice_at(bytes, off, FRAME_HEADER_LEN)
                .ok_or_else(|| truncated(off.min(bytes.len()), FRAME_HEADER_LEN))?;
            let i32f = |o: usize| i32_at(hdr, o).unwrap_or(0);
            let dim = |o: usize, what: &'static str| {
                u32::try_from(i32f(o))
                    .map_err(|_| invalid(what, off + o, format!("negative value {}", i32f(o))))
            };
            let length = u32_at(hdr, 28).unwrap_or(0) as usize;
            let data = slice_at(bytes, off + FRAME_HEADER_LEN, length).ok_or_else(|| {
                invalid(
                    "frame data",
                    off + 28,
                    format!("{length} bytes run past the end of the file"),
                )
            })?;
            copied = copied.saturating_add(length);
            if copied > budget {
                return Err(invalid(
                    "frame data",
                    off + 28,
                    "frame data exceeds the file length by more than the allowed slack",
                ));
            }
            frames.push(Dc6Frame {
                flipped: i32f(0) != 0,
                width: dim(4, "frame width")?,
                height: dim(8, "frame height")?,
                offset_x: i32f(12),
                offset_y: i32f(16),
                data: data.to_vec(),
            });
        }
        Ok(Dc6 {
            version,
            flags,
            encoding,
            directions,
            frames_per_direction,
            frames,
        })
    }

    /// The frame at `(dir, frame)`, `None` when either is out of range.
    pub fn frame(&self, dir: u32, frame: u32) -> Option<&Dc6Frame> {
        if dir >= self.directions || frame >= self.frames_per_direction {
            return None;
        }
        let index = u64::from(dir) * u64::from(self.frames_per_direction) + u64::from(frame);
        self.frames.get(usize::try_from(index).ok()?)
    }
}

impl Dc6Frame {
    /// Decodes the frame to a `width x height` image whose `offset_x` /
    /// `offset_y` place pixel (0,0) relative to the sprite anchor. Stream
    /// pixels that fall outside the frame are dropped; a stream that ends
    /// early leaves the rest transparent. Fails only for frames larger than
    /// 4096 pixels on a side or 16 Mi pixels.
    pub fn render(&self) -> Result<IndexedImage> {
        if self.width > MAX_FRAME_DIM
            || self.height > MAX_FRAME_DIM
            || u64::from(self.width) * u64::from(self.height) > MAX_FRAME_PIXELS
        {
            return Err(invalid(
                "frame size",
                0,
                format!("{}x{} exceeds the decoder limit", self.width, self.height),
            ));
        }
        let top = i64::from(self.offset_y) - i64::from(self.height) + 1;
        let top = i32::try_from(top).map_err(|_| invalid("frame offset", 0, "out of range"))?;
        let mut image = IndexedImage::new(self.width, self.height, self.offset_x, top);
        let (w, h) = (self.width as usize, self.height as usize);
        if w == 0 || h == 0 {
            return Ok(image);
        }
        let mut x = 0usize;
        // current row, as a signed value so that running off either end of
        // the frame is just "clipped"
        let (mut row, step): (i64, i64) = if self.flipped {
            (0, 1)
        } else {
            (h as i64 - 1, -1)
        };
        let data = self.data.as_slice();
        let mut pos = 0usize;
        while let Some(&op) = data.get(pos) {
            pos += 1;
            if op == 0x80 {
                x = 0;
                row += step;
            } else if op & 0x80 != 0 {
                x += usize::from(op & 0x7F);
            } else {
                let end = (pos + usize::from(op)).min(data.len());
                let literals = data.get(pos..end).unwrap_or(&[]);
                pos = end;
                if (0..h as i64).contains(&row) {
                    let base = row as usize * w;
                    for &p in literals {
                        if x < w {
                            let i = base + x;
                            if let (Some(px), Some(al)) =
                                (image.pixels.get_mut(i), image.alpha.get_mut(i))
                            {
                                *px = p;
                                *al = if p == 0 { 0 } else { 255 };
                            }
                        }
                        x += 1;
                    }
                } else {
                    x += literals.len();
                }
            }
        }
        Ok(image)
    }
}

// ---------------------------------------------------------------------------
// Encoder

/// Longest transparent or literal run one control byte can carry.
const MAX_RUN: usize = 127;
/// Bytes after each frame's RLE data (never read; the game writes `EE EE EE`).
const FRAME_TAIL: [u8; 3] = [0xEE; 3];

impl Dc6Frame {
    /// RLE-encodes an indexed image. A pixel is transparent when its alpha is
    /// 0 or its palette index is 0 (the DC6 transparent key). Transparent
    /// runs and literal runs are each capped at 127 pixels per control byte
    /// (the game's decoder crashes on longer skips, so those are split);
    /// `0x80` ends every scanline and trailing transparent pixels of a line
    /// are left out. Rows are stored bottom-up (`flipped == false`).
    ///
    /// `offset_x` / `offset_y` are the exact inverse of
    /// [`Dc6Frame::render`]: the stored bottom row is
    /// `img.offset_y + height - 1`.
    pub fn from_image(img: &IndexedImage) -> Dc6Frame {
        let (w, h) = (img.width as usize, img.height as usize);
        let opaque = |x: usize, y: usize| -> Option<u8> {
            let i = y * w + x;
            let p = img.pixels.get(i).copied().unwrap_or(0);
            let a = img.alpha.get(i).copied().unwrap_or(0);
            (p != 0 && a != 0).then_some(p)
        };
        let mut data = Vec::new();
        for y in (0..h).rev() {
            // end of the last opaque pixel: later transparent pixels need no opcode
            let end = (0..w)
                .rev()
                .find(|&x| opaque(x, y).is_some())
                .map_or(0, |x| x + 1);
            let mut x = 0usize;
            while x < end {
                if opaque(x, y).is_none() {
                    let start = x;
                    while x < end && opaque(x, y).is_none() {
                        x += 1;
                    }
                    let mut left = x - start;
                    while left > 0 {
                        let step = left.min(MAX_RUN);
                        data.push(0x80 | step as u8);
                        left -= step;
                    }
                } else {
                    let start = x;
                    while x < end && x - start < MAX_RUN && opaque(x, y).is_some() {
                        x += 1;
                    }
                    data.push((x - start) as u8);
                    data.extend((start..x).map(|px| opaque(px, y).unwrap_or(0)));
                }
            }
            data.push(0x80);
        }
        let bottom = i64::from(img.offset_y) + h as i64 - 1;
        Dc6Frame {
            flipped: false,
            width: img.width,
            height: img.height,
            offset_x: img.offset_x,
            offset_y: bottom.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            data,
        }
    }

    /// Bytes this frame occupies in a file: header, RLE data and slack.
    pub(crate) fn file_len(&self) -> usize {
        FRAME_HEADER_LEN + self.data.len() + FRAME_TAIL.len()
    }
}

impl Dc6 {
    /// Builds a version-6 DC6 (flags 1, encoding 0) from `frames[dir][frame]`.
    /// Every direction must hold the same number of frames.
    pub fn from_frames(frames: &[Vec<IndexedImage>]) -> Result<Dc6> {
        let per_dir = frames.first().map_or(0, Vec::len);
        if let Some((d, dir)) = frames.iter().enumerate().find(|(_, d)| d.len() != per_dir) {
            return Err(invalid(
                "frame count",
                0,
                format!(
                    "direction {d} has {} frames, direction 0 has {per_dir}",
                    dir.len()
                ),
            ));
        }
        Ok(Dc6 {
            version: 6,
            flags: 1,
            encoding: 0,
            directions: u32::try_from(frames.len())
                .map_err(|_| invalid("direction count", 0, "too many directions"))?,
            frames_per_direction: u32::try_from(per_dir)
                .map_err(|_| invalid("frames per direction", 0, "too many frames"))?,
            frames: frames.iter().flatten().map(Dc6Frame::from_image).collect(),
        })
    }

    /// Serialises the file: 24-byte header (termination `EE EE EE EE`), the
    /// frame offset table, then per frame a 32-byte header, the RLE data and
    /// three `EE` slack bytes. Fails when the frame count does not match
    /// `directions * frames_per_direction` or the file exceeds 4 GiB.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let expected = u64::from(self.directions) * u64::from(self.frames_per_direction);
        if expected != self.frames.len() as u64 {
            return Err(invalid(
                "frame count",
                16,
                format!(
                    "{} directions x {} frames, but {} frames present",
                    self.directions,
                    self.frames_per_direction,
                    self.frames.len()
                ),
            ));
        }
        let table_end = HEADER_LEN + 4 * self.frames.len();
        let total = table_end + self.frames.iter().map(Dc6Frame::file_len).sum::<usize>();
        if u32::try_from(total).is_err() {
            return Err(invalid("file size", 0, "file is larger than 4 GiB"));
        }
        let mut out = Vec::with_capacity(total);
        for v in [
            self.version as u32,
            self.flags,
            self.encoding,
            0xEEEE_EEEE,
            self.directions,
            self.frames_per_direction,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let mut at = table_end;
        for f in &self.frames {
            out.extend_from_slice(&(at as u32).to_le_bytes());
            at += f.file_len();
        }
        let mut at = table_end;
        for f in &self.frames {
            at += f.file_len();
            // offset of the following frame record (the game writes the file
            // length after the last one); the decoders never read it
            let fields = [
                i32::from(f.flipped),
                f.width as i32,
                f.height as i32,
                f.offset_x,
                f.offset_y,
                0,
                at as i32,
                f.data.len() as i32,
            ];
            for v in fields {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&f.data);
            out.extend_from_slice(&FRAME_TAIL);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod encode_tests;
#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
