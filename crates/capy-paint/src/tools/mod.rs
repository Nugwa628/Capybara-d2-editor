//! The editing tools, as methods on [`PaintSession`](crate::PaintSession).
//!
//! # Conventions
//!
//! - Coordinates are frame pixels; points outside the frame are clipped (a
//!   huge coordinate costs time proportional to the frame, not the distance).
//! - Corner pairs `(a, b)` of rectangles and ellipses are inclusive pixels.
//! - `mask: Some(m)` restricts a tool to the selected pixels. Tools that
//!   relocate pixels (`move_selection`, `flip`, `rotate90` with a mask)
//!   treat the mask as the lifted content: the result lands inside the
//!   mask's bounding box (flip) or around its centre (rotate). `outline`
//!   with a mask outlines the *mask* instead of the opaque pixels (see
//!   [`PaintSession::outline`](crate::PaintSession::outline)).
//! - `Paint::Index(0)` is rejected with `Invalid`: index 0 is the transparent
//!   key and opaque pixels never use it.
//! - A tool that would change nothing returns `Ok` and records no history.

mod draw;
mod effects;
mod frames;
mod select;

use crate::error::{Result, invalid};
use crate::mask::Mask;
use crate::session::{Brush, BrushShape, Paint};
use crate::{RampId, session::canonicalize};
use d2_formats::IndexedImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    Contiguous,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeStyle {
    Outline,
    Filled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientShape {
    Linear,
    Radial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dither {
    None,
    Bayer4,
    FloydSteinberg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GradientColors {
    Two(u8, u8),
    /// Dark to light along a ramp.
    Ramp(RampId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineColor {
    Fixed(u8),
    /// The neighbouring opaque pixel's colour, this many ramp steps darker
    /// (`shift(index, -n)`).
    AutoDarker(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineSide {
    Outside,
    Inside,
}

/// A pixel value as stored: `(index, alpha)`.
pub(crate) type Px = (u8, u8);

pub(crate) fn paint_value(p: Paint) -> Result<Px> {
    match p {
        Paint::Index(0) => {
            invalid("palette index 0 is the transparent key; use Paint::Transparent")
        }
        Paint::Index(i) => Ok((i, 255)),
        Paint::Transparent => Ok((0, 0)),
    }
}

pub(crate) fn opaque_index(i: u8) -> Result<u8> {
    if i == 0 {
        invalid("palette index 0 is the transparent key")
    } else {
        Ok(i)
    }
}

/// Maximum brush side, to bound the cost of a stamp.
const MAX_BRUSH: u32 = 256;

pub(crate) fn selected(mask: Option<&Mask>, x: i32, y: i32) -> bool {
    mask.is_none_or(|m| m.get(x, y))
}

pub(crate) fn in_frame(img: &IndexedImage, x: i64, y: i64) -> bool {
    x >= 0 && y >= 0 && x < i64::from(img.width) && y < i64::from(img.height)
}

pub(crate) fn at(img: &IndexedImage, x: i32, y: i32) -> usize {
    y as usize * img.width as usize + x as usize
}

pub(crate) fn pixel(img: &IndexedImage, i: usize) -> Px {
    (
        img.pixels.get(i).copied().unwrap_or(0),
        img.alpha.get(i).copied().unwrap_or(0),
    )
}

pub(crate) fn put(img: &mut IndexedImage, i: usize, v: Px) {
    if let (Some(p), Some(a)) = (img.pixels.get_mut(i), img.alpha.get_mut(i)) {
        *p = v.0;
        *a = v.1;
    }
}

pub(crate) fn put_xy(img: &mut IndexedImage, x: i32, y: i32, v: Px) {
    let i = at(img, x, y);
    put(img, i, v);
}

/// Write `v` to every pixel of `touched` that `mask` allows.
pub(crate) fn apply(img: &mut IndexedImage, touched: &Mask, mask: Option<&Mask>, v: Px) {
    for y in 0..img.height as i32 {
        for x in 0..img.width as i32 {
            if touched.get(x, y) && selected(mask, x, y) {
                put_xy(img, x, y, v);
            }
        }
    }
    canonicalize(img);
}

/// Offsets (relative to the brush position) covered by one stamp.
///
/// A square brush of size `n` covers `x - (n-1)/2 ..= x - (n-1)/2 + n - 1`
/// (so even sizes extend one pixel further right/down). A round brush keeps
/// the offsets within a radius of `(n-1)/2 + 0.25` of the brush centre.
pub(crate) fn brush_offsets(brush: Brush) -> Vec<(i32, i32)> {
    let n = brush.size.clamp(1, MAX_BRUSH) as i32;
    let lo = -((n - 1) / 2);
    let centre2 = n - 1; // twice the centre, relative to the first column
    // Compare in quarter-pixel units to stay in integers: radius = (n-1)/2 + 1/4.
    let r4 = 2 * (n - 1) + 1; // radius * 4
    let mut out = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let keep = match brush.shape {
                BrushShape::Square => true,
                BrushShape::Round => {
                    let dx4 = 2 * (2 * i - centre2); // (i - c) * 4
                    let dy4 = 2 * (2 * j - centre2);
                    dx4 * dx4 + dy4 * dy4 <= r4 * r4
                }
            };
            if keep {
                out.push((lo + i, lo + j));
            }
        }
    }
    out
}

/// Stamp the brush at every point of `points` into `touched`.
pub(crate) fn stamp(touched: &mut Mask, offsets: &[(i32, i32)], points: &[(i32, i32)]) {
    for &(px, py) in points {
        for &(ox, oy) in offsets {
            let (x, y) = (i64::from(px) + i64::from(ox), i64::from(py) + i64::from(oy));
            if x >= 0 && y >= 0 && x < i64::from(touched.width) && y < i64::from(touched.height) {
                touched.set(x as i32, y as i32, true);
            }
        }
    }
}

/// Pixels of the line from `a` to `b` that fall within the frame extended by
/// `margin` pixels. The set is independent of direction (`a`/`b` swapped gives
/// the same pixels) and of clipping, because every pixel is computed from the
/// exact line equation instead of by stepping.
pub(crate) fn line_points(
    a: (i32, i32),
    b: (i32, i32),
    w: u32,
    h: u32,
    margin: i64,
) -> Vec<(i32, i32)> {
    let (ax, ay, bx, by) = (
        i64::from(a.0),
        i64::from(a.1),
        i64::from(b.0),
        i64::from(b.1),
    );
    let steep = (by - ay).abs() > (bx - ax).abs();
    // Major axis first.
    let (mut p0, mut p1) = if steep {
        ((ay, ax), (by, bx))
    } else {
        ((ax, ay), (bx, by))
    };
    if p0.0 > p1.0 || (p0.0 == p1.0 && p0.1 > p1.1) {
        std::mem::swap(&mut p0, &mut p1);
    }
    let (lo, hi) = if steep {
        (0, i64::from(h))
    } else {
        (0, i64::from(w))
    };
    let start = p0.0.max(lo - margin);
    let end = p1.0.min(hi + margin);
    let major_len = p1.0 - p0.0;
    let mut out = Vec::new();
    for m in start..=end {
        let minor: i64 = if major_len == 0 {
            p0.1
        } else {
            let num = i128::from(2 * (p1.1 - p0.1)) * i128::from(m - p0.0) + i128::from(major_len);
            (i128::from(p0.1) + num.div_euclid(i128::from(2 * major_len))) as i64
        };
        out.push(if steep {
            (minor as i32, m as i32)
        } else {
            (m as i32, minor as i32)
        });
    }
    out
}

/// Which pixels does a flood from `at` select? Exact (index, alpha) match.
/// `restrict` limits where the region may spread. Out-of-frame start: empty.
pub(crate) fn region_select(
    img: &IndexedImage,
    start: (i32, i32),
    mode: FillMode,
    restrict: Option<&Mask>,
) -> Mask {
    let mut out = Mask::none(img.width, img.height);
    if !in_frame(img, i64::from(start.0), i64::from(start.1))
        || !selected(restrict, start.0, start.1)
    {
        return out;
    }
    let target = pixel(img, at(img, start.0, start.1));
    match mode {
        FillMode::Global => {
            for y in 0..img.height as i32 {
                for x in 0..img.width as i32 {
                    if pixel(img, at(img, x, y)) == target && selected(restrict, x, y) {
                        out.set(x, y, true);
                    }
                }
            }
        }
        FillMode::Contiguous => {
            let mut stack = vec![start];
            out.set(start.0, start.1, true);
            while let Some((x, y)) = stack.pop() {
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    if in_frame(img, i64::from(nx), i64::from(ny))
                        && !out.get(nx, ny)
                        && pixel(img, at(img, nx, ny)) == target
                        && selected(restrict, nx, ny)
                    {
                        out.set(nx, ny, true);
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }
    out
}
