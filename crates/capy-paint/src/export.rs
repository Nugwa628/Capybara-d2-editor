//! Palette files, indexed PNG, animated GIF and the onion-skin preview.

use crate::error::{PaintError, Result};
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;
use std::borrow::Cow;

fn rgb_bytes(palette: &Palette) -> Vec<u8> {
    palette.colors.iter().flatten().copied().collect()
}

/// GIMP palette (`.gpl`), 16 columns, one named line per index.
pub fn palette_gpl(palette: &Palette, name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out = format!("GIMP Palette\nName: {name}\nColumns: 16\n#\n");
    for (i, [r, g, b]) in palette.colors.iter().enumerate() {
        out.push_str(&format!("{r:3} {g:3} {b:3}\tIndex {i}\n"));
    }
    out
}

/// Adobe colour table (`.act`): 256 RGB triplets, 768 bytes, no trailer.
pub fn palette_act(palette: &Palette) -> Vec<u8> {
    rgb_bytes(palette)
}

/// JASC-PAL text palette (`\r\n` line endings, as Paint Shop Pro writes it).
pub fn palette_jasc(palette: &Palette) -> String {
    let mut out = String::from("JASC-PAL\r\n0100\r\n256\r\n");
    for [r, g, b] in &palette.colors {
        out.push_str(&format!("{r} {g} {b}\r\n"));
    }
    out
}

/// Largest canvas (pixels) the GIF and onion exporters will allocate.
const MAX_CANVAS: u64 = 64 << 20;

fn encode_err(e: impl std::fmt::Display) -> PaintError {
    PaintError::Encode(e.to_string())
}

/// PNG with a PLTE chunk of the palette and a tRNS chunk making index 0
/// transparent, so edits in GIMP or Photoshop keep exact indices. Transparent
/// pixels are written as index 0. Offsets are not stored.
pub fn indexed_png(img: &IndexedImage, palette: &Palette) -> Result<Vec<u8>> {
    let len = (img.width as usize).saturating_mul(img.height as usize);
    if img.width == 0 || img.height == 0 {
        return Err(PaintError::Encode("image has no pixels".into()));
    }
    if img.pixels.len() != len || img.alpha.len() != len {
        return Err(PaintError::Invalid(
            "pixel buffers do not match the image size".into(),
        ));
    }
    let data: Vec<u8> = img
        .pixels
        .iter()
        .zip(&img.alpha)
        .map(|(&p, &a)| if a == 0 { 0 } else { p })
        .collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(Cow::Owned(rgb_bytes(palette)));
        enc.set_trns(Cow::Owned(vec![0u8]));
        let mut writer = enc.write_header().map_err(encode_err)?;
        writer.write_image_data(&data).map_err(encode_err)?;
        writer.finish().map_err(encode_err)?;
    }
    Ok(out)
}

/// The box (min x, min y, width, height) covering every image, in anchor space.
fn union_box<'a>(images: impl Iterator<Item = &'a IndexedImage>) -> Option<(i32, i32, u32, u32)> {
    let mut b: Option<(i64, i64, i64, i64)> = None;
    for img in images {
        if img.width == 0 || img.height == 0 {
            continue;
        }
        let (x0, y0) = (i64::from(img.offset_x), i64::from(img.offset_y));
        let (x1, y1) = (x0 + i64::from(img.width), y0 + i64::from(img.height));
        b = Some(match b {
            None => (x0, y0, x1, y1),
            Some((a, bb, c, d)) => (a.min(x0), bb.min(y0), c.max(x1), d.max(y1)),
        });
    }
    let (x0, y0, x1, y1) = b?;
    Some((x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// Animated GIF of one direction, `delay_cs` centiseconds per frame, looping
/// forever. The palette is the global colour table with index 0 as the
/// transparent index; every frame is drawn on one canvas (the union of the
/// frames' boxes) at its offset, and each frame replaces the previous one.
pub fn animated_gif(frames: &[IndexedImage], palette: &Palette, delay_cs: u16) -> Result<Vec<u8>> {
    let Some((bx, by, w, h)) = union_box(frames.iter()) else {
        return Err(PaintError::Encode("no frames to encode".into()));
    };
    let (Ok(cw), Ok(ch)) = (u16::try_from(w), u16::try_from(h)) else {
        return Err(PaintError::Encode(format!(
            "canvas {w}x{h} exceeds the GIF limit of 65535"
        )));
    };
    if u64::from(w) * u64::from(h) > MAX_CANVAS {
        return Err(PaintError::Encode(format!("canvas {w}x{h} is too large")));
    }
    let mut out = Vec::new();
    {
        let mut enc =
            gif::Encoder::new(&mut out, cw, ch, &rgb_bytes(palette)).map_err(encode_err)?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(encode_err)?;
        for img in frames {
            let mut canvas = vec![0u8; w as usize * h as usize];
            for y in 0..img.height as usize {
                for x in 0..img.width as usize {
                    let i = y * img.width as usize + x;
                    if img.alpha.get(i).copied().unwrap_or(0) == 0 {
                        continue;
                    }
                    let cx = (i64::from(img.offset_x) - i64::from(bx)) as usize + x;
                    let cy = (i64::from(img.offset_y) - i64::from(by)) as usize + y;
                    canvas[cy * w as usize + cx] = img.pixels.get(i).copied().unwrap_or(0);
                }
            }
            let frame = gif::Frame {
                width: cw,
                height: ch,
                delay: delay_cs,
                dispose: gif::DisposalMethod::Background,
                transparent: Some(0),
                buffer: Cow::Owned(canvas),
                ..gif::Frame::default()
            };
            enc.write_frame(&frame).map_err(encode_err)?;
        }
    }
    Ok(out)
}

/// RGBA preview on a uniform canvas (the union of the images' boxes): the
/// current frame opaque on top of `next` and `prev`, which are drawn at
/// `fade` (0..=1) opacity. Returns `(width, height, rgba)`.
pub fn onion_composite(
    current: &IndexedImage,
    prev: Option<&IndexedImage>,
    next: Option<&IndexedImage>,
    palette: &Palette,
    fade: f32,
) -> (u32, u32, Vec<u8>) {
    let layers = [(prev, fade), (next, fade), (Some(current), 1.0)];
    let Some((bx, by, w, h)) = union_box(layers.iter().filter_map(|(i, _)| *i)) else {
        return (0, 0, Vec::new());
    };
    if u64::from(w) * u64::from(h) > MAX_CANVAS {
        return (0, 0, Vec::new());
    }
    let mut canvas = vec![0u8; w as usize * h as usize * 4];
    for (img, opacity) in layers {
        let Some(img) = img else { continue };
        let strength = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
        for y in 0..img.height as usize {
            for x in 0..img.width as usize {
                let i = y * img.width as usize + x;
                if img.alpha.get(i).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let src = palette.colors[usize::from(img.pixels.get(i).copied().unwrap_or(0))];
                let cx = (i64::from(img.offset_x) - i64::from(bx)) as usize + x;
                let cy = (i64::from(img.offset_y) - i64::from(by)) as usize + y;
                let o = (cy * w as usize + cx) * 4;
                let dst_a = u32::from(canvas[o + 3]);
                let out_a = strength + dst_a * (255 - strength) / 255;
                if out_a == 0 {
                    continue;
                }
                for c in 0..3 {
                    let s = u32::from(src[c]) * strength;
                    let d = u32::from(canvas[o + c]) * dst_a * (255 - strength) / 255;
                    canvas[o + c] = ((s + d) / out_a).min(255) as u8;
                }
                canvas[o + 3] = out_a.min(255) as u8;
            }
        }
    }
    (w, h, canvas)
}
