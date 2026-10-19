//! Shared helpers for the integration tests.
#![allow(dead_code)]

use capy_paint::{Brush, BrushShape, FrameRef, PaintSession};
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

pub const F0: FrameRef = FrameRef { dir: 0, frame: 0 };

/// Reds 1..=8, greens 9..=16, blues 17..=24 (each dark to light), and a flat
/// light grey for 25..=255. Index 0 is black.
pub fn test_palette() -> Palette {
    let mut colors = [[250u8; 3]; 256];
    colors[0] = [0, 0, 0];
    for k in 0..8u8 {
        let v = 60 + 24 * k;
        colors[1 + usize::from(k)] = [v, 10, 10];
        colors[9 + usize::from(k)] = [10, v, 10];
        colors[17 + usize::from(k)] = [10, 10, v];
    }
    Palette { colors }
}

pub fn sq(size: u32) -> Brush {
    Brush {
        size,
        shape: BrushShape::Square,
    }
}

pub fn round(size: u32) -> Brush {
    Brush {
        size,
        shape: BrushShape::Round,
    }
}

fn parse_char(c: char) -> (u8, u8) {
    match c {
        '.' => (0, 0),
        c => (c.to_digit(36).map_or(1, |d| d as u8), 255),
    }
}

/// Build an image from rows: `.` is transparent, `0-9a-z` are indices 0..=35
/// (so `1` is index 1; index 0 cannot be written and is rejected).
pub fn image(rows: &[&str]) -> IndexedImage {
    let h = rows.len() as u32;
    let w = rows.first().map_or(0, |r| r.chars().count()) as u32;
    let mut img = IndexedImage::new(w, h, 0, 0);
    for (y, row) in rows.iter().enumerate() {
        assert_eq!(row.chars().count() as u32, w, "ragged rows");
        for (x, c) in row.chars().enumerate() {
            let (p, a) = parse_char(c);
            assert!(c == '.' || p != 0, "index 0 in art");
            img.pixels[y * w as usize + x] = p;
            img.alpha[y * w as usize + x] = a;
        }
    }
    img
}

pub fn session(rows: &[&str]) -> PaintSession {
    PaintSession::new(vec![vec![image(rows)]], test_palette()).unwrap()
}

pub fn blank(w: u32, h: u32) -> PaintSession {
    PaintSession::new(vec![vec![IndexedImage::new(w, h, 0, 0)]], test_palette()).unwrap()
}

/// Render an image back to rows (inverse of [`image`]).
pub fn render_img(img: &IndexedImage) -> Vec<String> {
    (0..img.height as usize)
        .map(|y| {
            (0..img.width as usize)
                .map(|x| {
                    let i = y * img.width as usize + x;
                    if img.alpha[i] == 0 {
                        '.'
                    } else {
                        char::from_digit(u32::from(img.pixels[i]), 36).unwrap_or('?')
                    }
                })
                .collect()
        })
        .collect()
}

pub fn render(s: &PaintSession) -> Vec<String> {
    render_img(s.frame(F0).unwrap())
}

pub fn render_at(s: &PaintSession, f: FrameRef) -> Vec<String> {
    render_img(s.frame(f).unwrap())
}

/// Assert the palette-lock invariant on every frame.
pub fn assert_locked(s: &PaintSession) {
    for (d, dir) in s.frames().iter().enumerate() {
        for (i, img) in dir.iter().enumerate() {
            let len = img.width as usize * img.height as usize;
            assert_eq!(img.pixels.len(), len, "frame {d}/{i} pixel len");
            assert_eq!(img.alpha.len(), len, "frame {d}/{i} alpha len");
            for k in 0..len {
                match img.alpha[k] {
                    0 => assert_eq!(
                        img.pixels[k], 0,
                        "transparent pixel with index at {d}/{i}/{k}"
                    ),
                    255 => assert_ne!(img.pixels[k], 0, "opaque index 0 at {d}/{i}/{k}"),
                    a => panic!("alpha {a} at {d}/{i}/{k}"),
                }
            }
        }
    }
}
