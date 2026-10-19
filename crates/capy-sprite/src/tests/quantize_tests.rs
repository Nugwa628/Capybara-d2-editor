use d2_formats::IndexedImage;
use proptest::prelude::*;

use crate::model::RgbaFrame;
use crate::quantize::{Dither, QuantizeOptions, quantize, to_rgba};
use crate::testutil::{image, synthetic_palette};

fn frame(w: u32, h: u32, px: &[[u8; 4]]) -> RgbaFrame {
    RgbaFrame {
        width: w,
        height: h,
        offset_x: -3,
        offset_y: 8,
        pixels: px.iter().flatten().copied().collect(),
    }
}

fn opts(dither: Dither) -> QuantizeOptions {
    QuantizeOptions {
        dither,
        ..QuantizeOptions::default()
    }
}

#[test]
fn defaults_follow_the_contract() {
    let o = QuantizeOptions::default();
    assert_eq!(o.dither, Dither::None);
    assert_eq!(o.transparent_rgb, Some([0, 0, 0]));
    assert_eq!(o.alpha_threshold, 128);
    assert!(o.protect_transparent);
}

#[test]
fn exact_palette_colours_map_to_themselves() {
    let pal = synthetic_palette();
    let px: Vec<[u8; 4]> = (1..=255usize)
        .map(|i| {
            let c = pal.colors[i];
            [c[0], c[1], c[2], 255]
        })
        .collect();
    for dither in [Dither::None, Dither::FloydSteinberg] {
        let out = quantize(&frame(255, 1, &px), &pal, &opts(dither));
        let want: Vec<u8> = (1..=255).collect();
        assert_eq!(out.pixels, want, "{dither:?}");
        assert!(out.alpha.iter().all(|&a| a == 255));
        assert_eq!((out.offset_x, out.offset_y), (-3, 8), "offsets are copied");
    }
}

#[test]
fn nearest_colour_wins_and_ties_go_to_the_lowest_index() {
    let mut pal = synthetic_palette();
    pal.colors[10] = [100, 100, 100];
    pal.colors[11] = [110, 110, 110];
    pal.colors[12] = [100, 100, 100];
    let f = frame(
        3,
        1,
        &[
            [101, 100, 100, 255],
            [109, 110, 110, 255],
            [100, 100, 100, 255],
        ],
    );
    let out = quantize(&f, &pal, &opts(Dither::None));
    assert_eq!(out.pixels, vec![10, 11, 10]);
}

#[test]
fn protect_transparent_never_maps_an_opaque_pixel_to_zero() {
    let pal = synthetic_palette();
    let dark = [[1u8, 1, 1, 255], [0, 0, 1, 255], [2, 0, 0, 255]];
    let f = frame(3, 1, &dark);
    let keep = QuantizeOptions {
        transparent_rgb: None,
        ..QuantizeOptions::default()
    };
    let out = quantize(&f, &pal, &keep);
    assert!(out.pixels.iter().all(|&p| p != 0), "{:?}", out.pixels);
    assert!(out.alpha.iter().all(|&a| a == 255));

    // without protection the palette's index 0 (black) is the nearest colour
    // and the pixel turns transparent
    let loose = QuantizeOptions {
        protect_transparent: false,
        transparent_rgb: None,
        ..QuantizeOptions::default()
    };
    let out = quantize(&frame(1, 1, &[[0, 0, 1, 255]]), &pal, &loose);
    assert_eq!((out.pixels[0], out.alpha[0]), (0, 0));
}

#[test]
fn alpha_and_colour_keying_both_make_pixels_transparent() {
    let pal = synthetic_palette();
    let c = pal.colors[40];
    let px = [
        [c[0], c[1], c[2], 255], // opaque colour
        [c[0], c[1], c[2], 0],   // alpha keyed
        [c[0], c[1], c[2], 127], // below the default threshold
        [c[0], c[1], c[2], 128], // at the threshold: opaque
        [0, 0, 0, 255],          // colour keyed (D2 black)
        [255, 0, 255, 255],      // other key colour, not set by default
    ];
    let f = frame(6, 1, &px);
    let out = quantize(&f, &pal, &QuantizeOptions::default());
    assert_eq!(out.alpha, vec![255, 0, 0, 255, 0, 255]);
    assert_eq!(out.pixels[1], 0);
    assert_eq!(out.pixels[4], 0);

    // alpha only: black stays opaque (and avoids index 0)
    let alpha_only = QuantizeOptions {
        transparent_rgb: None,
        ..QuantizeOptions::default()
    };
    let out = quantize(&f, &pal, &alpha_only);
    assert_eq!(out.alpha, vec![255, 0, 0, 255, 255, 255]);
    assert_ne!(out.pixels[4], 0);

    // another key colour
    let magenta = QuantizeOptions {
        transparent_rgb: Some([255, 0, 255]),
        ..QuantizeOptions::default()
    };
    let out = quantize(&f, &pal, &magenta);
    assert_eq!(out.alpha, vec![255, 0, 0, 255, 255, 0]);

    // threshold 0 keeps even fully transparent pixels opaque
    let zero = QuantizeOptions {
        alpha_threshold: 0,
        transparent_rgb: None,
        ..QuantizeOptions::default()
    };
    assert!(quantize(&f, &pal, &zero).alpha.iter().all(|&a| a == 255));
}

#[test]
fn dithering_mixes_colours_and_leaves_transparency_alone() {
    let mut pal = synthetic_palette();
    pal.colors[1] = [0, 0, 8];
    pal.colors[2] = [200, 200, 200];
    for c in pal.colors.iter_mut().skip(3) {
        *c = [250, 10, 10];
    }
    // a flat mid grey between the two nearest palette entries
    let mut px = vec![[100u8, 100, 100, 255]; 16 * 16];
    px[0] = [100, 100, 100, 0];
    let f = frame(16, 16, &px);
    let plain = quantize(&f, &pal, &opts(Dither::None));
    let dithered = quantize(&f, &pal, &opts(Dither::FloydSteinberg));
    let distinct = |img: &IndexedImage| {
        let mut v: Vec<u8> = img
            .pixels
            .iter()
            .zip(&img.alpha)
            .filter(|&(_, &a)| a != 0)
            .map(|(&p, _)| p)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    assert_eq!(distinct(&plain).len(), 1);
    assert!(distinct(&dithered).len() >= 2, "{:?}", distinct(&dithered));
    assert_eq!(dithered.alpha[0], 0);
    assert_eq!(dithered.pixels[0], 0);
}

#[test]
fn to_rgba_round_trips_through_quantize() {
    let pal = synthetic_palette();
    let px: Vec<u8> = (0..64)
        .map(|i| if i % 5 == 0 { 0 } else { 1 + i as u8 * 3 })
        .collect();
    let img = image(8, 8, -4, -7, px);
    let rgba = to_rgba(&img, &pal);
    assert_eq!(
        (rgba.width, rgba.height, rgba.offset_x, rgba.offset_y),
        (8, 8, -4, -7)
    );
    assert_eq!(
        &rgba.pixels[0..4],
        &[0, 0, 0, 0],
        "transparent is black with alpha 0"
    );
    let back = quantize(
        &rgba,
        &pal,
        &QuantizeOptions {
            transparent_rgb: None,
            ..QuantizeOptions::default()
        },
    );
    assert_eq!(back, img);
}

proptest! {
    #[test]
    fn output_is_always_valid(
        w in 1u32..12, h in 1u32..12, seed in any::<u64>(), dither in any::<bool>(),
        protect in any::<bool>(), key in any::<bool>(), thr in any::<u8>(),
    ) {
        let pal = synthetic_palette();
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 8) as u8
        };
        let pixels: Vec<u8> = (0..w * h * 4).map(|_| next()).collect();
        let f = RgbaFrame { width: w, height: h, offset_x: 0, offset_y: 0, pixels };
        let o = QuantizeOptions {
            dither: if dither { Dither::FloydSteinberg } else { Dither::None },
            transparent_rgb: key.then_some([0, 0, 0]),
            alpha_threshold: thr,
            protect_transparent: protect,
        };
        let out = quantize(&f, &pal, &o);
        prop_assert_eq!(out.pixels.len(), (w * h) as usize);
        for i in 0..(w * h) as usize {
            let (p, a) = (out.pixels[i], out.alpha[i]);
            prop_assert!(a == 0 || a == 255);
            prop_assert_eq!(a == 0, p == 0);
            let src = &f.pixels[i * 4..i * 4 + 4];
            if src[3] < thr {
                prop_assert_eq!(a, 0);
            }
            if protect && src[3] >= thr && !(key && src[0..3] == [0, 0, 0]) {
                prop_assert_eq!(a, 255, "an opaque pixel must stay visible");
            }
        }
    }
}
