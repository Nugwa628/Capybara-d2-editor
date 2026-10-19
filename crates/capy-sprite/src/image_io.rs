//! Image file input and output: PNG, BMP and GIF through the `image` crate
//! (GIF: first frame), PCX through a small reader of our own.
//!
//! PCX support: RLE or raw data; 8 bits per pixel with 1 plane (palette at
//! the end of the file, grey ramp when missing), 3 planes (RGB) or 4 planes
//! (RGBA); 1, 2 and 4 bits per pixel with 1 to 4 planes (the header's
//! 16-colour palette; one 1-bit plane is black and white).

use std::io::Cursor;

use image::{ExtendedColorType, ImageEncoder, ImageFormat};

use crate::error::{Result, SpriteError};
use crate::model::RgbaFrame;

/// Largest image side or pixel count accepted from a file.
const MAX_SIDE: usize = 1 << 14;
const MAX_PIXELS: usize = 1 << 26;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Bmp,
    Gif,
    Pcx,
}

impl ImageKind {
    /// The kind for a file extension (any case): png, bmp, gif or pcx.
    pub fn from_extension(ext: &str) -> Option<ImageKind> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(ImageKind::Png),
            "bmp" => Some(ImageKind::Bmp),
            "gif" => Some(ImageKind::Gif),
            "pcx" => Some(ImageKind::Pcx),
            _ => None,
        }
    }
}

/// Decodes an image file to RGBA with offsets 0.
pub fn import_image(bytes: &[u8], kind: ImageKind) -> Result<RgbaFrame> {
    let format = match kind {
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Bmp => ImageFormat::Bmp,
        ImageKind::Gif => ImageFormat::Gif,
        ImageKind::Pcx => return read_pcx(bytes),
    };
    let img = image::load_from_memory_with_format(bytes, format)
        .map_err(|e| SpriteError::Image(e.to_string()))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    if width as usize > MAX_SIDE || height as usize > MAX_SIDE {
        return Err(SpriteError::Image(format!(
            "{width}x{height} is larger than {MAX_SIDE} pixels on a side"
        )));
    }
    Ok(RgbaFrame {
        width,
        height,
        offset_x: 0,
        offset_y: 0,
        pixels: img.into_raw(),
    })
}

/// Encodes the frame as an RGBA PNG (offsets are not stored in the file).
pub fn export_png(frame: &RgbaFrame) -> Result<Vec<u8>> {
    if frame.width == 0 || frame.height == 0 {
        return Err(SpriteError::Image("a PNG cannot be empty".to_owned()));
    }
    let want = (frame.width as usize)
        .saturating_mul(frame.height as usize)
        .saturating_mul(4);
    if frame.pixels.len() != want {
        return Err(SpriteError::invalid(format!(
            "{}x{} frame has {} bytes, expected {want}",
            frame.width,
            frame.height,
            frame.pixels.len()
        )));
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(Cursor::new(&mut out))
        .write_image(
            &frame.pixels,
            frame.width,
            frame.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| SpriteError::Image(e.to_string()))?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// PCX

fn pcx_err(msg: impl Into<String>) -> SpriteError {
    SpriteError::Image(format!("PCX: {}", msg.into()))
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn read_pcx(bytes: &[u8]) -> Result<RgbaFrame> {
    let header = bytes
        .get(..128)
        .ok_or_else(|| pcx_err("file is shorter than the 128-byte header"))?;
    if header[0] != 0x0A {
        return Err(pcx_err("bad manufacturer byte"));
    }
    let rle = match header[2] {
        0 => false,
        1 => true,
        e => return Err(pcx_err(format!("unknown encoding {e}"))),
    };
    let bpp = usize::from(header[3]);
    let field = |at| u16_le(header, at).map(usize::from).unwrap_or(0);
    let (xmin, ymin, xmax, ymax) = (field(4), field(6), field(8), field(10));
    if xmax < xmin || ymax < ymin {
        return Err(pcx_err("empty or inverted window"));
    }
    let (width, height) = (xmax - xmin + 1, ymax - ymin + 1);
    if width > MAX_SIDE || height > MAX_SIDE || width * height > MAX_PIXELS {
        return Err(pcx_err(format!("{width}x{height} is too large")));
    }
    let planes = usize::from(header[65]);
    let per_line = field(66);
    if planes == 0 || planes > 4 {
        return Err(pcx_err(format!("{planes} colour planes")));
    }
    if !matches!(bpp, 1 | 2 | 4 | 8) {
        return Err(pcx_err(format!("{bpp} bits per pixel")));
    }
    if per_line * 8 < width * bpp {
        return Err(pcx_err("bytes per line too small for the width"));
    }
    if bpp == 8 && planes == 2 {
        return Err(pcx_err("8-bit data with 2 planes"));
    }

    // scan lines
    let row_len = planes * per_line;
    let total = row_len * height;
    let mut data = Vec::with_capacity(total);
    let mut pos = 128usize;
    if rle {
        while data.len() < total {
            let b = *bytes
                .get(pos)
                .ok_or_else(|| pcx_err("truncated pixel data"))?;
            pos += 1;
            if b >= 0xC0 {
                let run = usize::from(b & 0x3F);
                let v = *bytes
                    .get(pos)
                    .ok_or_else(|| pcx_err("truncated pixel data"))?;
                pos += 1;
                let room = total - data.len();
                data.extend(std::iter::repeat_n(v, run.min(room)));
            } else {
                data.push(b);
            }
        }
    } else {
        data.extend_from_slice(
            bytes
                .get(pos..pos + total)
                .ok_or_else(|| pcx_err("truncated pixel data"))?,
        );
        pos += total;
    }

    let mut out = RgbaFrame::new(width as u32, height as u32, 0, 0);
    let row = |y: usize| data.get(y * row_len..(y + 1) * row_len).unwrap_or(&[]);
    if bpp == 8 && planes == 1 {
        // 256-colour palette after the data: 0x0C marker + 768 bytes
        let tail = bytes.len().checked_sub(769);
        let palette = tail
            .filter(|&t| t >= pos.min(bytes.len()) && bytes.get(t) == Some(&0x0C))
            .and_then(|t| bytes.get(t + 1..t + 769));
        for y in 0..height {
            for x in 0..width {
                let i = usize::from(row(y).get(x).copied().unwrap_or(0));
                let rgb = match palette {
                    Some(p) => [p[i * 3], p[i * 3 + 1], p[i * 3 + 2]],
                    None => [i as u8; 3],
                };
                out.put(x as u32, y as u32, [rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    } else if bpp == 8 {
        for y in 0..height {
            let r = row(y);
            for x in 0..width {
                let ch = |p: usize| r.get(p * per_line + x).copied().unwrap_or(0);
                let a = if planes == 4 { ch(3) } else { 255 };
                out.put(x as u32, y as u32, [ch(0), ch(1), ch(2), a]);
            }
        }
    } else {
        // 1, 2 or 4 bits per pixel, bit planes combined into a palette index
        let palette: Vec<[u8; 3]> = if bpp == 1 && planes == 1 {
            vec![[0, 0, 0], [255, 255, 255]]
        } else {
            header[16..64].as_chunks::<3>().0.to_vec()
        };
        for y in 0..height {
            let r = row(y);
            for x in 0..width {
                let mut index = 0usize;
                for p in 0..planes {
                    let byte = r.get(p * per_line + x * bpp / 8).copied().unwrap_or(0);
                    let shift = 8 - bpp - (x * bpp) % 8;
                    let v = usize::from(byte >> shift) & ((1 << bpp) - 1);
                    index |= v << (p * bpp);
                }
                let rgb = palette.get(index).copied().unwrap_or([0, 0, 0]);
                out.put(x as u32, y as u32, [rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }
    Ok(out)
}
