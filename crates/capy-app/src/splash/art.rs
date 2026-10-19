//! The loading screen's pictures, ported from the user's earlier "MXL
//! Capybara Editor" (its `mxl_splash` module, Python 3.10 bytecode): Diablo
//! II's own door-opening animation with the robed figure painted out, the old
//! "LOADING..." text cleared, and the capybara of the app icon standing in
//! the door, lit by the door's light.
//!
//! Every step keeps the old program's arithmetic (same constants, same order
//! of float operations, Python's `int()` truncation when a channel is
//! stored), so the frames match it up to the icon's Lanczos resize.

use crate::branding::RgbaImage;

/// Edge of the door animation's frames.
pub const SIZE: usize = 256;
/// The robed figure that is painted out: (x0, y0, x1, y1), exclusive ends.
pub const FIGURE: (usize, usize, usize, usize) = (78, 110, 112, 190);
/// Width of the soft edge between the painted region and the original.
pub const FEATHER: f64 = 5.0;
/// Where the capybara stands: centred in x, feet on y1.
pub const CAPY_BOX: (usize, usize, usize, usize) = (70, 146, 122, 188);
/// The old "LOADING..." text on the plate.
pub const TEXT_BOX: (usize, usize, usize, usize) = (90, 235, 170, 250);
/// The plate row copied over the text (alternating with the row above it).
pub const PLATE_ROW: usize = 234;
/// How much of the stone texture beside the door is laid over the fill.
pub const TEXTURE: f64 = 0.15;
/// Relaxation sweeps of the fill (the old program's constant).
const SWEEPS: usize = 260;
/// Over-relaxation factor of the fill.
const OMEGA: f64 = 1.9;
/// The title's text, font, colour and centre row.
pub const TITLE: &str = "Capybara's D2 Editor";
pub const TITLE_FONT: &str = "fontexocet10";
pub const TITLE_GOLD: [u8; 3] = [240, 200, 90];
pub const TITLE_Y: usize = 243;
/// Milliseconds per animation frame.
pub const FRAME_MS: u64 = 85;

/// `_px`: a pixel's RGB as Python ints (as f64 for the arithmetic).
fn px(img: &RgbaImage, x: usize, y: usize) -> [f64; 3] {
    let i = (y * img.width + x) * 4;
    match img.data.get(i..i + 3) {
        Some(p) => [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
        None => [0.0; 3],
    }
}

/// `_put`: store `max(0, min(255, int(v)))` per channel, alpha 255.
/// (`as u8` truncates toward zero and saturates, which is exactly that.)
fn put(img: &mut RgbaImage, x: usize, y: usize, rgb: [f64; 3]) {
    if x >= img.width {
        return;
    }
    let i = (y * img.width + x) * 4;
    if let Some(p) = img.data.get_mut(i..i + 4) {
        p.copy_from_slice(&[rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255]);
    }
}

/// `_lum`: Rec. 601 luma.
pub fn lum(rgb: [f64; 3]) -> f64 {
    0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]
}

/// `_inpaint_figure`: replace the figure box by a smooth fill (a harmonic
/// fill of each channel from the box's one-pixel border, started from a
/// left-right blend and relaxed by in-place over-relaxation), add 15% of the
/// texture of the wall `W + 4` px to the left (minus its row mean), and
/// feather the result into the original over `FEATHER + 1` px.
pub fn inpaint_figure(img: &mut RgbaImage) {
    let (x0, y0, x1, y1) = FIGURE;
    let (w, h) = (x1 - x0, y1 - y0);
    let shift = w + 4;
    // ch[c][y + 1][x + 1] for x in -1..=W, y in -1..=H
    let mut ch: Vec<Vec<Vec<f64>>> = Vec::with_capacity(3);
    for c in 0..3 {
        let mut g: Vec<Vec<f64>> = (0..h + 2)
            .map(|gy| {
                (0..w + 2)
                    .map(|gx| px(img, x0 + gx - 1, y0 + gy - 1)[c])
                    .collect()
            })
            .collect();
        for row in g.iter_mut().take(h + 1).skip(1) {
            let (left, right) = (row[0], row[w + 1]);
            for (x, v) in row.iter_mut().enumerate().take(w + 1).skip(1) {
                let t = x as f64 / (w + 1) as f64;
                *v = left * (1.0 - t) + right * t;
            }
        }
        for _ in 0..SWEEPS {
            for y in 1..=h {
                for x in 1..=w {
                    let v = (g[y][x - 1] + g[y][x + 1] + g[y - 1][x] + g[y + 1][x]) * 0.25;
                    g[y][x] += (v - g[y][x]) * OMEGA;
                }
            }
        }
        ch.push(g);
    }
    for y in 0..h {
        let src: Vec<[f64; 3]> = (0..w).map(|x| px(img, x0 + x - shift, y0 + y)).collect();
        let mean: [f64; 3] =
            std::array::from_fn(|i| src.iter().map(|p| p[i]).sum::<f64>() / w as f64);
        for x in 0..w {
            let d = (x + 1).min(y + 1).min(w - x).min(h - y);
            let k = (d as f64 / (FEATHER + 1.0)).min(1.0);
            let orig = px(img, x0 + x, y0 + y);
            let fill: [f64; 3] =
                std::array::from_fn(|i| ch[i][y + 1][x + 1] + (src[x][i] - mean[i]) * TEXTURE);
            let out: [f64; 3] = std::array::from_fn(|i| fill[i] * k + orig[i] * (1.0 - k));
            put(img, x0 + x, y0 + y, out);
        }
    }
}

/// `_clear_text`: cover the old text with the plate's own rows 234 and 233,
/// alternating, so the plate keeps its grain.
pub fn clear_text(img: &mut RgbaImage) {
    let (x0, y0, x1, y1) = TEXT_BOX;
    for y in y0..y1 {
        let from = PLATE_ROW - (y - y0) % 2;
        for x in x0..x1 {
            let rgb = px(img, x, from);
            put(img, x, y, rgb);
        }
    }
}

/// The door's light in a frame: the mean colour of a 3 px grid over the
/// (already painted) figure region, slightly to the right of it.
pub fn door_light(img: &RgbaImage) -> [f64; 3] {
    let (x0, y0, x1, y1) = FIGURE;
    let samples: Vec<[f64; 3]> = (y0..y1)
        .step_by(3)
        .flat_map(|y| (x0 + 4..x1 + 10).step_by(3).map(move |x| (x, y)))
        .map(|(x, y)| px(img, x, y))
        .collect();
    let n = samples.len().max(1) as f64;
    std::array::from_fn(|k| samples.iter().map(|p| p[k]).sum::<f64>() / n)
}

/// How far the door has opened past the first frame: 0 at frame 0, 1 at a
/// luma of 200 or more.
pub fn glow(lum: f64, base: f64) -> f64 {
    ((lum - base) / (200.0 - base).max(1.0)).clamp(0.0, 1.0)
}

/// `_capybara`: the icon cropped to its opaque part (alpha > 40, scanning
/// every second row/column), scaled with Lanczos to fit `CAPY_BOX` and
/// mirrored so it faces the other way. `None` for an empty icon.
pub fn prepare_capybara(icon: &RgbaImage) -> Option<RgbaImage> {
    let (w, h) = (icon.width, icon.height);
    let alpha = |x: usize, y: usize| icon.data.get((y * w + x) * 4 + 3).copied().unwrap_or(0);
    let xs: Vec<usize> = (0..w)
        .filter(|&x| (0..h).step_by(2).any(|y| alpha(x, y) > 40))
        .collect();
    let ys: Vec<usize> = (0..h)
        .filter(|&y| (0..w).step_by(2).any(|x| alpha(x, y) > 40))
        .collect();
    let (&cx0, &cx1, &cy0, &cy1) = (xs.first()?, xs.last()?, ys.first()?, ys.last()?);
    let (cw, chh) = (cx1 + 1 - cx0, cy1 + 1 - cy0);
    let (bw, bh) = (CAPY_BOX.2 - CAPY_BOX.0, CAPY_BOX.3 - CAPY_BOX.1);
    let k = (bw as f64 / cw as f64).min(bh as f64 / chh as f64);
    // Python's round(): halves to even.
    let tw = ((cw as f64 * k).round_ties_even() as usize).max(1);
    let th = ((chh as f64 * k).round_ties_even() as usize).max(1);

    // PIL resizes RGBA premultiplied; do the same in f32.
    let mut pre = image::Rgba32FImage::new(cw as u32, chh as u32);
    for (x, y, p) in pre.enumerate_pixels_mut() {
        let i = ((y as usize + cy0) * w + x as usize + cx0) * 4;
        if let Some(s) = icon.data.get(i..i + 4) {
            // normalised to 0..1: the f32 resize clamps to that range
            let a = f32::from(s[3]) / 255.0;
            *p = image::Rgba([
                f32::from(s[0]) / 255.0 * a,
                f32::from(s[1]) / 255.0 * a,
                f32::from(s[2]) / 255.0 * a,
                a,
            ]);
        }
    }
    let small = image::imageops::resize(
        &pre,
        tw as u32,
        th as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let mut data = vec![0u8; tw * th * 4];
    for (x, y, p) in small.enumerate_pixels() {
        let a = p[3].clamp(0.0, 1.0);
        let un = |v: f32| {
            if a > 0.0 {
                (v / a * 255.0).clamp(0.0, 255.0).round() as u8
            } else {
                0
            }
        };
        // mirrored: column x lands at tw - 1 - x
        let j = (y as usize * tw + (tw - 1 - x as usize)) * 4;
        data[j..j + 4].copy_from_slice(&[un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round() as u8]);
    }
    Some(RgbaImage {
        width: tw,
        height: th,
        data,
    })
}

/// `_place_capybara`: a soft elliptic contact shadow under the feet, then
/// the capybara shaded by the door's light: ambient by the light's luma, a
/// left-to-right falloff, a rim light on its right edge growing with the
/// glow, and a glow tint. Partly transparent pixels blend over the frame.
pub fn place_capybara(img: &mut RgbaImage, cap: &RgbaImage, light: [f64; 3], glow: f64) {
    let (tw, th) = (cap.width, cap.height);
    let (bx0, _by0, bx1, by1) = CAPY_BOX;
    // Python: bx0 + (bx1 - bx0 - tw) // 2 (floor division, may be negative)
    let ox = bx0 as i64 + (bx1 as i64 - bx0 as i64 - tw as i64).div_euclid(2);
    let oy = by1 as i64 - th as i64;
    let w = img.width as i64;
    let amb = (lum(light) / 140.0).clamp(0.35, 1.0);

    let (cx, cy, rx, ry) = (
        ox as f64 + tw as f64 / 2.0,
        by1 as f64 - 1.5,
        tw as f64 * 0.48,
        3.5,
    );
    for yy in (cy - ry) as i64..=(cy + ry) as i64 {
        for xx in (cx - rx) as i64..=(cx + rx) as i64 {
            let q = ((xx as f64 - cx) / rx).powi(2) + ((yy as f64 - cy) / ry).powi(2);
            if q < 1.0 && (0..w).contains(&xx) && (0..SIZE as i64).contains(&yy) {
                let f = 1.0 - (1.0 - q) * (0.35 + 0.35 * glow);
                let (x, y) = (xx as usize, yy as usize);
                let p = px(img, x, y);
                put(img, x, y, [p[0] * f, p[1] * f, p[2] * f]);
            }
        }
    }

    let alpha = |x: usize, y: usize| cap.data.get((y * tw + x) * 4 + 3).copied().unwrap_or(0);
    for y in 0..th {
        for x in 0..tw {
            let i = (y * tw + x) * 4;
            let a = alpha(x, y);
            if a < 30 {
                continue;
            }
            let rgb = [
                f64::from(cap.data[i]),
                f64::from(cap.data[i + 1]),
                f64::from(cap.data[i + 2]),
            ];
            // distance (1..4) to the transparent right-hand edge
            let mut edge = 0.0;
            for d in 1..5 {
                let xx = x + d;
                if xx >= tw || alpha(xx, y) < 30 {
                    edge = (5 - d) as f64;
                    break;
                }
            }
            let rim = edge / 4.0 * (0.3 + 0.7 * glow);
            let side = 0.55 + 0.45 * (x as f64 / (tw.saturating_sub(1).max(1)) as f64);
            let mut col: [f64; 3] = std::array::from_fn(|k| {
                let (c, l) = (rgb[k], light[k]);
                let mut v = c * amb * side * (0.55 + 0.45 * l / 255.0);
                v += (l * 1.15 - v) * rim * 0.75;
                v += (l - v) * glow * 0.18;
                v
            });
            let (xx, yy) = (ox + x as i64, oy + y as i64);
            if !(0..w).contains(&xx) || !(0..SIZE as i64).contains(&yy) {
                continue;
            }
            let (xx, yy) = (xx as usize, yy as usize);
            if a < 255 {
                let bg = px(img, xx, yy);
                let f = f64::from(a) / 255.0;
                col = std::array::from_fn(|k| col[k] * f + bg[k] * (1.0 - f));
            }
            put(img, xx, yy, col);
        }
    }
}

/// A door frame as a `SIZE` x `SIZE` image: palette colours where the DC6
/// frame has pixels, black elsewhere (the old program drew on black).
pub fn door_frame(
    frame: &d2_formats::dc6::Dc6Frame,
    palette: &d2_formats::palette::Palette,
) -> Result<RgbaImage, String> {
    let indexed = frame.render().map_err(|e| format!("door frame: {e}"))?;
    let (fw, fh) = (indexed.width as usize, indexed.height as usize);
    let mut img = RgbaImage {
        width: SIZE,
        height: SIZE,
        data: vec![0u8; SIZE * SIZE * 4],
    };
    for y in 0..SIZE.min(fh) {
        for x in 0..SIZE.min(fw) {
            let s = y * fw + x;
            let j = (y * SIZE + x) * 4;
            let rgb = if indexed.alpha.get(s).copied().unwrap_or(0) != 0 {
                let index = indexed.pixels.get(s).copied().unwrap_or(0);
                palette.colors[usize::from(index)]
            } else {
                [0, 0, 0]
            };
            img.data[j..j + 3].copy_from_slice(&rgb);
        }
    }
    for p in img.data.chunks_exact_mut(4) {
        p[3] = 255; // drawn on black: everything is opaque
    }
    Ok(img)
}

/// `build_frames`: paint out the figure and the text of every frame (in
/// parallel), then measure each frame's door light against frame 0 and put
/// the lit capybara in.
pub fn compose_frames(mut frames: Vec<RgbaImage>, cap: &RgbaImage) -> Vec<RgbaImage> {
    std::thread::scope(|scope| {
        for frame in frames.iter_mut() {
            scope.spawn(move || {
                inpaint_figure(frame);
                clear_text(frame);
            });
        }
    });
    let mut base: Option<f64> = None;
    for frame in &mut frames {
        let light = door_light(frame);
        let l = lum(light);
        let base_l = *base.get_or_insert(l);
        place_capybara(frame, cap, light, glow(l, base_l));
    }
    frames
}

/// Where the title goes in a view scaled by `n`: the old program centred it
/// on (W/2, TITLE_Y * scale) with Tk's integer `x - width/2`, at its own
/// pixel size on a 2x splash. Here it is scaled by `max(1, n / 2)`, which
/// keeps that proportion at every window size. Returns (left, top, scale).
pub fn title_place(n: usize, title_w: usize, title_h: usize) -> (i64, i64, usize) {
    let ts = (n / 2).max(1);
    let cx = (SIZE * n / 2) as i64;
    let cy = (TITLE_Y * n) as i64;
    (
        cx - (title_w * ts / 2) as i64,
        cy - (title_h * ts / 2) as i64,
        ts,
    )
}

/// What the screen shows: `frame` scaled by `n` (nearest), the title
/// blended on top. The script hook `splash-save` writes this.
pub fn compose_view(frame: &RgbaImage, title: Option<&RgbaImage>, n: usize) -> RgbaImage {
    let n = n.max(1);
    let edge = frame.width * n;
    let mut out = RgbaImage {
        width: edge,
        height: frame.height * n,
        data: vec![0u8; edge * frame.height * n * 4],
    };
    for y in 0..out.height {
        for x in 0..out.width {
            let s = ((y / n) * frame.width + x / n) * 4;
            let d = (y * out.width + x) * 4;
            out.data[d..d + 4].copy_from_slice(&frame.data[s..s + 4]);
        }
    }
    if let Some(t) = title {
        let (left, top, ts) = title_place(n, t.width, t.height);
        for y in 0..t.height * ts {
            for x in 0..t.width * ts {
                let (ox, oy) = (left + x as i64, top + y as i64);
                if ox < 0 || oy < 0 || ox as usize >= out.width || oy as usize >= out.height {
                    continue;
                }
                let s = ((y / ts) * t.width + x / ts) * 4;
                let d = (oy as usize * out.width + ox as usize) * 4;
                let a = u32::from(t.data[s + 3]);
                for k in 0..3 {
                    let v = (u32::from(t.data[s + k]) * a
                        + u32::from(out.data[d + k]) * (255 - a)
                        + 127)
                        / 255;
                    out.data[d + k] = v as u8;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgb: [u8; 3]) -> RgbaImage {
        let mut data = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        RgbaImage {
            width: w,
            height: h,
            data,
        }
    }

    fn rgb_at(img: &RgbaImage, x: usize, y: usize) -> [u8; 3] {
        let i = (y * img.width + x) * 4;
        [img.data[i], img.data[i + 1], img.data[i + 2]]
    }

    #[test]
    fn put_truncates_and_clamps_like_python_int() {
        let mut img = solid(2, 1, [0, 0, 0]);
        put(&mut img, 0, 0, [12.9, -3.7, 300.0]);
        assert_eq!(rgb_at(&img, 0, 0), [12, 0, 255]);
        put(&mut img, 5, 0, [1.0; 3]); // off the image: ignored
        assert_eq!(img.data.len(), 8);
    }

    #[test]
    fn luma_is_rec601() {
        assert!((lum([255.0, 255.0, 255.0]) - 255.0).abs() < 1e-9);
        assert!((lum([100.0, 0.0, 0.0]) - 29.9).abs() < 1e-9);
    }

    #[test]
    fn inpaint_of_a_flat_wall_is_flat() {
        // A uniform frame stays uniform: the fill equals its border and the
        // texture term (wall minus its own mean) is zero.
        let mut img = solid(SIZE, SIZE, [80, 60, 40]);
        // a "figure" to remove
        for y in FIGURE.1 + 10..FIGURE.3 - 10 {
            for x in FIGURE.0 + 8..FIGURE.2 - 8 {
                put(&mut img, x, y, [250.0, 10.0, 200.0]);
            }
        }
        inpaint_figure(&mut img);
        for y in FIGURE.1..FIGURE.3 {
            for x in FIGURE.0..FIGURE.2 {
                let p = rgb_at(&img, x, y);
                for (c, want) in p.iter().zip([80u8, 60, 40]) {
                    assert!(c.abs_diff(want) <= 1, "{x},{y}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn inpaint_interpolates_between_the_sides_and_feathers_the_edge() {
        // Left of the box dark, right bright: the fill is a left-to-right
        // ramp; the outermost ring keeps 5/6 of the original.
        let mut img = solid(SIZE, SIZE, [0, 0, 0]);
        for y in 0..SIZE {
            for x in FIGURE.2..SIZE {
                put(&mut img, x, y, [240.0, 240.0, 240.0]);
            }
        }
        for y in FIGURE.1..FIGURE.3 {
            for x in FIGURE.0..FIGURE.2 {
                put(&mut img, x, y, [100.0, 0.0, 0.0]);
            }
        }
        inpaint_figure(&mut img);
        let mid_y = (FIGURE.1 + FIGURE.3) / 2;
        let row: Vec<u8> = (FIGURE.0 + 6..FIGURE.2 - 6)
            .map(|x| rgb_at(&img, x, mid_y)[1])
            .collect();
        assert!(row.windows(2).all(|w| w[0] <= w[1]), "{row:?}");
        assert!(row[0] < 60 && *row.last().expect("row") > 180, "{row:?}");
        // corner pixel: d = 1, k = 1/6: mostly the original red
        let corner = rgb_at(&img, FIGURE.0, FIGURE.1);
        assert!(corner[0] >= 80, "{corner:?}");
    }

    #[test]
    fn clear_text_copies_the_plate_rows_alternating() {
        let mut img = solid(SIZE, SIZE, [9, 9, 9]);
        for x in 0..SIZE {
            put(&mut img, x, PLATE_ROW, [200.0, 100.0, 50.0]);
            put(&mut img, x, PLATE_ROW - 1, [20.0, 10.0, 5.0]);
        }
        clear_text(&mut img);
        let (x0, y0, x1, y1) = TEXT_BOX;
        for y in y0..y1 {
            let want = if (y - y0) % 2 == 0 {
                [200, 100, 50]
            } else {
                [20, 10, 5]
            };
            assert_eq!(rgb_at(&img, x0, y), want);
            assert_eq!(rgb_at(&img, x1 - 1, y), want);
        }
        assert_eq!(rgb_at(&img, x1, y0), [9, 9, 9]);
    }

    #[test]
    fn light_and_glow() {
        let img = solid(SIZE, SIZE, [100, 50, 0]);
        let l = door_light(&img);
        assert_eq!(l, [100.0, 50.0, 0.0]);
        assert_eq!(glow(50.0, 50.0), 0.0);
        assert_eq!(glow(200.0, 50.0), 1.0);
        assert!((glow(125.0, 50.0) - 0.5).abs() < 1e-12);
        assert_eq!(glow(10.0, 50.0), 0.0);
        // base above 199: the divisor never drops below 1
        assert_eq!(glow(250.0, 210.0), 1.0);
    }

    fn blob_icon() -> RgbaImage {
        // 64x48 icon: an opaque 40x20 block at (10, 20), white, the rest clear
        let (w, h) = (64, 48);
        let mut data = vec![0u8; w * h * 4];
        for y in 20..40 {
            for x in 10..50 {
                let i = (y * w + x) * 4;
                data[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        RgbaImage {
            width: w,
            height: h,
            data,
        }
    }

    #[test]
    fn capybara_is_cropped_scaled_into_the_box_and_mirrored() {
        let cap = prepare_capybara(&blob_icon()).expect("not empty");
        // 40x20 cropped, k = min(52/40, 42/20) = 1.3 -> 52x26
        assert_eq!((cap.width, cap.height), (52, 26));
        let mut icon = blob_icon();
        // mark the left column red: after mirroring it is on the right
        for y in 20..40 {
            let i = (y * 64 + 10) * 4;
            icon.data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        let cap = prepare_capybara(&icon).expect("not empty");
        let right = rgb_at(&cap, cap.width - 1, cap.height / 2);
        let left = rgb_at(&cap, 0, cap.height / 2);
        assert!(right[1] < 200 && left[1] > 230, "{left:?} {right:?}");
        let empty = RgbaImage {
            width: 4,
            height: 4,
            data: vec![0; 64],
        };
        assert!(prepare_capybara(&empty).is_none());
    }

    #[test]
    fn capybara_is_lit_by_the_door_and_stands_on_the_box_floor() {
        let cap = RgbaImage {
            width: 10,
            height: 8,
            data: [200u8, 200, 200, 255].repeat(80),
        };
        let mut dark = solid(SIZE, SIZE, [0, 0, 0]);
        place_capybara(&mut dark, &cap, [20.0, 20.0, 20.0], 0.0);
        let mut bright = solid(SIZE, SIZE, [0, 0, 0]);
        place_capybara(&mut bright, &cap, [230.0, 200.0, 120.0], 1.0);
        // ox = 70 + (52 - 10) // 2 = 91; oy = 188 - 8 = 180
        let (ox, oy) = (91, 180);
        assert_eq!(rgb_at(&dark, ox - 1, oy), [0, 0, 0]);
        let d = rgb_at(&dark, ox + 2, oy + 4);
        let b = rgb_at(&bright, ox + 2, oy + 4);
        assert!(d.iter().all(|&c| c > 0), "{d:?}");
        assert!(b[0] > d[0] && b[0] > b[2], "{d:?} {b:?}");
        // left-to-right falloff: the right side is brighter (before the rim)
        let l = rgb_at(&dark, ox, oy + 4);
        let r = rgb_at(&dark, ox + 5, oy + 4);
        assert!(r[0] > l[0], "{l:?} {r:?}");
        // the rim: the last column is pulled toward the light
        let rim = rgb_at(&bright, ox + 9, oy + 4);
        let inner = rgb_at(&bright, ox + 4, oy + 4);
        assert!(rim[0] > inner[0], "{inner:?} {rim:?}");
        // the contact shadow darkens the floor under the feet
        let mut floor = solid(SIZE, SIZE, [100, 100, 100]);
        place_capybara(&mut floor, &cap, [20.0, 20.0, 20.0], 0.0);
        assert!(rgb_at(&floor, 96, 188)[0] < 100);
        assert_eq!(rgb_at(&floor, 60, 187), [100, 100, 100]);
    }

    #[test]
    fn half_transparent_capybara_pixels_blend_over_the_frame() {
        let cap = RgbaImage {
            width: 1,
            height: 1,
            data: vec![0, 0, 0, 128],
        };
        let mut img = solid(SIZE, SIZE, [200, 200, 200]);
        place_capybara(&mut img, &cap, [0.0; 3], 0.0);
        // ox = 70 + 51 // 2 = 95, oy = 187; black at 50% over the shadowed floor
        let p = rgb_at(&img, 95, 187);
        assert!(p[0] > 40 && p[0] < 110, "{p:?}");
    }

    #[test]
    fn view_scales_the_frame_and_centres_the_title() {
        let frame = solid(SIZE, SIZE, [10, 20, 30]);
        let title = RgbaImage {
            width: 20,
            height: 6,
            data: [240u8, 200, 90, 255].repeat(120),
        };
        assert_eq!(title_place(2, 20, 6), (256 - 10, 486 - 3, 1));
        assert_eq!(title_place(4, 20, 6), (512 - 20, 972 - 6, 2));
        assert_eq!(title_place(1, 20, 6), (128 - 10, 243 - 3, 1));
        let v = compose_view(&frame, Some(&title), 2);
        assert_eq!((v.width, v.height), (512, 512));
        assert_eq!(rgb_at(&v, 0, 0), [10, 20, 30]);
        assert_eq!(rgb_at(&v, 256, 486), [240, 200, 90]);
        assert_eq!(rgb_at(&v, 245, 486), [10, 20, 30]);
        assert_eq!(rgb_at(&v, 246, 483), [240, 200, 90]);
    }

    #[test]
    fn compose_runs_every_step_on_every_frame() {
        let frames = vec![
            solid(SIZE, SIZE, [30, 30, 30]),
            solid(SIZE, SIZE, [120, 110, 90]),
        ];
        let cap = RgbaImage {
            width: 10,
            height: 8,
            data: [200u8, 150, 100, 255].repeat(80),
        };
        let out = compose_frames(frames, &cap);
        assert_eq!(out.len(), 2);
        // the capybara is in both, brighter in the brighter frame
        let a = rgb_at(&out[0], 95, 184);
        let b = rgb_at(&out[1], 95, 184);
        assert!(a != [30, 30, 30] && b[0] > a[0], "{a:?} {b:?}");
    }
}
