use std::collections::BTreeMap;

use d2_formats::IndexedImage;
use proptest::prelude::*;

use crate::model::Sprite;
use crate::ops::{
    add_direction, center_offset, join_frames, offset_all, remove_directions, remove_frames,
    split_frame, trim_borders,
};
use crate::testutil::{image, small_sprite};

/// The visible pixels of an image keyed by their position relative to the
/// anchor.
fn on_screen(img: &IndexedImage) -> BTreeMap<(i32, i32), u8> {
    let mut out = BTreeMap::new();
    for y in 0..img.height {
        for x in 0..img.width {
            let i = (y * img.width + x) as usize;
            if img.alpha[i] != 0 && img.pixels[i] != 0 {
                out.insert(
                    (img.offset_x + x as i32, img.offset_y + y as i32),
                    img.pixels[i],
                );
            }
        }
    }
    out
}

fn noisy(w: u32, h: u32, seed: u8) -> IndexedImage {
    let px = (0..w * h)
        .map(|i| {
            let v = (i as u8).wrapping_mul(31).wrapping_add(seed);
            if v.is_multiple_of(3) { 0 } else { v | 1 }
        })
        .collect();
    image(w, h, -7, 12, px)
}

#[test]
fn split_cuts_row_major_with_consistent_offsets() {
    let img = noisy(10, 7, 3);
    let tiles = split_frame(&img, 4, 3);
    assert_eq!(tiles.len(), 3 * 3);
    assert_eq!((tiles[0].width, tiles[0].height), (4, 3));
    assert_eq!((tiles[2].width, tiles[2].height), (2, 3), "right edge");
    assert_eq!((tiles[8].width, tiles[8].height), (2, 1), "corner");
    assert_eq!((tiles[1].offset_x, tiles[1].offset_y), (-7 + 4, 12));
    assert_eq!((tiles[3].offset_x, tiles[3].offset_y), (-7, 12 + 3));
    // together the tiles show exactly the picture
    let mut all = BTreeMap::new();
    for t in &tiles {
        all.extend(on_screen(t));
    }
    assert_eq!(all, on_screen(&img));
}

#[test]
fn split_degenerate_cases_return_the_image() {
    let img = noisy(5, 5, 1);
    assert_eq!(split_frame(&img, 0, 3), vec![img.clone()]);
    assert_eq!(split_frame(&img, 3, 0), vec![img.clone()]);
    assert_eq!(split_frame(&img, 256, 256), vec![img.clone()]);
    let empty = IndexedImage::new(0, 0, 1, 2);
    assert_eq!(split_frame(&empty, 4, 4), vec![empty]);
}

#[test]
fn join_rejects_bad_arguments() {
    assert!(join_frames(&[], 2).is_err());
    assert!(join_frames(&[noisy(2, 2, 0)], 0).is_err());
}

#[test]
fn join_pads_uneven_tiles() {
    // 3 tiles in 2 columns: the last row is half empty
    let a = noisy(4, 3, 1);
    let b = noisy(2, 3, 2);
    let c = noisy(4, 2, 3);
    let j = join_frames(&[a.clone(), b.clone(), c.clone()], 2).unwrap();
    assert_eq!((j.width, j.height), (6, 5));
    assert_eq!((j.offset_x, j.offset_y), (a.offset_x, a.offset_y));
    // tile c sits at (0, 3)
    for y in 0..c.height {
        for x in 0..c.width {
            let (si, di) = ((y * c.width + x) as usize, ((3 + y) * 6 + x) as usize);
            assert_eq!(j.pixels[di], c.pixels[si]);
        }
    }
}

proptest! {
    #[test]
    fn split_and_join_are_inverse(
        w in 1u32..40, h in 1u32..40, tw in 1u32..20, th in 1u32..20, seed in any::<u8>(),
    ) {
        let img = noisy(w, h, seed);
        let tiles = split_frame(&img, tw, th);
        let columns = w.div_ceil(tw);
        prop_assert_eq!(tiles.len() as u32, columns * h.div_ceil(th));
        let back = join_frames(&tiles, columns).unwrap();
        prop_assert_eq!(back, img);
    }

    #[test]
    fn trim_keeps_the_on_screen_position(
        w in 1u32..30, h in 1u32..30, seed in any::<u8>(), clear in 0u8..4,
    ) {
        let mut img = noisy(w, h, seed);
        // blank out borders to give trim something to do
        for y in 0..h {
            for x in 0..w {
                let border = x < u32::from(clear) || y < u32::from(clear)
                    || x + u32::from(clear) >= w || y + u32::from(clear) >= h;
                if border {
                    let i = (y * w + x) as usize;
                    img.pixels[i] = 0;
                    img.alpha[i] = 0;
                }
            }
        }
        let trimmed = trim_borders(&img);
        let before = on_screen(&img);
        prop_assert_eq!(on_screen(&trimmed), before.clone());
        if !before.is_empty() {
            // tight: the first/last rows and columns each hold a pixel
            let t = &trimmed;
            let row_has = |y: u32| (0..t.width).any(|x| t.alpha[(y * t.width + x) as usize] != 0);
            let col_has = |x: u32| (0..t.height).any(|y| t.alpha[(y * t.width + x) as usize] != 0);
            prop_assert!(row_has(0) && row_has(t.height - 1));
            prop_assert!(col_has(0) && col_has(t.width - 1));
        }
    }
}

#[test]
fn trim_of_an_empty_image_is_one_transparent_pixel() {
    let img = IndexedImage::new(6, 4, -3, 9);
    let t = trim_borders(&img);
    assert_eq!((t.width, t.height, t.offset_x, t.offset_y), (1, 1, -3, 9));
    assert_eq!((t.pixels.clone(), t.alpha.clone()), (vec![0], vec![0]));
    let empty = IndexedImage::new(0, 0, 0, 0);
    assert_eq!(
        (trim_borders(&empty).width, trim_borders(&empty).height),
        (1, 1)
    );
}

#[test]
fn center_offset_is_bottom_centre() {
    let mut img = IndexedImage::new(10, 6, 99, 99);
    center_offset(&mut img);
    assert_eq!((img.offset_x, img.offset_y), (-5, -5));
    let mut odd = IndexedImage::new(7, 1, 0, 0);
    center_offset(&mut odd);
    assert_eq!((odd.offset_x, odd.offset_y), (-3, 0));
    let mut empty = IndexedImage::new(0, 0, 5, 5);
    center_offset(&mut empty);
    assert_eq!((empty.offset_x, empty.offset_y), (0, 0));
}

#[test]
fn sprite_editing() {
    let mut s = small_sprite(3, 4, 5);
    let before = s.clone();
    offset_all(&mut s, 10, -20);
    for (a, b) in s
        .directions
        .iter()
        .flatten()
        .zip(before.directions.iter().flatten())
    {
        assert_eq!((a.offset_x, a.offset_y), (b.offset_x + 10, b.offset_y - 20));
        assert_eq!(a.pixels, b.pixels);
    }

    let mut s = before.clone();
    remove_directions(&mut s, &[1, 1, 9]);
    assert_eq!(
        s.directions,
        vec![before.directions[0].clone(), before.directions[2].clone()]
    );

    let mut s = before.clone();
    remove_frames(&mut s, &[0, 2, 7]);
    for (dir, orig) in s.directions.iter().zip(&before.directions) {
        assert_eq!(dir, &vec![orig[1].clone(), orig[3].clone()]);
    }
    assert_eq!(s.frames_per_direction(), 2);

    let mut s = before.clone();
    assert!(add_direction(&mut s, vec![noisy(2, 2, 0)]).is_err());
    assert_eq!(s.directions.len(), 3);
    let new_dir = before.directions[0].clone();
    add_direction(&mut s, new_dir.clone()).unwrap();
    assert_eq!(s.directions.len(), 4);
    assert_eq!(s.directions[3], new_dir);

    let mut empty = Sprite::default();
    add_direction(&mut empty, vec![noisy(2, 2, 0); 5]).unwrap();
    assert_eq!(empty.frames_per_direction(), 5);
}

#[test]
fn bounds_is_the_union_with_exclusive_max() {
    let s = Sprite {
        directions: vec![vec![
            image(4, 3, -2, -5, vec![1; 12]),
            image(2, 2, 3, 0, vec![1; 4]),
            IndexedImage::new(0, 0, -100, -100),
        ]],
    };
    assert_eq!(s.bounds(), Some((-2, -5, 5, 2)));
    assert_eq!(Sprite::default().bounds(), None);
}
