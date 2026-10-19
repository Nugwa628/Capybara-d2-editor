#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

/// A deterministic noisy image with a transparent border.
fn noisy(width: u32, height: u32, seed: u32) -> RgbaImage {
    let mut img = RgbaImage::filled(width, height, [0, 0, 0, 0]);
    let mut state = seed;
    for y in 4..height - 4 {
        for x in 4..width - 4 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = (state >> 24) as u8;
            let i = (y as usize * width as usize + x as usize) * 4;
            img.pixels[i..i + 4].copy_from_slice(&[v, v / 2, 255 - v, 255]);
        }
    }
    img
}

fn shifted(img: &RgbaImage, dx: i32, dy: i32) -> RgbaImage {
    // out(x + dx, y + dy) = img(x, y)
    let mut out = RgbaImage::filled(img.width, img.height, [0, 0, 0, 0]);
    for y in 0..img.height {
        for x in 0..img.width {
            let (nx, ny) = (i64::from(x) + i64::from(dx), i64::from(y) + i64::from(dy));
            if let (Ok(nx), Ok(ny)) = (u32::try_from(nx), u32::try_from(ny))
                && let Some(p) = img.get(x, y)
            {
                let i = (ny as usize * img.width as usize + nx as usize) * 4;
                if i + 4 <= out.pixels.len() && nx < img.width {
                    out.pixels[i..i + 4].copy_from_slice(&p);
                }
            }
        }
    }
    out
}

#[test]
fn identical_images_match_perfectly() {
    let a = noisy(40, 30, 1);
    let d = compare_images(&a, &a, 0, 3);
    assert_eq!(d.best_shift, (0, 0));
    assert_eq!((d.width, d.height), (40, 30));
    assert!((d.within_tolerance - 1.0).abs() < 1e-12);
    assert!(d.mean_abs.abs() < 1e-12);
}

#[test]
fn the_shift_search_finds_an_offset_canvas() {
    let a = noisy(48, 36, 7);
    // The oracle holds our pixel (x, y) at (x + 3, y - 2).
    let b = shifted(&a, 3, -2);
    let d = compare_images(&a, &b, 0, 5);
    assert_eq!(d.best_shift, (3, -2));
    assert!(d.within_tolerance > 0.95, "{}", d.within_tolerance);
}

#[test]
fn tolerance_is_a_max_channel_difference() {
    let a = noisy(30, 30, 3);
    let mut b = a.clone();
    for p in b.pixels.as_chunks_mut::<4>().0 {
        if p[3] != 0 {
            p[0] = p[0].saturating_add(10);
        }
    }
    let strict = compare_images(&a, &b, 8, 0);
    let loose = compare_images(&a, &b, 16, 0);
    assert!(strict.within_tolerance < 0.2, "{}", strict.within_tolerance);
    assert!(loose.within_tolerance > 0.9, "{}", loose.within_tolerance);
    assert!(loose.mean_abs > 0.0);
}

#[test]
fn background_in_both_images_is_excluded() {
    let a = RgbaImage::filled(10, 10, [0, 0, 0, 0]);
    let d = compare_images(&a, &a, 0, 0);
    assert_eq!(d.within_tolerance, 0.0);
    let r = compare_images_with(
        &a,
        &a,
        &CompareOptions {
            tolerance: 0,
            max_shift: 0,
            coverage: Coverage::Either,
            mask: None,
        },
    );
    assert_eq!(r.compared, 0);
}

#[test]
fn coverage_rules_treat_one_sided_pixels_differently() {
    let a = noisy(20, 20, 5);
    let mut b = a.clone();
    // Blank a 4x4 patch of the oracle.
    for y in 6..10 {
        for x in 6..10 {
            let i = (y * 20 + x) * 4;
            b.pixels[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
    }
    let both = compare_images_with(
        &a,
        &b,
        &CompareOptions {
            tolerance: 0,
            max_shift: 0,
            coverage: Coverage::Both,
            mask: None,
        },
    );
    assert_eq!(both.only_ours, 16);
    assert!((both.diff.within_tolerance - 1.0).abs() < 1e-12);
    let either = compare_images_with(
        &a,
        &b,
        &CompareOptions {
            tolerance: 0,
            max_shift: 0,
            coverage: Coverage::Either,
            mask: None,
        },
    );
    assert!(either.diff.within_tolerance < 1.0);
    assert_eq!(either.compared, both.compared + 16);
}

#[test]
fn the_mask_excludes_pixels() {
    let a = noisy(20, 20, 9);
    let mut b = a.clone();
    let mut mask = vec![false; 400];
    for y in 8..12 {
        for x in 8..12 {
            let i = (y * 20 + x) * 4;
            b.pixels[i] = b.pixels[i].wrapping_add(100);
            mask[y * 20 + x] = true;
        }
    }
    let r = compare_images_with(
        &a,
        &b,
        &CompareOptions {
            tolerance: 0,
            max_shift: 0,
            coverage: Coverage::Both,
            mask: Some(&mask),
        },
    );
    assert_eq!(r.masked, 16);
    assert!((r.diff.within_tolerance - 1.0).abs() < 1e-12);
    let unmasked = compare_images(&a, &b, 0, 0);
    assert!(unmasked.within_tolerance < 1.0);
}
