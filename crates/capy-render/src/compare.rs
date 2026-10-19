//! Image comparison for the oracle test.

use crate::compose::RgbaImage;

/// Result of [`compare_images`].
#[derive(Clone, Debug, PartialEq)]
pub struct ImageDiff {
    /// Size of `ours` (the image the shift search is anchored on).
    pub width: u32,
    pub height: u32,
    /// `(dx, dy)`: our pixel `(x, y)` is compared with oracle pixel
    /// `(x + dx, y + dy)`.
    pub best_shift: (i32, i32),
    /// Fraction of compared pixels whose largest channel difference is
    /// `<= tolerance`.
    pub within_tolerance: f64,
    /// Mean absolute difference over the colour channels of compared pixels.
    pub mean_abs: f64,
}

/// What counts as a pixel to compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Compare every pixel that is opaque in at least one image; a pixel
    /// opaque in only one counts as a mismatch. Used by [`compare_images`].
    Either,
    /// Compare only pixels opaque in both images ("exclude pixels that are
    /// background in either image"); the one-sided pixels are only counted in
    /// [`CompareReport`].
    Both,
}

#[derive(Clone, Debug)]
pub struct CompareOptions<'a> {
    pub tolerance: u8,
    pub max_shift: i32,
    pub coverage: Coverage,
    /// `width * height` flags in `ours` space; `true` excludes the pixel.
    pub mask: Option<&'a [bool]>,
}

/// [`ImageDiff`] plus the pixel counts behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct CompareReport {
    pub diff: ImageDiff,
    /// Pixels that entered `within_tolerance` / `mean_abs`.
    pub compared: u64,
    /// Opaque in ours, background in the oracle (not masked).
    pub only_ours: u64,
    /// Opaque in the oracle, background in ours (not masked).
    pub only_oracle: u64,
    /// Pixels excluded by the mask.
    pub masked: u64,
}

fn is_opaque(p: [u8; 4]) -> bool {
    p[3] != 0
}

fn max_diff(a: [u8; 4], b: [u8; 4]) -> u8 {
    let d = |i: usize| a[i].abs_diff(b[i]);
    d(0).max(d(1)).max(d(2))
}

fn oracle_at(oracle: &RgbaImage, x: i64, y: i64) -> Option<[u8; 4]> {
    let (x, y) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
    oracle.get(x, y)
}

/// Compares after searching integer shifts in `-max_shift..=max_shift` (canvas
/// origins may differ). Pixels transparent in both images are excluded.
pub fn compare_images(
    ours: &RgbaImage,
    oracle: &RgbaImage,
    tolerance: u8,
    max_shift: i32,
) -> ImageDiff {
    compare_images_with(
        ours,
        oracle,
        &CompareOptions {
            tolerance,
            max_shift,
            coverage: Coverage::Either,
            mask: None,
        },
    )
    .diff
}

/// Score of one shift on a sample of the pixels: matches / opaque-in-ours.
fn shift_score(
    ours: &RgbaImage,
    oracle: &RgbaImage,
    opts: &CompareOptions<'_>,
    shift: (i32, i32),
    step: usize,
) -> (u64, u64) {
    let mut good = 0u64;
    let mut total = 0u64;
    let w = ours.width as usize;
    for y in (0..ours.height as usize).step_by(step) {
        for x in (0..w).step_by(step) {
            if let Some(mask) = opts.mask
                && mask.get(y * w + x).copied().unwrap_or(false)
            {
                continue;
            }
            let Some(a) = ours.get(x as u32, y as u32) else {
                continue;
            };
            if !is_opaque(a) {
                continue;
            }
            total += 1;
            let ox = x as i64 + i64::from(shift.0);
            let oy = y as i64 + i64::from(shift.1);
            if let Some(b) = oracle_at(oracle, ox, oy)
                && is_opaque(b)
                && max_diff(a, b) <= opts.tolerance
            {
                good += 1;
            }
        }
    }
    (good, total)
}

/// Full comparison with a mask and a coverage rule.
pub fn compare_images_with(
    ours: &RgbaImage,
    oracle: &RgbaImage,
    opts: &CompareOptions<'_>,
) -> CompareReport {
    let max_shift = opts.max_shift.max(0);
    // Pick the shift on a sample, then evaluate it on every pixel.
    let step = ((ours.width as usize * ours.height as usize) / 60_000).clamp(1, 16);
    let mut best = (0, 0);
    let mut best_score = -1.0f64;
    let mut best_cost = i32::MAX;
    for dy in -max_shift..=max_shift {
        for dx in -max_shift..=max_shift {
            let (good, total) = shift_score(ours, oracle, opts, (dx, dy), step);
            let score = if total == 0 {
                0.0
            } else {
                good as f64 / total as f64
            };
            let cost = dx.abs() + dy.abs();
            if score > best_score + 1e-12
                || ((score - best_score).abs() <= 1e-12 && cost < best_cost)
            {
                best = (dx, dy);
                best_score = score;
                best_cost = cost;
            }
        }
    }

    let w = ours.width as usize;
    let mut compared = 0u64;
    let mut within = 0u64;
    let mut sum_abs = 0u64;
    let mut only_ours = 0u64;
    let mut only_oracle = 0u64;
    let mut masked = 0u64;
    for y in 0..ours.height as usize {
        for x in 0..w {
            if let Some(mask) = opts.mask
                && mask.get(y * w + x).copied().unwrap_or(false)
            {
                masked += 1;
                continue;
            }
            let a = ours.get(x as u32, y as u32).filter(|p| is_opaque(*p));
            let b = oracle_at(
                oracle,
                x as i64 + i64::from(best.0),
                y as i64 + i64::from(best.1),
            )
            .filter(|p| is_opaque(*p));
            match (a, b) {
                (Some(a), Some(b)) => {
                    compared += 1;
                    if max_diff(a, b) <= opts.tolerance {
                        within += 1;
                    }
                    sum_abs += (0..3).map(|i| u64::from(a[i].abs_diff(b[i]))).sum::<u64>();
                }
                (Some(_), None) => {
                    only_ours += 1;
                    if opts.coverage == Coverage::Either {
                        compared += 1;
                        sum_abs += 3 * 255;
                    }
                }
                (None, Some(_)) => {
                    only_oracle += 1;
                    if opts.coverage == Coverage::Either {
                        compared += 1;
                        sum_abs += 3 * 255;
                    }
                }
                (None, None) => {}
            }
        }
    }
    // Oracle pixels outside our canvas also count as one-sided for `Either`.
    let (within_tolerance, mean_abs) = if compared == 0 {
        (0.0, 0.0)
    } else {
        (
            within as f64 / compared as f64,
            sum_abs as f64 / (compared as f64 * 3.0),
        )
    };
    CompareReport {
        diff: ImageDiff {
            width: ours.width,
            height: ours.height,
            best_shift: best,
            within_tolerance,
            mean_abs,
        },
        compared,
        only_ours,
        only_oracle,
        masked,
    }
}

#[cfg(test)]
mod tests;
