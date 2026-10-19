//! Gradient, shading brush, outline and stray-pixel clean-up.

use super::{
    Dither, GradientColors, GradientShape, OutlineColor, OutlineSide, at, brush_offsets, in_frame,
    line_points, opaque_index, pixel, put, put_xy, selected, stamp,
};
use crate::color::LinearTable;
use crate::error::{Result, invalid};
use crate::mask::Mask;
use crate::session::{Brush, FrameRef, PaintSession, canonicalize};
use d2_formats::IndexedImage;

/// Bayer 4x4 threshold matrix (values 0..16).
const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Neighbour order used everywhere a tie needs a deterministic winner:
/// N, W, E, S, then the diagonals NW, NE, SW, SE.
const NEIGHBORS: [(i32, i32); 8] = [
    (0, -1),
    (-1, 0),
    (1, 0),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

fn neighbors(diagonals: bool) -> &'static [(i32, i32)] {
    if diagonals {
        &NEIGHBORS
    } else {
        &NEIGHBORS[..4]
    }
}

/// The colour a gradient has at parameter `t` in `[0, 1]`, in linear light.
struct Source {
    stops: Vec<[f64; 3]>,
}

impl Source {
    fn at(&self, t: f64) -> [f64; 3] {
        let last = self.stops.len() - 1;
        if last == 0 {
            return self.stops[0];
        }
        let p = t.clamp(0.0, 1.0) * last as f64;
        let i = (p.floor() as usize).min(last - 1);
        let frac = p - i as f64;
        let (a, b) = (self.stops[i], self.stops[i + 1]);
        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * frac)
    }
}

impl PaintSession {
    /// Fill the masked pixels (the whole frame without a mask) with a
    /// gradient. Pixel `(x, y)` is sampled at exactly `(x, y)`, so passing the
    /// coordinates of two pixels gives those pixels the end colours.
    ///
    /// The colour is interpolated in linear light between the two colours (or
    /// along the ramp, dark to light) and then mapped to the nearest palette
    /// index (restricted to the ramp for `Ramp`; index 0 is never used).
    /// `Bayer4` perturbs the target by a 4x4 ordered threshold scaled to the
    /// average step between neighbouring candidate colours; `FloydSteinberg`
    /// diffuses the mapping error to unvisited pixels inside the mask.
    /// Linear: `t` is the projection on `from -> to`; radial: the distance from
    /// `from` over the distance to `to`. Both clamp to `[0, 1]`.
    pub fn gradient(
        &mut self,
        f: FrameRef,
        from: (f32, f32),
        to: (f32, f32),
        shape: GradientShape,
        colors: GradientColors,
        dither: Dither,
        mask: Option<&Mask>,
    ) -> Result<()> {
        if ![from.0, from.1, to.0, to.1].iter().all(|v| v.is_finite()) {
            return invalid("gradient endpoints must be finite");
        }
        let mut img = self.img(f)?.clone();
        let table = LinearTable::new();
        let ramps = self.ramps();
        let (within, stops): (Option<Vec<u8>>, Vec<[f64; 3]>) = match &colors {
            GradientColors::Two(a, b) => {
                let (a, b) = (opaque_index(*a)?, opaque_index(*b)?);
                (
                    None,
                    vec![
                        table.to_linear(ramps.color(a)),
                        table.to_linear(ramps.color(b)),
                    ],
                )
            }
            GradientColors::Ramp(id) => {
                let ramp = ramps.ramp(*id);
                if ramp.is_empty() {
                    return invalid(format!("no ramp with id {}", id.0));
                }
                let stops = ramp
                    .iter()
                    .map(|&i| table.to_linear(ramps.color(i)))
                    .collect();
                (Some(ramp.to_vec()), stops)
            }
        };
        let source = Source { stops };
        let map = |lin: [f64; 3]| ramps.nearest(table.to_srgb8(lin), within.as_deref(), false);

        // Average linear step between neighbouring candidates, for Bayer.
        let spread: [f64; 3] = {
            let strip: Vec<[f64; 3]> = match &within {
                Some(_) => source.stops.clone(),
                None => {
                    let mut seen: Vec<u8> = Vec::new();
                    for k in 0..=64 {
                        let i = map(source.at(f64::from(k) / 64.0));
                        if seen.last() != Some(&i) {
                            seen.push(i);
                        }
                    }
                    seen.iter()
                        .map(|&i| table.to_linear(ramps.color(i)))
                        .collect()
                }
            };
            let mut s = [0.0; 3];
            if strip.len() > 1 {
                for w in strip.windows(2) {
                    for c in 0..3 {
                        s[c] += (w[1][c] - w[0][c]).abs();
                    }
                }
                s = s.map(|v| v / (strip.len() - 1) as f64);
            }
            s
        };

        let (fx, fy) = (f64::from(from.0), f64::from(from.1));
        let (tx, ty) = (f64::from(to.0), f64::from(to.1));
        let (dx, dy) = (tx - fx, ty - fy);
        let len2 = dx * dx + dy * dy;
        let t_at = |x: i32, y: i32| -> f64 {
            let (px, py) = (f64::from(x) - fx, f64::from(y) - fy);
            if len2 <= 1e-12 {
                return 0.0;
            }
            match shape {
                GradientShape::Linear => ((px * dx + py * dy) / len2).clamp(0.0, 1.0),
                GradientShape::Radial => ((px * px + py * py) / len2).sqrt().clamp(0.0, 1.0),
            }
        };

        let (w, h) = (img.width as i32, img.height as i32);
        match dither {
            Dither::None | Dither::Bayer4 => {
                for y in 0..h {
                    for x in 0..w {
                        if !selected(mask, x, y) {
                            continue;
                        }
                        let mut lin = source.at(t_at(x, y));
                        if dither == Dither::Bayer4 {
                            let th = (f64::from(BAYER4[(y & 3) as usize][(x & 3) as usize]) + 0.5)
                                / 16.0
                                - 0.5;
                            for c in 0..3 {
                                lin[c] += th * spread[c];
                            }
                        }
                        put_xy(&mut img, x, y, (map(lin), 255));
                    }
                }
            }
            Dither::FloydSteinberg => {
                let mut err = vec![[0.0f64; 3]; (w as usize) * (h as usize)];
                for y in 0..h {
                    for x in 0..w {
                        if !selected(mask, x, y) {
                            continue;
                        }
                        let i = at(&img, x, y);
                        let base = source.at(t_at(x, y));
                        let want = [0, 1, 2].map(|c| (base[c] + err[i][c]).clamp(0.0, 1.0));
                        let chosen = map(want);
                        put(&mut img, i, (chosen, 255));
                        let got = table.to_linear(ramps.color(chosen));
                        let e = [0, 1, 2].map(|c| (want[c] - got[c]).clamp(-1.0, 1.0));
                        for (nx, ny, weight) in [
                            (x + 1, y, 7.0 / 16.0),
                            (x - 1, y + 1, 3.0 / 16.0),
                            (x, y + 1, 5.0 / 16.0),
                            (x + 1, y + 1, 1.0 / 16.0),
                        ] {
                            if in_frame(&img, i64::from(nx), i64::from(ny))
                                && selected(mask, nx, ny)
                            {
                                let j = at(&img, nx, ny);
                                for c in 0..3 {
                                    err[j][c] += e[c] * weight;
                                }
                            }
                        }
                    }
                }
            }
        }
        canonicalize(&mut img);
        self.step("gradient", |s| s.commit_image(f, img))
    }

    /// Lighten (`steps > 0`) or darken (`steps < 0`) the opaque pixels under
    /// the brush along their own ramp, clamped at the ramp ends. Points are
    /// joined by lines like `pencil`; a pixel covered several times in one
    /// call is shifted once. Transparent pixels are untouched.
    pub fn shade(
        &mut self,
        f: FrameRef,
        points: &[(i32, i32)],
        brush: Brush,
        steps: i32,
        mask: Option<&Mask>,
    ) -> Result<()> {
        let mut img = self.img(f)?.clone();
        let mut touched = Mask::none(img.width, img.height);
        let offsets = brush_offsets(brush);
        let m = i64::from(brush.size.min(256)) + 1;
        if let Some(&first) = points.first() {
            stamp(&mut touched, &offsets, &[first]);
        }
        for pair in points.windows(2) {
            stamp(
                &mut touched,
                &offsets,
                &line_points(pair[0], pair[1], img.width, img.height, m),
            );
        }
        let ramps = self.ramps();
        for y in 0..img.height as i32 {
            for x in 0..img.width as i32 {
                if touched.get(x, y) && selected(mask, x, y) {
                    let i = at(&img, x, y);
                    let (idx, alpha) = pixel(&img, i);
                    if alpha != 0 {
                        put(&mut img, i, (ramps.shift(idx, steps), 255));
                    }
                }
            }
        }
        canonicalize(&mut img);
        self.step("shade", |s| s.commit_image(f, img))
    }

    /// Outline the opaque pixels, or the `mask` region when one is given
    /// (`thickness` 1..=8, 0 does nothing).
    ///
    /// - `Outside`: grows `thickness` rings of pixels that are not part of the
    ///   shape, 4-connected (or 8-connected with `diagonals`). Only ring
    ///   pixels are written, wherever they are, so with a mask the outline
    ///   lands outside the selection.
    /// - `Inside`: recolours the shape's own border pixels, `thickness`
    ///   deep. The frame edge counts as outside, so a tightly cropped sprite
    ///   is outlined all round. With a mask this only touches selected pixels.
    ///
    /// `AutoDarker(n)` takes the colour of the shape pixel the ring grew from
    /// (the pixel itself for `Inside`) shifted `n` steps darker along its ramp;
    /// pixels whose source is transparent are skipped. `Fixed(0)` is invalid.
    pub fn outline(
        &mut self,
        f: FrameRef,
        color: OutlineColor,
        side: OutlineSide,
        thickness: u32,
        diagonals: bool,
        mask: Option<&Mask>,
    ) -> Result<()> {
        if let OutlineColor::Fixed(i) = color {
            opaque_index(i)?;
        }
        let img = self.img(f)?.clone();
        let thickness = thickness.min(8);
        if thickness == 0 {
            return Ok(());
        }
        let (w, h) = (img.width as i32, img.height as i32);
        let in_shape = |x: i32, y: i32| match mask {
            Some(m) => m.get(x, y),
            None => {
                in_frame(&img, i64::from(x), i64::from(y)) && pixel(&img, at(&img, x, y)).1 != 0
            }
        };
        let dirs = neighbors(diagonals);
        let ramps = self.ramps();
        let colour_for = |source: Option<u8>| -> Option<u8> {
            match color {
                OutlineColor::Fixed(i) => Some(i),
                OutlineColor::AutoDarker(n) => source.map(|s| ramps.shift(s, n.saturating_neg())),
            }
        };
        let source_of = |x: i32, y: i32| -> Option<u8> {
            let (idx, alpha) = pixel(&img, at(&img, x, y));
            (alpha != 0).then_some(idx)
        };

        let mut out = img.clone();
        // ring[i] = Some(source index) for pixels in a ring.
        let mut ring: Vec<Option<Option<u8>>> = vec![None; (w as usize) * (h as usize)];
        match side {
            OutlineSide::Outside => {
                let mut frontier: Vec<(i32, i32)> = Vec::new();
                let mut src: Vec<Option<u8>> = vec![None; ring.len()];
                for y in 0..h {
                    for x in 0..w {
                        if in_shape(x, y) {
                            src[at(&img, x, y)] = source_of(x, y);
                            frontier.push((x, y));
                        }
                    }
                }
                for _ in 0..thickness {
                    let mut next = Vec::new();
                    for &(x, y) in &frontier {
                        for &(ox, oy) in dirs {
                            let (nx, ny) = (x + ox, y + oy);
                            if !in_frame(&img, i64::from(nx), i64::from(ny)) || in_shape(nx, ny) {
                                continue;
                            }
                            let j = at(&img, nx, ny);
                            if ring[j].is_none() {
                                ring[j] = Some(src[at(&img, x, y)]);
                                src[j] = src[at(&img, x, y)];
                                next.push((nx, ny));
                            }
                        }
                    }
                    next.sort_by_key(|&(x, y)| (y, x));
                    frontier = next;
                }
            }
            OutlineSide::Inside => {
                let mut frontier: Vec<(i32, i32)> = Vec::new();
                for y in 0..h {
                    for x in 0..w {
                        if in_shape(x, y) && dirs.iter().any(|&(ox, oy)| !in_shape(x + ox, y + oy))
                        {
                            ring[at(&img, x, y)] = Some(source_of(x, y));
                            frontier.push((x, y));
                        }
                    }
                }
                for _ in 1..thickness {
                    let mut next = Vec::new();
                    for &(x, y) in &frontier {
                        for &(ox, oy) in dirs {
                            let (nx, ny) = (x + ox, y + oy);
                            if in_frame(&img, i64::from(nx), i64::from(ny))
                                && in_shape(nx, ny)
                                && ring[at(&img, nx, ny)].is_none()
                            {
                                ring[at(&img, nx, ny)] = Some(source_of(nx, ny));
                                next.push((nx, ny));
                            }
                        }
                    }
                    frontier = next;
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                let i = at(&img, x, y);
                if let Some(Some(c)) = ring[i].map(colour_for) {
                    put(&mut out, i, (c, 255));
                }
            }
        }
        canonicalize(&mut out);
        self.step("outline", |s| s.commit_image(f, out))
    }

    /// Remove isolated opaque pixels (no opaque 8-neighbour) and fill
    /// single-pixel transparent holes (all four neighbours opaque) with the
    /// most common neighbouring colour (ties: N, W, E, S order). Decisions are
    /// made on the original frame, so one pass does not cascade.
    pub fn clean_strays(&mut self, f: FrameRef, mask: Option<&Mask>) -> Result<()> {
        let src = self.img(f)?.clone();
        let mut out = src.clone();
        let opaque = |img: &IndexedImage, x: i32, y: i32| {
            in_frame(img, i64::from(x), i64::from(y)) && pixel(img, at(img, x, y)).1 != 0
        };
        for y in 0..src.height as i32 {
            for x in 0..src.width as i32 {
                if !selected(mask, x, y) {
                    continue;
                }
                let i = at(&src, x, y);
                if opaque(&src, x, y) {
                    if !NEIGHBORS
                        .iter()
                        .any(|&(ox, oy)| opaque(&src, x + ox, y + oy))
                    {
                        put(&mut out, i, (0, 0));
                    }
                } else if NEIGHBORS[..4]
                    .iter()
                    .all(|&(ox, oy)| opaque(&src, x + ox, y + oy))
                {
                    let around: Vec<u8> = NEIGHBORS[..4]
                        .iter()
                        .map(|&(ox, oy)| pixel(&src, at(&src, x + ox, y + oy)).0)
                        .collect();
                    let best = around
                        .iter()
                        .copied()
                        .max_by_key(|&c| {
                            let n = around.iter().filter(|&&o| o == c).count();
                            (
                                n,
                                usize::MAX - around.iter().position(|&o| o == c).unwrap_or(0),
                            )
                        })
                        .unwrap_or(1);
                    put(&mut out, i, (best, 255));
                }
            }
        }
        canonicalize(&mut out);
        self.step("clean strays", |s| s.commit_image(f, out))
    }
}
