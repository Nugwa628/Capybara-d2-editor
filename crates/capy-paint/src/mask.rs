//! Per-pixel selections.
//!
//! Rectangle and ellipse corners are **inclusive pixel coordinates** (the same
//! convention as the shape tools): `rect(w, h, 2, 2, 4, 3)` selects 3 x 2
//! pixels. Corners may be given in either order and are clipped to the frame.

/// Per-pixel selection for one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    /// Row-major, `width * height` entries.
    pub bits: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskOp {
    Replace,
    Add,
    Subtract,
    Intersect,
}

fn area(w: u32, h: u32) -> usize {
    (w as usize).saturating_mul(h as usize)
}

impl Mask {
    pub fn none(w: u32, h: u32) -> Mask {
        Mask {
            width: w,
            height: h,
            bits: vec![false; area(w, h)],
        }
    }

    pub fn all(w: u32, h: u32) -> Mask {
        Mask {
            width: w,
            height: h,
            bits: vec![true; area(w, h)],
        }
    }

    /// True when no pixel is selected.
    pub fn is_empty(&self) -> bool {
        !self.bits.iter().any(|&b| b)
    }

    /// Number of selected pixels.
    pub fn count(&self) -> usize {
        self.bits.iter().filter(|&&b| b).count()
    }

    /// Is the pixel selected? Out-of-range coordinates are not.
    pub fn get(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return false;
        }
        self.bits
            .get(y as usize * self.width as usize + x as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Select or deselect a pixel; out-of-range coordinates are ignored.
    pub fn set(&mut self, x: i32, y: i32, on: bool) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        if let Some(b) = self.bits.get_mut(i) {
            *b = on;
        }
    }

    /// Bounding box of the selection as `(x, y, width, height)`.
    pub fn bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut any = false;
        let w = self.width.max(1) as usize;
        for (i, &b) in self.bits.iter().enumerate() {
            if b {
                any = true;
                let (x, y) = ((i % w) as u32, (i / w) as u32);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        any.then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Rectangle with inclusive corners.
    pub fn rect(w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> Mask {
        let mut m = Mask::none(w, h);
        // Sides beyond i32 clip at i32::MAX - 1 instead of wrapping.
        let last = |side: u32| i32::try_from(side).unwrap_or(i32::MAX) - 1;
        let (xa, xb) = (x0.min(x1).max(0), x0.max(x1).min(last(w)));
        let (ya, yb) = (y0.min(y1).max(0), y0.max(y1).min(last(h)));
        for y in ya..=yb {
            for x in xa..=xb {
                m.set(x, y, true);
            }
        }
        m
    }

    /// Polygon selection: a pixel is selected when its centre is inside
    /// (even-odd rule). Polygon points are in pixel coordinates where pixel
    /// `(x, y)` has its centre at `(x + 0.5, y + 0.5)`.
    pub fn lasso(w: u32, h: u32, polygon: &[(f32, f32)]) -> Mask {
        let mut m = Mask::none(w, h);
        if polygon.len() < 3 {
            return m;
        }
        for y in 0..h {
            let py = y as f32 + 0.5;
            for x in 0..w {
                let px = x as f32 + 0.5;
                let mut inside = false;
                let mut j = polygon.len() - 1;
                for i in 0..polygon.len() {
                    let (xi, yi) = polygon[i];
                    let (xj, yj) = polygon[j];
                    if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                if inside {
                    m.set(x as i32, y as i32, true);
                }
            }
        }
        m
    }

    /// Ellipse inscribed in the box with inclusive corners; a pixel is
    /// selected when its centre is inside.
    pub fn ellipse(w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> Mask {
        let mut m = Mask::none(w, h);
        for (x, y) in Ellipse::new(x0, y0, x1, y1).pixels(w, h, 0) {
            m.set(x, y, true);
        }
        m
    }

    /// Combine with another mask. Pixels outside `other`'s extent count as
    /// unselected there.
    pub fn combine(&mut self, other: &Mask, op: MaskOp) {
        let w = self.width as usize;
        for (i, bit) in self.bits.iter_mut().enumerate() {
            let (x, y) = ((i % w.max(1)) as i32, (i / w.max(1)) as i32);
            let o = other.get(x, y);
            *bit = match op {
                MaskOp::Replace => o,
                MaskOp::Add => *bit || o,
                MaskOp::Subtract => *bit && !o,
                MaskOp::Intersect => *bit && o,
            };
        }
    }

    pub fn invert(&mut self) {
        for b in &mut self.bits {
            *b = !*b;
        }
    }

    /// Grow by `px` pixels (square neighbourhood).
    pub fn grow(&self, px: u32) -> Mask {
        let r = self.radius(px);
        // Separable max filter: horizontal then vertical.
        let horiz = self.filter_axis(r, true, true);
        horiz.filter_axis(r, false, true)
    }

    /// Shrink by `px` pixels (square neighbourhood). The frame edge does not
    /// erode the selection.
    pub fn shrink(&self, px: u32) -> Mask {
        let r = self.radius(px);
        let horiz = self.filter_axis(r, true, false);
        horiz.filter_axis(r, false, false)
    }

    /// Filter radius: `px`, at most the longer side (beyond it the result
    /// cannot change) and at most `i32::MAX`.
    fn radius(&self, px: u32) -> i32 {
        i32::try_from(px.min(self.width.max(self.height))).unwrap_or(i32::MAX)
    }

    /// One pass of a min (`grow == false`) or max (`grow == true`) filter of
    /// radius `r` along one axis. Out-of-range neighbours are skipped.
    fn filter_axis(&self, r: i32, horizontal: bool, grow: bool) -> Mask {
        let mut out = Mask::none(self.width, self.height);
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                let mut hit = !grow;
                for d in -r..=r {
                    let (nx, ny) = if horizontal {
                        (x.saturating_add(d), y)
                    } else {
                        (x, y.saturating_add(d))
                    };
                    if nx < 0 || ny < 0 || nx >= self.width as i32 || ny >= self.height as i32 {
                        continue;
                    }
                    let v = self.get(nx, ny);
                    if grow && v {
                        hit = true;
                        break;
                    }
                    if !grow && !v {
                        hit = false;
                        break;
                    }
                }
                out.set(x, y, hit);
            }
        }
        out
    }
}

/// The ellipse inscribed in an inclusive pixel box. A pixel belongs to it when
/// its centre is inside; the test uses exact integer arithmetic, so shapes
/// are symmetric and identical on every platform.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ellipse {
    xa: i64,
    xb: i64,
    ya: i64,
    yb: i64,
}

impl Ellipse {
    pub(crate) fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Ellipse {
        Ellipse {
            xa: i64::from(x0.min(x1)),
            xb: i64::from(x0.max(x1)),
            ya: i64::from(y0.min(y1)),
            yb: i64::from(y0.max(y1)),
        }
    }

    pub(crate) fn contains(&self, x: i64, y: i64) -> bool {
        if x < self.xa || x > self.xb || y < self.ya || y > self.yb {
            return false;
        }
        let w = (self.xb - self.xa + 1) as u128;
        let h = (self.yb - self.ya + 1) as u128;
        // Half-pixel units: centre sums, so no division is needed. Inside the
        // box |dx| < w and |dy| < h, so each product stays below 2^128.
        let dx = (2 * x + 1 - (self.xa + self.xb + 1)).unsigned_abs() as u128;
        let dy = (2 * y + 1 - (self.ya + self.yb + 1)).unsigned_abs() as u128;
        if w > 1 << 30 || h > 1 << 30 {
            // Boxes this large are far beyond any frame; exactness no longer matters.
            let (fx, fy) = (dx as f64 / w as f64, dy as f64 / h as f64);
            return fx * fx + fy * fy <= 1.0;
        }
        dx * dx * h * h + dy * dy * w * w <= w * w * h * h
    }

    /// Pixels of the ellipse inside `[-margin, w + margin) x [-margin, h + margin)`.
    pub(crate) fn pixels(&self, w: u32, h: u32, margin: i64) -> Vec<(i32, i32)> {
        let x0 = self.xa.max(-margin);
        let x1 = self.xb.min(i64::from(w) + margin);
        let y0 = self.ya.max(-margin);
        let y1 = self.yb.min(i64::from(h) + margin);
        let mut out = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                if self.contains(x, y) {
                    out.push((x as i32, y as i32));
                }
            }
        }
        out
    }

    /// Pixels of the one-pixel boundary: members with a 4-neighbour outside.
    pub(crate) fn outline(&self, w: u32, h: u32, margin: i64) -> Vec<(i32, i32)> {
        self.pixels(w, h, margin)
            .into_iter()
            .filter(|&(x, y)| {
                let (x, y) = (i64::from(x), i64::from(y));
                !(self.contains(x - 1, y)
                    && self.contains(x + 1, y)
                    && self.contains(x, y - 1)
                    && self.contains(x, y + 1))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(m: &Mask) -> Vec<String> {
        (0..m.height as i32)
            .map(|y| {
                (0..m.width as i32)
                    .map(|x| if m.get(x, y) { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn rect_is_inclusive_and_clipped() {
        let m = Mask::rect(5, 4, 3, 2, 1, 1);
        assert_eq!(rows(&m), [".....", ".###.", ".###.", "....."]);
        let m = Mask::rect(3, 3, -5, -5, 1, 10);
        assert_eq!(rows(&m), ["##.", "##.", "##."]);
        assert!(Mask::rect(3, 3, 5, 5, 9, 9).is_empty());
        assert_eq!(m.bounds(), Some((0, 0, 2, 3)));
        assert_eq!(Mask::none(3, 3).bounds(), None);
    }

    #[test]
    fn ellipse_and_lasso() {
        let m = Mask::ellipse(5, 5, 0, 0, 4, 4);
        assert_eq!(rows(&m), [".###.", "#####", "#####", "#####", ".###."]);
        let tri = Mask::lasso(4, 4, &[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)]);
        assert_eq!(rows(&tri), ["###.", "##..", "#...", "...."]);
        assert!(Mask::lasso(4, 4, &[(0.0, 0.0), (4.0, 4.0)]).is_empty());
    }

    #[test]
    fn combine_ops() {
        let a = Mask::rect(4, 1, 0, 0, 2, 0);
        let b = Mask::rect(4, 1, 1, 0, 3, 0);
        let run = |op| {
            let mut m = a.clone();
            m.combine(&b, op);
            rows(&m)[0].clone()
        };
        assert_eq!(run(MaskOp::Replace), ".###");
        assert_eq!(run(MaskOp::Add), "####");
        assert_eq!(run(MaskOp::Subtract), "#...");
        assert_eq!(run(MaskOp::Intersect), ".##.");
        let mut inv = a.clone();
        inv.invert();
        assert_eq!(rows(&inv)[0], "...#");
    }

    #[test]
    fn grow_and_shrink() {
        let m = Mask::rect(5, 5, 2, 2, 2, 2);
        let g = m.grow(1);
        assert_eq!(rows(&g), [".....", ".###.", ".###.", ".###.", "....."]);
        assert_eq!(g.shrink(1), m);
        // The frame edge does not erode.
        let full = Mask::all(3, 3);
        assert_eq!(full.shrink(2), full);
        assert_eq!(m.grow(0), m);
    }
}
