//! The in-memory sprite: palette-indexed frames plus the RGBA frame type
//! used for image import/export.

use d2_formats::IndexedImage;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::palette::Palette;

use crate::error::{Result, SpriteError};

/// Largest frame side [`Sprite::encode`] accepts (even an empty frame: the
/// encoders walk its rows).
const MAX_FRAME_SIDE: u32 = 1 << 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SpriteFormat {
    Dc6,
    Dcc,
}

impl SpriteFormat {
    /// File extension without the dot, lower case.
    pub fn extension(self) -> &'static str {
        match self {
            SpriteFormat::Dc6 => "dc6",
            SpriteFormat::Dcc => "dcc",
        }
    }

    /// The format for a file extension (any case), if it is one.
    pub fn from_extension(ext: &str) -> Option<SpriteFormat> {
        if ext.eq_ignore_ascii_case("dc6") {
            Some(SpriteFormat::Dc6)
        } else if ext.eq_ignore_ascii_case("dcc") {
            Some(SpriteFormat::Dcc)
        } else {
            None
        }
    }
}

/// Palette-indexed frames, `directions[dir][frame]`; every direction holds
/// the same number of frames. Palette index 0 / alpha 0 is transparent.
/// Frame offsets locate pixel (0,0) of each frame relative to the anchor
/// (y down), exactly like [`IndexedImage`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Sprite {
    pub directions: Vec<Vec<IndexedImage>>,
}

impl Sprite {
    /// Decodes a DC6 or DCC file.
    pub fn decode(bytes: &[u8], format: SpriteFormat) -> Result<Sprite> {
        let directions = match format {
            SpriteFormat::Dc6 => {
                let dc6 = Dc6::parse(bytes)?;
                let mut dirs = Vec::new();
                for d in 0..dc6.directions {
                    let mut frames = Vec::new();
                    for f in 0..dc6.frames_per_direction {
                        let frame = dc6.frame(d, f).ok_or_else(|| {
                            SpriteError::invalid(format!("missing frame {d}/{f}"))
                        })?;
                        frames.push(frame.render()?);
                    }
                    dirs.push(frames);
                }
                dirs
            }
            SpriteFormat::Dcc => Dcc::parse(bytes)?
                .directions
                .into_iter()
                .map(|d| d.frames)
                .collect(),
        };
        Ok(Sprite { directions })
    }

    /// Encodes to a DC6 or DCC file. DCC cells with more than four colours
    /// are reduced by palette-index distance; prefer
    /// [`Sprite::encode_with_palette`] when the palette is known.
    pub fn encode(&self, format: SpriteFormat) -> Result<Vec<u8>> {
        self.check_shape()?;
        match format {
            SpriteFormat::Dc6 => Ok(Dc6::from_frames(&self.directions)?.to_bytes()?),
            SpriteFormat::Dcc => Ok(Dcc::from_frames(&self.directions)?.to_bytes()?),
        }
    }

    /// [`Sprite::encode`] that reduces crowded DCC cells (more than four
    /// colours in a 4x4 cell, which the format cannot hold) to the nearest
    /// kept colour in RGB under `palette`. Identical to `encode` for DC6 and
    /// for DCC data that already fits.
    pub fn encode_with_palette(&self, format: SpriteFormat, palette: &Palette) -> Result<Vec<u8>> {
        if format == SpriteFormat::Dc6 {
            return self.encode(format);
        }
        self.check_shape()?;
        let distance = |a: u8, b: u8| {
            let (pa, pb) = (
                palette.colors[usize::from(a)],
                palette.colors[usize::from(b)],
            );
            pa.iter()
                .zip(&pb)
                .map(|(&x, &y)| u32::from(x.abs_diff(y)).pow(2))
                .sum()
        };
        let dcc = Dcc::from_frames_with(&self.directions, &distance)?;
        Ok(dcc.to_bytes()?)
    }

    fn check_shape(&self) -> Result<()> {
        if let Some((d, f)) = self.directions.iter().enumerate().find_map(|(d, frames)| {
            frames
                .iter()
                .position(|img| {
                    !crate::ops::backed(img)
                        || img.width > MAX_FRAME_SIDE
                        || img.height > MAX_FRAME_SIDE
                })
                .map(|f| (d, f))
        }) {
            return Err(SpriteError::invalid(format!(
                "frame {d}/{f} is larger than {MAX_FRAME_SIDE} pixels on a side or claims more pixels than its buffers hold"
            )));
        }
        let per = self.frames_per_direction();
        if let Some(d) = self.directions.iter().position(|d| d.len() != per) {
            return Err(SpriteError::invalid(format!(
                "direction {d} has {} frames, direction 0 has {per}",
                self.directions.get(d).map_or(0, Vec::len)
            )));
        }
        Ok(())
    }

    /// Frames in each direction (those of direction 0; 0 for no directions).
    pub fn frames_per_direction(&self) -> usize {
        self.directions.first().map_or(0, Vec::len)
    }

    /// Union box of all non-empty frames in anchor space:
    /// `(min_x, min_y, max_x, max_y)` with the max edges **exclusive**
    /// (`offset + size`), so the box is `max - min` wide. `None` when there
    /// is no frame with pixels.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let mut out: Option<(i32, i32, i32, i32)> = None;
        for img in self.directions.iter().flatten() {
            if img.width == 0 || img.height == 0 {
                continue;
            }
            let x1 = img.offset_x.saturating_add_unsigned(img.width);
            let y1 = img.offset_y.saturating_add_unsigned(img.height);
            out = Some(match out {
                None => (img.offset_x, img.offset_y, x1, y1),
                Some((a, b, c, d)) => (
                    a.min(img.offset_x),
                    b.min(img.offset_y),
                    c.max(x1),
                    d.max(y1),
                ),
            });
        }
        out
    }
}

/// A straight-alpha RGBA8 frame with its anchor offset (position of pixel
/// (0,0) relative to the anchor, y down). `pixels.len() == width * height * 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub pixels: Vec<u8>,
}

impl RgbaFrame {
    /// A fully transparent frame.
    pub fn new(width: u32, height: u32, offset_x: i32, offset_y: i32) -> RgbaFrame {
        let n = (width as usize).saturating_mul(height as usize);
        RgbaFrame {
            width,
            height,
            offset_x,
            offset_y,
            pixels: vec![0; n.saturating_mul(4)],
        }
    }

    /// `[r, g, b, a]` at `(x, y)`, `None` outside the frame.
    pub fn get(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let i = self.byte_index(x, y)?;
        let p = self.pixels.get(i..i.checked_add(4)?)?;
        Some([p[0], p[1], p[2], p[3]])
    }

    /// Sets `[r, g, b, a]` at `(x, y)`; a point outside the frame is ignored.
    pub fn put(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        let Some(i) = self.byte_index(x, y) else {
            return;
        };
        if let Some(p) = i.checked_add(4).and_then(|end| self.pixels.get_mut(i..end)) {
            p.copy_from_slice(&rgba);
        }
    }

    /// Byte offset of pixel `(x, y)`; `None` outside the frame or when the
    /// declared size overflows (a frame whose fields disagree with its
    /// buffer).
    fn byte_index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        (y as usize)
            .checked_mul(self.width as usize)?
            .checked_add(x as usize)?
            .checked_mul(4)
    }
}
