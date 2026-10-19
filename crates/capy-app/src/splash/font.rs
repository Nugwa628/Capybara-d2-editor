//! The loading screen's title in one of the game's own fonts
//! (`data\local\font\latin\<name>.tbl` + `.dc6`), drawn the way the old
//! editor's `Art.text(..., solid=True)` did: every glyph pixel in one colour
//! with a dark 1 px outline, cropped to the rows that hold ink.
//!
//! Font `.tbl` layout (classic D2): a 12-byte header, then 14-byte entries
//! `u16 char, u8, u8 width (advance), u8 height, u8, u16, u16 DC6 frame, u32`.
//! Only the advance and the frame are used; glyph placement comes from the
//! DC6 frame's own size and offsets.

use std::collections::HashMap;

use d2_formats::dc6::Dc6;

use crate::branding::RgbaImage;

/// The outline colour of the old program (dark brown, alpha 230).
const OUTLINE: [u8; 4] = [12, 8, 4, 230];
const HEADER: usize = 12;
const ENTRY: usize = 14;

/// One glyph's ink: which pixels of its DC6 frame are set.
#[derive(Clone, Debug)]
struct GlyphInk {
    width: usize,
    height: usize,
    offset_x: i64,
    offset_y: i64,
    ink: Vec<bool>,
}

#[derive(Clone, Debug)]
struct Glyph {
    advance: i64,
    frame: GlyphInk,
}

/// A game font: character code -> glyph.
#[derive(Clone, Debug, Default)]
pub struct GameFont {
    glyphs: HashMap<u16, Glyph>,
}

impl GameFont {
    /// Parse the `.tbl` and its `.dc6`. Entries naming a missing frame are
    /// skipped; a later entry for the same character replaces an earlier one.
    pub fn parse(tbl: &[u8], dc6: &[u8]) -> Result<GameFont, String> {
        let dc6 = Dc6::parse(dc6).map_err(|e| format!("font dc6: {e}"))?;
        let mut glyphs = HashMap::new();
        let count = tbl.len().saturating_sub(HEADER) / ENTRY;
        for i in 0..count {
            let e = &tbl[HEADER + i * ENTRY..HEADER + (i + 1) * ENTRY];
            let code = u16::from_le_bytes([e[0], e[1]]);
            let advance = i64::from(e[3]);
            let frame_index = usize::from(u16::from_le_bytes([e[8], e[9]]));
            let Some(frame) = dc6.frames.get(frame_index) else {
                continue;
            };
            let image = frame.render().map_err(|e| format!("font glyph: {e}"))?;
            glyphs.insert(
                code,
                Glyph {
                    advance,
                    frame: GlyphInk {
                        width: image.width as usize,
                        height: image.height as usize,
                        offset_x: i64::from(frame.offset_x),
                        offset_y: i64::from(frame.offset_y),
                        ink: image.alpha.iter().map(|&a| a != 0).collect(),
                    },
                },
            );
        }
        if glyphs.is_empty() {
            return Err("the font has no glyphs".to_owned());
        }
        Ok(GameFont { glyphs })
    }

    fn glyph(&self, c: char) -> Option<&Glyph> {
        let code = |c: char| u16::try_from(u32::from(c)).ok();
        code(c)
            .and_then(|k| self.glyphs.get(&k))
            .or_else(|| code('?').and_then(|k| self.glyphs.get(&k)))
    }

    /// `text` in `color`, outlined and cropped (see the module docs).
    pub fn render_solid(&self, text: &str, color: [u8; 3]) -> RgbaImage {
        let chars: Vec<&Glyph> = text.chars().filter_map(|c| self.glyph(c)).collect();
        let width = chars.iter().map(|g| g.advance).sum::<i64>().max(1);
        let top = chars
            .iter()
            .map(|g| g.frame.height as i64 - g.frame.offset_y)
            .chain([1])
            .max()
            .unwrap_or(1);
        let bottom = chars
            .iter()
            .map(|g| g.frame.offset_y)
            .chain([0])
            .max()
            .unwrap_or(0);
        let height = (top + bottom).max(1);
        let (w, h) = (width as usize, height as usize);
        let mut on0 = vec![false; w * h];
        let mut x = 0i64;
        for g in &chars {
            let f = &g.frame;
            let y0 = top - (f.height as i64 - f.offset_y);
            for yy in 0..f.height {
                let ty = y0 + yy as i64;
                if !(0..height).contains(&ty) {
                    continue;
                }
                for xx in 0..f.width {
                    let tx = x + f.offset_x + xx as i64;
                    if f.ink[yy * f.width + xx] && (0..width).contains(&tx) {
                        on0[ty as usize * w + tx as usize] = true;
                    }
                }
            }
            x += g.advance;
        }
        solid(&on0, w, h, color)
    }
}

/// `_solid` + the row crop: grow the canvas by 1 px on every side, ink in
/// `color`, the 8-neighbourhood of the ink in the outline colour.
fn solid(on0: &[bool], w: usize, h: usize, color: [u8; 3]) -> RgbaImage {
    let (ww, hh) = (w + 2, h + 2);
    let on = |x: i64, y: i64| -> bool {
        x > 0
            && y > 0
            && (x as usize) < ww - 1
            && (y as usize) < hh - 1
            && on0[(y as usize - 1) * w + x as usize - 1]
    };
    let mut data = vec![0u8; ww * hh * 4];
    for y in 0..hh as i64 {
        for x in 0..ww as i64 {
            let j = (y as usize * ww + x as usize) * 4;
            if on(x, y) {
                data[j..j + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
            } else if (-1..=1).any(|dx| (-1..=1).any(|dy| on(x + dx, y + dy))) {
                data[j..j + 4].copy_from_slice(&OUTLINE);
            }
        }
    }
    let inked = |y: usize| (0..ww).any(|x| data[(y * ww + x) * 4 + 3] != 0);
    let rows: Vec<usize> = (0..hh).filter(|&y| inked(y)).collect();
    match (rows.first(), rows.last()) {
        (Some(&y0), Some(&y1)) => RgbaImage {
            width: ww,
            height: y1 + 1 - y0,
            data: data[y0 * ww * 4..(y1 + 1) * ww * 4].to_vec(),
        },
        _ => RgbaImage {
            width: ww,
            height: hh,
            data,
        },
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use d2_formats::dc6::{Dc6, Dc6Frame};

    /// A DC6 with one direction of solid rectangular glyph frames.
    pub fn synthetic_font(glyphs: &[(u16, u8, u32, u32, i32)]) -> (Vec<u8>, Vec<u8>) {
        let frames: Vec<Dc6Frame> = glyphs
            .iter()
            .map(|&(_, _, w, h, oy)| {
                let mut img = d2_formats::image::IndexedImage::new(w, h, 0, 0);
                img.pixels.iter_mut().for_each(|p| *p = 7);
                img.alpha.iter_mut().for_each(|a| *a = 255);
                let mut f = Dc6Frame::from_image(&img);
                f.offset_y = oy;
                f
            })
            .collect();
        let dc6 = Dc6 {
            version: 6,
            flags: 1,
            encoding: 0,
            directions: 1,
            frames_per_direction: frames.len() as u32,
            frames,
        };
        let mut tbl = b"Woo!\x01\x00\x00\x00\x0a\x0c\x00\x00".to_vec();
        for (i, &(code, adv, _, h, _)) in glyphs.iter().enumerate() {
            tbl.extend_from_slice(&code.to_le_bytes());
            tbl.extend_from_slice(&[0, adv, h as u8, 0, 0, 0]);
            tbl.extend_from_slice(&(i as u16).to_le_bytes());
            tbl.extend_from_slice(&[0; 4]);
        }
        (tbl, dc6.to_bytes().expect("dc6 bytes"))
    }

    #[test]
    fn glyphs_advance_sit_on_the_baseline_and_get_an_outline() {
        // 'A': 3x4 ink, advance 5; 'g': 2x4 with offset_y 2 (descender);
        // '?': 1x1.
        let (tbl, dc6) = synthetic_font(&[
            (b'A' as u16, 5, 3, 4, 0),
            (b'g' as u16, 3, 2, 4, 2),
            (b'?' as u16, 2, 1, 1, 0),
        ]);
        let font = GameFont::parse(&tbl, &dc6).expect("font");
        let img = font.render_solid("Ag", [240, 200, 90]);
        // top = max(4 - 0, 4 - 2, 1) = 4, bottom = 2 -> 6 rows of ink, + 2 outline rows
        assert_eq!((img.width, img.height), (5 + 3 + 2, 8));
        let at = |x: usize, y: usize| {
            let i = (y * img.width + x) * 4;
            [
                img.data[i],
                img.data[i + 1],
                img.data[i + 2],
                img.data[i + 3],
            ]
        };
        assert_eq!(at(1, 1), [240, 200, 90, 255]); // 'A' top-left ink
        assert_eq!(at(0, 0), OUTLINE);
        assert_eq!(at(4, 1), OUTLINE); // right of 'A'
        assert_eq!(at(5, 1), [0, 0, 0, 0]); // beyond its outline
        // 'g' starts at x = 5 (+1 border), its rows 2..6 (+1)
        assert_eq!(at(6, 3), [240, 200, 90, 255]);
        assert_eq!(at(6, 6), [240, 200, 90, 255]);
        assert_eq!(at(6, 2), OUTLINE);
        // unknown characters fall back to '?'
        let q = font.render_solid("\u{263a}", [1, 2, 3]);
        assert_eq!(q.width, 2 + 2);
    }

    #[test]
    fn rows_without_ink_are_cropped() {
        let (tbl, dc6) = synthetic_font(&[(b'.' as u16, 2, 1, 1, 0), (b'T' as u16, 4, 3, 6, 0)]);
        let font = GameFont::parse(&tbl, &dc6).expect("font");
        // '.' alone: top = 1 -> 1 row of ink + outline = 3 rows
        assert_eq!(font.render_solid(".", [9, 9, 9]).height, 3);
        // with 'T' the canvas is 6 high, '.' on the baseline
        let img = font.render_solid(".T", [9, 9, 9]);
        assert_eq!(img.height, 8);
        let ink = |x: usize, y: usize| img.data[(y * img.width + x) * 4 + 3] == 255;
        assert!(ink(1, 6) && !ink(1, 5));
    }

    #[test]
    fn broken_tables_are_errors_or_skipped() {
        let (tbl, dc6) = synthetic_font(&[(b'A' as u16, 5, 3, 4, 0)]);
        assert!(GameFont::parse(&tbl, b"junk").is_err());
        assert!(GameFont::parse(&tbl[..HEADER], &dc6).is_err()); // no entries
        let mut bad = tbl.clone();
        bad[HEADER + 8] = 9; // frame 9 does not exist
        assert!(GameFont::parse(&bad, &dc6).is_err());
        assert!(GameFont::parse(&tbl[..HEADER + ENTRY - 1], &dc6).is_err());
    }
}
