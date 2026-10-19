//! Pencil, shapes, fill, replace colour and the picker.

use super::{
    FillMode, ShapeStyle, apply, at, brush_offsets, line_points, paint_value, pixel, put,
    region_select, selected, stamp,
};
use crate::error::Result;
use crate::mask::{Ellipse, Mask};
use crate::session::{Brush, FrameRef, Paint, PaintSession, canonicalize};

/// Pixels beyond the frame that are still walked, so a wide brush centred just
/// outside the frame still paints its overlap.
fn margin(brush: Brush) -> i64 {
    i64::from(brush.size.min(256)) + 1
}

impl PaintSession {
    /// Stamps the brush at every point and joins consecutive points with
    /// straight lines, so sparse mouse samples still make a continuous stroke.
    /// Square brushes of even size extend one pixel to the right and down.
    pub fn pencil(
        &mut self,
        f: FrameRef,
        points: &[(i32, i32)],
        brush: Brush,
        paint: Paint,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let v = paint_value(paint)?;
        let mut img = self.img(f)?.clone();
        let mut touched = Mask::none(img.width, img.height);
        let offsets = brush_offsets(brush);
        let m = margin(brush);
        if let Some(&first) = points.first() {
            stamp(&mut touched, &offsets, &[first]);
        }
        for pair in points.windows(2) {
            let line = line_points(pair[0], pair[1], img.width, img.height, m);
            stamp(&mut touched, &offsets, &line);
        }
        apply(&mut img, &touched, mask, v);
        self.step("pencil", |s| s.commit_image(f, img))
    }

    /// A line with the brush stamped along it. `line(a, b)` and `line(b, a)`
    /// paint the same pixels.
    pub fn line(
        &mut self,
        f: FrameRef,
        a: (i32, i32),
        b: (i32, i32),
        brush: Brush,
        paint: Paint,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let v = paint_value(paint)?;
        let mut img = self.img(f)?.clone();
        let mut touched = Mask::none(img.width, img.height);
        let line = line_points(a, b, img.width, img.height, margin(brush));
        stamp(&mut touched, &brush_offsets(brush), &line);
        apply(&mut img, &touched, mask, v);
        self.step("line", |s| s.commit_image(f, img))
    }

    /// Rectangle between the inclusive corners `a` and `b`. `Outline` strokes
    /// the four edges with the brush; `Filled` paints every pixel (the brush
    /// is ignored).
    pub fn rectangle(
        &mut self,
        f: FrameRef,
        a: (i32, i32),
        b: (i32, i32),
        style: ShapeStyle,
        brush: Brush,
        paint: Paint,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let v = paint_value(paint)?;
        let mut img = self.img(f)?.clone();
        let mut touched = Mask::none(img.width, img.height);
        match style {
            ShapeStyle::Filled => {
                touched = Mask::rect(img.width, img.height, a.0, a.1, b.0, b.1);
            }
            ShapeStyle::Outline => {
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                let offsets = brush_offsets(brush);
                let m = margin(brush);
                for (p, q) in [
                    ((x0, y0), (x1, y0)),
                    ((x0, y1), (x1, y1)),
                    ((x0, y0), (x0, y1)),
                    ((x1, y0), (x1, y1)),
                ] {
                    stamp(
                        &mut touched,
                        &offsets,
                        &line_points(p, q, img.width, img.height, m),
                    );
                }
            }
        }
        apply(&mut img, &touched, mask, v);
        self.step("rectangle", |s| s.commit_image(f, img))
    }

    /// Ellipse inscribed in the inclusive box `a`..`b`.
    pub fn ellipse(
        &mut self,
        f: FrameRef,
        a: (i32, i32),
        b: (i32, i32),
        style: ShapeStyle,
        brush: Brush,
        paint: Paint,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let v = paint_value(paint)?;
        let mut img = self.img(f)?.clone();
        let mut touched = Mask::none(img.width, img.height);
        let shape = Ellipse::new(a.0, a.1, b.0, b.1);
        let m = margin(brush);
        match style {
            ShapeStyle::Filled => {
                for (x, y) in shape.pixels(img.width, img.height, 0) {
                    touched.set(x, y, true);
                }
            }
            ShapeStyle::Outline => {
                let ring = shape.outline(img.width, img.height, m);
                stamp(&mut touched, &brush_offsets(brush), &ring);
            }
        }
        apply(&mut img, &touched, mask, v);
        self.step("ellipse", |s| s.commit_image(f, img))
    }

    /// Flood fill from `at`, matching the exact index (or transparency) of
    /// the pixel there. `Contiguous` spreads 4-connected; `Global` takes every
    /// matching pixel. With a mask, only selected pixels are filled and the
    /// flood cannot cross unselected ones. A start outside the frame or
    /// outside the mask is a no-op.
    pub fn fill(
        &mut self,
        f: FrameRef,
        at: (i32, i32),
        paint: Paint,
        mode: FillMode,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let v = paint_value(paint)?;
        let mut img = self.img(f)?.clone();
        let region = region_select(&img, at, mode, mask);
        apply(&mut img, &region, None, v);
        self.step("fill", |s| s.commit_image(f, img))
    }

    /// Replace every pixel equal to `from` with `to`, in one frame or (None)
    /// in the whole sprite as one undo step.
    pub fn replace_color(
        &mut self,
        f: Option<FrameRef>,
        from: Paint,
        to: Paint,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let from = paint_value(from)?;
        let to = paint_value(to)?;
        let targets: Vec<FrameRef> = match f {
            Some(f) => {
                self.img(f)?;
                vec![f]
            }
            None => self
                .frames
                .iter()
                .enumerate()
                .flat_map(|(dir, d)| (0..d.len()).map(move |frame| FrameRef { dir, frame }))
                .collect(),
        };
        self.step("replace colour", |s| {
            for t in targets {
                let mut img = s.img(t)?.clone();
                for y in 0..img.height as i32 {
                    for x in 0..img.width as i32 {
                        let i = at(&img, x, y);
                        if pixel(&img, i) == from && selected(mask, x, y) {
                            put(&mut img, i, to);
                        }
                    }
                }
                canonicalize(&mut img);
                s.commit_image(t, img)?;
            }
            Ok(())
        })
    }

    /// The colour under a point: `Some(Paint::Index)` or `Some(Paint::Transparent)`,
    /// `None` outside the frame or for a missing frame.
    pub fn pick(&self, f: FrameRef, at_: (i32, i32)) -> Option<Paint> {
        let img = self.frame(f)?;
        if at_.0 < 0 || at_.1 < 0 || at_.0 as u32 >= img.width || at_.1 as u32 >= img.height {
            return None;
        }
        let (idx, alpha) = pixel(img, at(img, at_.0, at_.1));
        Some(if alpha == 0 {
            Paint::Transparent
        } else {
            Paint::Index(idx)
        })
    }
}
