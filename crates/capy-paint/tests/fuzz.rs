//! Fuzz-style robustness tests: masks, exports and the DCC cell check on
//! images whose fields disagree with their buffers, extreme offsets and
//! coordinates, and session input that must be rejected rather than
//! trusted. Nothing may panic. Case counts follow `PROPTEST_CASES`.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use capy_paint::{
    Dither, FrameRef, GradientColors, GradientShape, Mask, MaskOp, PaintSession, animated_gif,
    dcc_cell_violations, indexed_png, onion_composite, reduce_dcc_cells,
};
use common::test_palette;
use d2_formats::IndexedImage;
use proptest::prelude::*;

fn coord() -> impl Strategy<Value = i32> {
    prop_oneof![-20i32..30, Just(i32::MIN), Just(i32::MAX), any::<i32>()]
}

/// Small sides, or huge ones no buffer backs.
fn side() -> impl Strategy<Value = u32> {
    prop_oneof![0u32..20, Just(u32::MAX), (1u32 << 31)..=u32::MAX]
}

fn hostile_image() -> impl Strategy<Value = IndexedImage> {
    (
        side(),
        side(),
        coord(),
        coord(),
        prop::collection::vec(any::<u8>(), 0..300),
        prop::collection::vec(any::<u8>(), 0..300),
    )
        .prop_map(
            |(width, height, offset_x, offset_y, pixels, alpha)| IndexedImage {
                width,
                height,
                offset_x,
                offset_y,
                pixels,
                alpha,
            },
        )
}

/// A consistent image of 1..12 pixels a side at any offset.
fn good_image() -> impl Strategy<Value = IndexedImage> {
    (1u32..12, 1u32..12, coord(), coord(), any::<u64>()).prop_map(|(w, h, ox, oy, seed)| {
        let n = (w * h) as usize;
        let pixels: Vec<u8> = (0..n)
            .map(|i| (seed.rotate_left(i as u32 % 64) as u8) % 9)
            .collect();
        let alpha = pixels
            .iter()
            .map(|&p| if p == 0 { 0 } else { 255 })
            .collect();
        IndexedImage {
            width: w,
            height: h,
            offset_x: ox,
            offset_y: oy,
            pixels,
            alpha,
        }
    })
}

/// A mask whose declared size is small or zero-area huge, with any bits.
fn hostile_mask() -> impl Strategy<Value = Mask> {
    prop_oneof![
        (
            0u32..16,
            0u32..16,
            prop::collection::vec(any::<bool>(), 0..300)
        )
            .prop_map(|(width, height, bits)| Mask {
                width,
                height,
                bits
            }),
        (side(), prop::collection::vec(any::<bool>(), 0..40)).prop_map(|(width, bits)| Mask {
            width,
            height: 0,
            bits
        }),
        (side(), prop::collection::vec(any::<bool>(), 0..40)).prop_map(|(height, bits)| Mask {
            width: 0,
            height,
            bits
        }),
    ]
}

fn float() -> impl Strategy<Value = f32> {
    prop_oneof![
        -30.0f32..30.0,
        Just(f32::NAN),
        Just(f32::INFINITY),
        Just(f32::NEG_INFINITY),
        any::<f32>()
    ]
}

proptest! {
    #[test]
    fn masks_take_any_input(
        m in hostile_mask(),
        other in hostile_mask(),
        (w, h) in (0u32..16, 0u32..16),
        huge in side(),
        (a, b, c, d) in (coord(), coord(), coord(), coord()),
        poly in prop::collection::vec((float(), float()), 0..6),
        px in prop_oneof![0u32..6, any::<u32>()],
        op in prop::sample::select(vec![MaskOp::Replace, MaskOp::Add, MaskOp::Subtract, MaskOp::Intersect]),
    ) {
        let r = Mask::rect(w, h, a, b, c, d);
        prop_assert_eq!(r.bits.len(), (w * h) as usize);
        // Zero-area masks with a huge side.
        let _ = Mask::rect(huge, 0, a, b, c, d);
        let _ = Mask::rect(0, huge, a, b, c, d);
        let _ = Mask::ellipse(w, h, a, b, c, d);
        let _ = Mask::lasso(w, h, &poly);
        let mut x = m.clone();
        x.combine(&other, op);
        x.invert();
        let _ = x.bounds();
        let _ = x.count();
        let _ = x.get(a, b);
        x.set(c, d, true);
        let _ = m.grow(px);
        let _ = m.shrink(px);
    }

    #[test]
    fn exports_take_any_image(
        imgs in prop::collection::vec(prop_oneof![hostile_image(), good_image()], 0..4),
        fade in float(),
        delay in any::<u16>(),
    ) {
        let pal = test_palette();
        for img in &imgs {
            let _ = indexed_png(img, &pal);
        }
        if let Ok(gif) = animated_gif(&imgs, &pal, delay) {
            prop_assert!(!gif.is_empty());
        }
        if let Some(cur) = imgs.first() {
            let (w, h, rgba) = onion_composite(cur, imgs.get(1), imgs.get(2), &pal, fade);
            prop_assert_eq!(rgba.len(), w as usize * h as usize * 4);
        }
    }

    #[test]
    fn dcc_check_takes_any_frames(
        frames in prop::collection::vec(
            prop::collection::vec(prop_oneof![hostile_image(), good_image()], 0..3),
            0..3,
        ),
    ) {
        for v in dcc_cell_violations(&frames) {
            prop_assert!(v.colors > 4);
        }
    }

    /// Consistent frames at any offset go through a session; reduction keeps
    /// the shape. Inconsistent ones must be rejected by `new`.
    #[test]
    fn sessions_accept_good_frames_and_reject_bad_ones(
        good in prop::collection::vec(good_image(), 1..4),
        bad in hostile_image(),
        clip in prop_oneof![hostile_image(), good_image()],
        at in (coord(), coord()),
        rgba in (0u32..6, 0u32..6, prop::collection::vec(any::<u8>(), 0..150)),
        resize in (prop_oneof![0u32..20, any::<u32>()], prop_oneof![0u32..20, any::<u32>()]),
        grad in (float(), float(), float(), float()),
    ) {
        let consistent = bad.pixels.len() == (bad.width as usize).saturating_mul(bad.height as usize)
            && bad.alpha.len() == bad.pixels.len()
            && bad.width <= 8192
            && bad.height <= 8192;
        let r = PaintSession::new(vec![vec![bad]], test_palette());
        prop_assert_eq!(r.is_ok(), consistent);

        let mut s = PaintSession::new(vec![good], test_palette()).unwrap();
        let f = FrameRef { dir: 0, frame: 0 };
        let _ = reduce_dcc_cells(&mut s);
        let _ = s.paste(f, &clip, at);
        let _ = s.paste_rgba(f, rgba.0, rgba.1, &rgba.2, at, Dither::FloydSteinberg);
        let _ = s.gradient(
            f,
            (grad.0, grad.1),
            (grad.2, grad.3),
            GradientShape::Radial,
            GradientColors::Two(1, 2),
            Dither::Bayer4,
            None,
        );
        let _ = s.resize_canvas(f, resize.0, resize.1, at);
        let _ = s.set_offset(f, at.0, at.1);
        let _ = s.mirror_direction(0, 0);
        let _ = dcc_cell_violations(s.frames());
        for dir in s.frames() {
            for img in dir {
                prop_assert_eq!(img.pixels.len(), img.width as usize * img.height as usize);
                prop_assert_eq!(img.alpha.len(), img.pixels.len());
            }
        }
    }
}
