//! Selections and their content: wand, copy / cut / paste, move, flip, rotate.

use super::{Dither, FillMode, at, in_frame, pixel, put_xy, region_select, selected};
use crate::error::{Result, invalid};
use crate::mask::Mask;
use crate::session::{FrameRef, PaintSession, canonicalize};
use d2_formats::IndexedImage;

/// Bounding box `(x, y, w, h)` of the selected pixels *inside* the frame.
fn frame_bounds(img: &IndexedImage, mask: &Mask) -> Option<(i32, i32, i32, i32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..img.height as i32 {
        for x in 0..img.width as i32 {
            if mask.get(x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x1 >= x0).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// The selected pixels of the frame, as `(x, y, value)`.
fn lift(img: &IndexedImage, mask: Option<&Mask>) -> Vec<(i32, i32, (u8, u8))> {
    let mut out = Vec::new();
    for y in 0..img.height as i32 {
        for x in 0..img.width as i32 {
            if selected(mask, x, y) {
                out.push((x, y, pixel(img, at(img, x, y))));
            }
        }
    }
    out
}

fn clear(img: &mut IndexedImage, lifted: &[(i32, i32, (u8, u8))]) {
    for &(x, y, _) in lifted {
        put_xy(img, x, y, (0, 0));
    }
}

/// Write the opaque lifted pixels at `map(x, y)`; returns the written mask.
fn drop_lifted(
    img: &mut IndexedImage,
    lifted: &[(i32, i32, (u8, u8))],
    map: impl Fn(i32, i32) -> (i64, i64),
) -> Mask {
    let mut written = Mask::none(img.width, img.height);
    for &(x, y, v) in lifted {
        let (nx, ny) = map(x, y);
        if v.1 != 0 && v.0 != 0 && in_frame(img, nx, ny) {
            put_xy(img, nx as i32, ny as i32, v);
            written.set(nx as i32, ny as i32, true);
        }
    }
    written
}

fn check_clip(clip: &IndexedImage) -> Result<()> {
    let len = (clip.width as usize).saturating_mul(clip.height as usize);
    if clip.pixels.len() != len || clip.alpha.len() != len {
        return invalid("clipboard image buffers do not match its size");
    }
    Ok(())
}

const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

impl PaintSession {
    /// The region a click would select: 4-connected same-index pixels
    /// (`Contiguous`) or all of them (`Global`). Transparent pixels select
    /// transparent regions. A missing frame or a point outside it gives an
    /// empty mask.
    pub fn magic_wand(&self, f: FrameRef, at: (i32, i32), mode: FillMode) -> Mask {
        match self.frame(f) {
            Some(img) => region_select(img, at, mode, None),
            None => Mask::none(0, 0),
        }
    }

    /// The selected pixels cropped to the mask's bounding box. `offset_x` and
    /// `offset_y` of the result are the box position **in the frame** (not the
    /// sprite anchor), so `paste(f, &clip, (clip.offset_x, clip.offset_y))`
    /// puts it back in place. Unselected pixels are transparent. A missing
    /// frame or an empty selection gives a 0x0 image.
    pub fn copy(&self, f: FrameRef, mask: &Mask) -> IndexedImage {
        let Some(img) = self.frame(f) else {
            return IndexedImage::new(0, 0, 0, 0);
        };
        copy_from(img, mask)
    }

    /// `copy`, then clear the selected pixels to transparent.
    pub fn cut(&mut self, f: FrameRef, mask: &Mask) -> Result<IndexedImage> {
        let mut img = self.img(f)?.clone();
        let clip = copy_from(&img, mask);
        let lifted = lift(&img, Some(mask));
        clear(&mut img, &lifted);
        self.step("cut", |s| s.commit_image(f, img))?;
        Ok(clip)
    }

    /// Paste with the clip's top-left at `at`. Transparent clip pixels do not
    /// overwrite. Returns the mask of the pixels written (frame-sized).
    pub fn paste(&mut self, f: FrameRef, clip: &IndexedImage, at_: (i32, i32)) -> Result<Mask> {
        check_clip(clip)?;
        let mut img = self.img(f)?.clone();
        let lifted: Vec<_> = (0..clip.height as i32)
            .flat_map(|y| (0..clip.width as i32).map(move |x| (x, y)))
            .map(|(x, y)| (x, y, pixel(clip, at(clip, x, y))))
            .collect();
        let written = drop_lifted(&mut img, &lifted, |x, y| {
            (
                i64::from(at_.0) + i64::from(x),
                i64::from(at_.1) + i64::from(y),
            )
        });
        self.step("paste", |s| s.commit_image(f, img))?;
        Ok(written)
    }

    /// Paste an RGBA image (`width * height * 4` bytes), mapping each colour
    /// to the nearest palette index (never 0). Pixels with alpha below 128 are
    /// transparent and do not overwrite. `Bayer4` adds an ordered threshold of
    /// +-32 per channel before mapping; `FloydSteinberg` diffuses the error
    /// (in sRGB bytes) to the opaque neighbours.
    pub fn paste_rgba(
        &mut self,
        f: FrameRef,
        width: u32,
        height: u32,
        rgba: &[u8],
        at_: (i32, i32),
        dither: Dither,
    ) -> Result<Mask> {
        let need = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if need != Some(rgba.len()) {
            return invalid(format!(
                "rgba buffer has {} bytes, {width}x{height} needs {need:?}",
                rgba.len()
            ));
        }
        let mut img = self.img(f)?.clone();
        let ramps = self.ramps();
        let (w, h) = (width as i32, height as i32);
        let opaque = |x: i32, y: i32| -> bool {
            x >= 0
                && y >= 0
                && x < w
                && y < h
                && rgba[(y as usize * w as usize + x as usize) * 4 + 3] >= 128
        };
        let mut lifted: Vec<(i32, i32, (u8, u8))> = Vec::new();
        let mut err = vec![[0.0f32; 3]; (w as usize) * (h as usize)];
        for y in 0..h {
            for x in 0..w {
                let i = y as usize * w as usize + x as usize;
                if !opaque(x, y) {
                    lifted.push((x, y, (0, 0)));
                    continue;
                }
                let src = [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]];
                let mut want = src.map(f32::from);
                match dither {
                    Dither::None => {}
                    Dither::Bayer4 => {
                        let th = (f32::from(BAYER4[(y & 3) as usize][(x & 3) as usize]) + 0.5)
                            / 16.0
                            - 0.5;
                        for c in &mut want {
                            *c += th * 64.0;
                        }
                    }
                    Dither::FloydSteinberg => {
                        for c in 0..3 {
                            want[c] += err[i][c];
                        }
                    }
                }
                let want_u8 = want.map(|c| c.round().clamp(0.0, 255.0) as u8);
                let idx = ramps.nearest(want_u8, None, false);
                lifted.push((x, y, (idx, 255)));
                if dither == Dither::FloydSteinberg {
                    let got = ramps.color(idx);
                    let e: [f32; 3] =
                        [0, 1, 2].map(|c| want[c].clamp(0.0, 255.0) - f32::from(got[c]));
                    for (nx, ny, wt) in [
                        (x + 1, y, 7.0 / 16.0),
                        (x - 1, y + 1, 3.0 / 16.0),
                        (x, y + 1, 5.0 / 16.0),
                        (x + 1, y + 1, 1.0 / 16.0),
                    ] {
                        if opaque(nx, ny) {
                            let j = ny as usize * w as usize + nx as usize;
                            for c in 0..3 {
                                err[j][c] += e[c] * wt;
                            }
                        }
                    }
                }
            }
        }
        let written = drop_lifted(&mut img, &lifted, |x, y| {
            (
                i64::from(at_.0) + i64::from(x),
                i64::from(at_.1) + i64::from(y),
            )
        });
        self.step("paste image", |s| s.commit_image(f, img))?;
        Ok(written)
    }

    /// Move the selected pixels by `(dx, dy)`: they are lifted, the source is
    /// cleared, and the opaque ones are dropped at the new position (transparent
    /// ones do not overwrite; pixels pushed outside the frame are lost).
    /// Returns the shifted mask.
    pub fn move_selection(&mut self, f: FrameRef, mask: &Mask, dx: i32, dy: i32) -> Result<Mask> {
        let mut img = self.img(f)?.clone();
        let lifted = lift(&img, Some(mask));
        clear(&mut img, &lifted);
        drop_lifted(&mut img, &lifted, |x, y| {
            (i64::from(x) + i64::from(dx), i64::from(y) + i64::from(dy))
        });
        let mut moved = Mask::none(img.width, img.height);
        for &(x, y, _) in &lifted {
            moved.set(
                (i64::from(x) + i64::from(dx)).clamp(-1, i64::from(img.width)) as i32,
                (i64::from(y) + i64::from(dy)).clamp(-1, i64::from(img.height)) as i32,
                true,
            );
        }
        self.step("move", |s| s.commit_image(f, img))?;
        Ok(moved)
    }

    /// Mirror the whole frame (no mask) or the selected content within the
    /// mask's bounding box. Offsets are unchanged.
    pub fn flip(&mut self, f: FrameRef, mask: Option<&Mask>, horizontal: bool) -> Result<()> {
        let mut img = self.img(f)?.clone();
        let (bx, by, bw, bh) = match mask {
            Some(m) => match frame_bounds(&img, m) {
                Some(b) => b,
                None => return Ok(()),
            },
            None => (0, 0, img.width as i32, img.height as i32),
        };
        let lifted = lift(&img, mask);
        clear(&mut img, &lifted);
        drop_lifted(&mut img, &lifted, |x, y| {
            if horizontal {
                (i64::from(bx + (bw - 1 - (x - bx))), i64::from(y))
            } else {
                (i64::from(x), i64::from(by + (bh - 1 - (y - by))))
            }
        });
        self.step("flip", |s| s.commit_image(f, img))
    }

    /// Rotate by 90 degrees. Without a mask the whole frame turns (width and
    /// height swap, the offset moves so the centre stays put). With a mask the
    /// selected content rotates about the centre of its bounding box and is
    /// clipped to the frame.
    pub fn rotate90(&mut self, f: FrameRef, mask: Option<&Mask>, clockwise: bool) -> Result<()> {
        let mut img = self.img(f)?.clone();
        match mask {
            None => {
                let (w, h) = (img.width as i32, img.height as i32);
                let mut out = IndexedImage::new(
                    img.height,
                    img.width,
                    img.offset_x.saturating_add((w - h).div_euclid(2)),
                    img.offset_y.saturating_add((h - w).div_euclid(2)),
                );
                for y in 0..h {
                    for x in 0..w {
                        let (nx, ny) = if clockwise {
                            (h - 1 - y, x)
                        } else {
                            (y, w - 1 - x)
                        };
                        put_xy(&mut out, nx, ny, pixel(&img, at(&img, x, y)));
                    }
                }
                canonicalize(&mut out);
                self.step("rotate", |s| s.commit_image(f, out))
            }
            Some(m) => {
                let Some((bx, by, bw, bh)) = frame_bounds(&img, m) else {
                    return Ok(());
                };
                let lifted = lift(&img, mask);
                clear(&mut img, &lifted);
                let nbx = bx + (bw - bh).div_euclid(2);
                let nby = by + (bh - bw).div_euclid(2);
                drop_lifted(&mut img, &lifted, |x, y| {
                    let (rx, ry) = (x - bx, y - by);
                    let (nx, ny) = if clockwise {
                        (bh - 1 - ry, rx)
                    } else {
                        (ry, bw - 1 - rx)
                    };
                    (i64::from(nbx + nx), i64::from(nby + ny))
                });
                self.step("rotate", |s| s.commit_image(f, img))
            }
        }
    }
}

fn copy_from(img: &IndexedImage, mask: &Mask) -> IndexedImage {
    let Some((bx, by, bw, bh)) = frame_bounds(img, mask) else {
        return IndexedImage::new(0, 0, 0, 0);
    };
    let mut clip = IndexedImage::new(bw as u32, bh as u32, bx, by);
    for y in 0..bh {
        for x in 0..bw {
            if mask.get(bx + x, by + y) {
                put_xy(&mut clip, x, y, pixel(img, at(img, bx + x, by + y)));
            }
        }
    }
    clip
}
