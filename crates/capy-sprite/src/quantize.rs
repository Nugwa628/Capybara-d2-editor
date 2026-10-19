//! RGBA to palette indices (and back).
//!
//! Colours are matched by squared RGB distance; ties go to the lowest
//! index. A pixel is transparent when its alpha is below
//! [`QuantizeOptions::alpha_threshold`] or when its (opaque) colour equals
//! [`QuantizeOptions::transparent_rgb`]; transparent pixels become index 0
//! with alpha 0. With [`QuantizeOptions::protect_transparent`] an opaque
//! pixel is never mapped to index 0, so near-black art stays visible.
//!
//! Floyd-Steinberg dithering diffuses the quantization error of opaque
//! pixels only, left to right, top to bottom, and never across transparent
//! pixels. Frames whose pixels are all exact palette colours come out
//! unchanged with or without dithering.

use std::collections::HashMap;

use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use crate::model::RgbaFrame;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dither {
    #[default]
    None,
    FloydSteinberg,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QuantizeOptions {
    pub dither: Dither,
    /// Opaque RGB treated as transparent (D2 convention: black). `None`
    /// means alpha only.
    pub transparent_rgb: Option<[u8; 3]>,
    /// Alpha below this value is transparent.
    pub alpha_threshold: u8,
    /// Never map an opaque pixel to palette index 0.
    pub protect_transparent: bool,
}

impl Default for QuantizeOptions {
    fn default() -> Self {
        QuantizeOptions {
            dither: Dither::None,
            transparent_rgb: Some([0, 0, 0]),
            alpha_threshold: 128,
            protect_transparent: true,
        }
    }
}

struct Matcher<'a> {
    palette: &'a Palette,
    first: usize,
    cache: HashMap<[u8; 3], u8>,
}

impl<'a> Matcher<'a> {
    fn new(palette: &'a Palette, protect_transparent: bool) -> Self {
        Matcher {
            palette,
            first: usize::from(protect_transparent),
            cache: HashMap::new(),
        }
    }

    fn nearest(&mut self, rgb: [u8; 3]) -> u8 {
        if let Some(&i) = self.cache.get(&rgb) {
            return i;
        }
        let mut best = (u32::MAX, self.first as u8);
        for (i, c) in self.palette.colors.iter().enumerate().skip(self.first) {
            let d: u32 = c
                .iter()
                .zip(&rgb)
                .map(|(&a, &b)| u32::from(a.abs_diff(b)).pow(2))
                .sum();
            if d < best.0 {
                best = (d, i as u8);
                if d == 0 {
                    break;
                }
            }
        }
        self.cache.insert(rgb, best.1);
        best.1
    }
}

/// Converts an RGBA frame to palette indices (offsets are copied).
pub fn quantize(frame: &RgbaFrame, palette: &Palette, opts: &QuantizeOptions) -> IndexedImage {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut out = IndexedImage::new(frame.width, frame.height, frame.offset_x, frame.offset_y);
    let mut matcher = Matcher::new(palette, opts.protect_transparent);
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * w + x) * 4;
        match frame.pixels.get(i..i + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0, 0, 0, 0],
        }
    };
    let transparent = |p: [u8; 4]| {
        p[3] < opts.alpha_threshold || opts.transparent_rgb == Some([p[0], p[1], p[2]])
    };
    let dither = opts.dither == Dither::FloydSteinberg;
    // error carried into the current and the next row, three channels per pixel
    let mut cur = vec![[0f32; 3]; w + 2];
    let mut next = vec![[0f32; 3]; w + 2];
    for y in 0..h {
        for x in 0..w {
            let p = pixel(x, y);
            if transparent(p) {
                continue;
            }
            let mut rgb = [p[0], p[1], p[2]];
            if dither {
                let e = cur.get(x + 1).copied().unwrap_or([0.0; 3]);
                for (c, err) in rgb.iter_mut().zip(e) {
                    *c = (f32::from(*c) + err).round().clamp(0.0, 255.0) as u8;
                }
            }
            let idx = matcher.nearest(rgb);
            let i = y * w + x;
            if idx != 0
                && let (Some(px), Some(a)) = (out.pixels.get_mut(i), out.alpha.get_mut(i))
            {
                *px = idx;
                *a = 255;
            }
            if dither {
                let chosen = palette.colors[usize::from(idx)];
                let err: [f32; 3] =
                    std::array::from_fn(|c| f32::from(rgb[c]) - f32::from(chosen[c]));
                let spread = |buf: &mut Vec<[f32; 3]>, at: usize, weight: f32| {
                    if let Some(slot) = buf.get_mut(at) {
                        for (s, e) in slot.iter_mut().zip(err) {
                            *s += e * weight;
                        }
                    }
                };
                spread(&mut cur, x + 2, 7.0 / 16.0);
                spread(&mut next, x, 3.0 / 16.0);
                spread(&mut next, x + 1, 5.0 / 16.0);
                spread(&mut next, x + 2, 1.0 / 16.0);
            }
        }
        if dither {
            std::mem::swap(&mut cur, &mut next);
            next.iter_mut().for_each(|e| *e = [0.0; 3]);
        }
    }
    out
}

/// Palette colours with the image's alpha. Transparent pixels get RGB 0,0,0.
pub fn to_rgba(img: &IndexedImage, palette: &Palette) -> RgbaFrame {
    let mut pixels = img.to_rgba(palette);
    for px in pixels.as_chunks_mut::<4>().0 {
        if px[3] == 0 {
            *px = [0; 4];
        }
    }
    RgbaFrame {
        width: img.width,
        height: img.height,
        offset_x: img.offset_x,
        offset_y: img.offset_y,
        pixels,
    }
}
