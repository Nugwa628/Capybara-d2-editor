//! Fuzz-style tests: random bytes, mutated synthetic files and mutated
//! private-corpus files must never panic `Dc6::parse`, frame rendering, or
//! the encoder fed with what a hostile file decodes to.
//! Knobs: `PROPTEST_CASES`, `CAPY_FUZZ_ITERS`.

use super::*;
use crate::ds1::fuzz_support::{Lcg, corpus_seeds, fuzz_iters, mutate, run_corpus_mutations};

/// Frames up to this many pixels are rendered and re-encoded (the decoder
/// allows 16 Mi; bigger ones only cost time here).
const RENDER_PIXELS: u64 = 1 << 18;

/// Parses `bytes`, renders the frames, writes the model back and re-encodes
/// the rendered images.
fn exercise(bytes: &[u8]) {
    let Ok(dc6) = Dc6::parse(bytes) else {
        return;
    };
    assert_eq!(
        dc6.frames.len() as u64,
        u64::from(dc6.directions) * u64::from(dc6.frames_per_direction)
    );
    let _ = dc6.frame(0, 0);
    let _ = dc6.frame(dc6.directions.wrapping_sub(1), dc6.frames_per_direction);
    let _ = dc6.frame(u32::MAX, u32::MAX);

    let written = dc6.to_bytes().expect("a parsed model writes back");
    assert_eq!(
        Dc6::parse(&written).expect("written bytes parse"),
        dc6,
        "write/parse round trip"
    );

    let mut images = Vec::new();
    for f in &dc6.frames {
        if u64::from(f.width) * u64::from(f.height) > RENDER_PIXELS {
            continue;
        }
        let img = f.render();
        if let Ok(img) = img {
            assert_eq!(img.pixels.len(), f.width as usize * f.height as usize);
            assert_eq!(img.alpha.len(), img.pixels.len());
            images.push(img);
        }
    }
    // the encoder on whatever the hostile file rendered to
    if let Ok(enc) = Dc6::from_frames(std::slice::from_ref(&images))
        && let Ok(out) = enc.to_bytes()
    {
        let back = Dc6::parse(&out).expect("encoder output parses");
        for (f, want) in back.frames.iter().zip(&images) {
            let got = f.render().expect("re-encoded frame renders");
            assert_eq!(&got, want, "re-encoded frame differs");
        }
    }
}

fn frames_strategy() -> impl Strategy<Value = (u32, u32, Vec<Img>)> {
    (1u32..3, 1u32..3).prop_flat_map(|(dirs, fpd)| {
        let n = (dirs * fpd) as usize;
        (
            Just(dirs),
            Just(fpd),
            prop::collection::vec(
                (
                    0usize..40,
                    0usize..20,
                    any::<u8>(),
                    any::<bool>(),
                    any::<i32>(),
                    any::<i32>(),
                )
                    .prop_map(|(w, h, seed, flip, ox, oy)| {
                        let mut i = img(w, h, seed, flip);
                        i.ox = ox;
                        i.oy = oy;
                        i
                    }),
                n,
            ),
        )
    })
}

proptest! {
    #[test]
    fn fuzz_random_bytes_up_to_8k(bytes in prop::collection::vec(any::<u8>(), 0..8192)) {
        exercise(&bytes);
    }

    #[test]
    fn fuzz_mutated_synthetic_files(
        (dirs, fpd, frames) in frames_strategy(),
        seed in any::<u64>(),
        rounds in 1usize..10,
    ) {
        let good = build(dirs, fpd, &frames);
        exercise(&good);
        let mut rng = Lcg(seed);
        exercise(&mutate(&good, &mut rng, rounds));
    }

    #[test]
    fn fuzz_truncated_synthetic_files(
        (dirs, fpd, frames) in frames_strategy(),
        cut in any::<prop::sample::Index>(),
    ) {
        let good = build(dirs, fpd, &frames);
        exercise(&good[..cut.index(good.len() + 1)]);
    }

    #[test]
    fn fuzz_frame_table_into_random_body(
        dirs in 0u32..4,
        fpd in 0u32..4,
        table in prop::collection::vec(0u32..600, 16),
        body in prop::collection::vec(any::<u8>(), 0..600),
    ) {
        let mut bytes = Vec::new();
        for v in [6u32, 1, 0, 0xEEEE_EEEE, dirs, fpd] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for off in table.iter().take((dirs * fpd) as usize) {
            bytes.extend_from_slice(&off.to_le_bytes());
        }
        bytes.extend_from_slice(&body);
        exercise(&bytes);
    }

    #[test]
    fn fuzz_rendering_extreme_frame_fields(
        width in prop::sample::select(vec![0u32, 1, 7, 4096, 4097, u32::MAX]),
        height in prop::sample::select(vec![0u32, 1, 3, 4096, 4097, u32::MAX]),
        offset_x in any::<i32>(),
        offset_y in prop::sample::select(vec![i32::MIN, -1, 0, 5, i32::MAX]),
        flipped in any::<bool>(),
        data in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let f = Dc6Frame { flipped, width, height, offset_x, offset_y, data };
        if u64::from(width) * u64::from(height) <= RENDER_PIXELS {
            let _ = f.render();
        } else if width > 4096 || height > 4096 {
            prop_assert!(f.render().is_err());
        }
        // the writer takes any model and the result parses or errors
        let model = Dc6 {
            version: 6,
            flags: 1,
            encoding: 0,
            directions: 1,
            frames_per_direction: 1,
            frames: vec![f],
        };
        let out = model.to_bytes().unwrap();
        let _ = Dc6::parse(&out);
    }
}

#[test]
fn fuzz_shared_frame_headers_cannot_amplify() {
    // 200k table entries all pointing at one zero-length frame header
    let n = 200_000u32;
    let mut out = Vec::new();
    for v in [6u32, 1, 0, 0, 1, n] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let frame_at = 24 + 4 * n;
    for _ in 0..n {
        out.extend_from_slice(&frame_at.to_le_bytes());
    }
    out.extend_from_slice(&[0u8; 32]);
    assert!(Dc6::parse(&out).is_err());
}

#[test]
fn fuzz_encoder_on_extreme_models() {
    // counts that overflow or disagree with the frames present
    for (dirs, fpd) in [(u32::MAX, u32::MAX), (u32::MAX, 0), (2, 1), (0, 5)] {
        let model = Dc6 {
            version: 6,
            flags: 1,
            encoding: 0,
            directions: dirs,
            frames_per_direction: fpd,
            frames: Vec::new(),
        };
        assert_eq!(model.to_bytes().is_ok(), dirs == 0 || fpd == 0);
    }
    // images at the ends of the offset range
    for (ox, oy) in [(i32::MIN, i32::MIN), (i32::MAX, i32::MAX), (0, i32::MAX)] {
        let mut img = IndexedImage::new(3, 2, ox, oy);
        img.pixels = vec![1, 0, 2, 3, 3, 0];
        img.alpha = vec![255; 6];
        let dc6 = Dc6::from_frames(&[vec![img.clone(), IndexedImage::new(0, 0, ox, oy)]]).unwrap();
        exercise(&dc6.to_bytes().unwrap());
    }
    // buffers shorter than the declared size: missing pixels are transparent
    let mut img = IndexedImage::new(0, 0, 0, 0);
    img.width = 300;
    img.height = 2;
    let f = Dc6Frame::from_image(&img);
    assert_eq!(f.render().unwrap().pixels, vec![0; 600]);
}

#[test]
fn fuzz_corpus_dc6_mutations() {
    let seeds = corpus_seeds("dc6", 24, 256 * 1024);
    run_corpus_mutations("dc6", &seeds, fuzz_iters(40), exercise);
}
