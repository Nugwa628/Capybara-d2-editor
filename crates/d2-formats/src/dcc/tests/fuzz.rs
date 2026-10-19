//! Fuzz-style tests: random bytes, mutated synthetic files and mutated
//! private-corpus files must never panic `Dcc::parse`, and the encoder must
//! return errors (not panic) on extreme or decoded-from-hostile frames.
//! Knobs: `PROPTEST_CASES`, `CAPY_FUZZ_ITERS`.

use super::*;
use crate::ds1::fuzz_support::{
    Lcg as Rng, cases, corpus_seeds, fuzz_iters, mutate, run_corpus_mutations,
};

/// Decoded directions up to this many pixels are also re-encoded (the trial
/// encoder runs four times per direction, so big ones only cost time here).
const REENCODE_PIXELS: u64 = 1 << 14;

/// Parses `bytes`, checks the decoded frames' invariants and, for small
/// results, writes the model back with the encoder.
fn exercise(bytes: &[u8]) {
    let Ok((dcc, _warnings)) = parse_checked(bytes) else {
        return;
    };
    let mut pixels = 0u64;
    for dir in &dcc.directions {
        let b = dir.bbox;
        assert!(b.width <= 2048 && b.height <= 2048);
        for f in &dir.frames {
            pixels += u64::from(f.width) * u64::from(f.height);
            assert_eq!(f.pixels.len(), f.width as usize * f.height as usize);
            assert_eq!(f.alpha.len(), f.pixels.len());
            assert!(f.offset_x >= b.x && f.offset_y >= b.y);
            assert!(
                i64::from(f.offset_x) + i64::from(f.width) <= i64::from(b.x) + i64::from(b.width)
            );
            assert!(
                i64::from(f.offset_y) + i64::from(f.height) <= i64::from(b.y) + i64::from(b.height)
            );
            let _ = cell_spans(i64::from(f.offset_x), i64::from(b.x), i64::from(f.width));
        }
    }
    assert!(pixels <= MAX_TOTAL_PIXELS);
    if pixels <= REENCODE_PIXELS
        && let Ok(out) = dcc.to_bytes()
    {
        Dcc::parse(&out).expect("encoder output parses");
    }
}

fn img(w: u32, h: u32, ox: i32, oy: i32, seed: u8) -> IndexedImage {
    let mut i = IndexedImage::new(w, h, ox, oy);
    let n = i.pixels.len();
    i.pixels = (0..n).map(|k| (k as u8).wrapping_mul(31) ^ seed).collect();
    i.alpha = vec![255; n];
    i
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases(128),
        ..ProptestConfig::default()
    })]

    #[test]
    fn fuzz_random_bytes_up_to_8k(bytes in prop::collection::vec(any::<u8>(), 0..8192)) {
        exercise(&bytes);
    }

    #[test]
    fn fuzz_mutated_synthetic_files(
        seed in any::<u64>(),
        ndirs in 1usize..3,
        nframes in 1usize..4,
        rounds in 1usize..10,
    ) {
        let mut rng = Lcg(seed | 1);
        let palette: Vec<u8> = (1..30).collect();
        let dirs: Vec<Vec<Frame>> =
            (0..ndirs).map(|_| random_direction(&mut rng, nframes, &palette)).collect();
        let good = build_file(&dirs, MODES[(seed % 4) as usize]);
        exercise(&good);
        let mut m = Rng(seed);
        exercise(&mutate(&good, &mut m, rounds));
    }

    #[test]
    fn fuzz_truncated_synthetic_files(seed in any::<u64>(), cut in any::<prop::sample::Index>()) {
        let mut rng = Lcg(seed | 1);
        let palette: Vec<u8> = (1..9).collect();
        let dirs = vec![random_direction(&mut rng, 2, &palette)];
        let good = build_file(&dirs, MODES[(seed % 4) as usize]);
        exercise(&good[..cut.index(good.len() + 1)]);
    }

    #[test]
    fn fuzz_cell_spans_any_i64(
        frame_min in prop_oneof![any::<i64>(), -10_000i64..10_000],
        dir_min in prop_oneof![any::<i64>(), -10_000i64..10_000],
        frame_len in prop_oneof![any::<i64>(), -10i64..5000],
    ) {
        let spans = cell_spans(frame_min, dir_min, frame_len);
        if frame_len > 0 && !spans.is_empty() {
            prop_assert_eq!(spans.iter().map(|s| s.1).sum::<i64>(), frame_len);
            prop_assert!(spans.iter().all(|s| (1..=5).contains(&s.1)));
        }
    }

    #[test]
    fn fuzz_encoder_on_odd_frames(
        frames in prop::collection::vec(
            (
                prop::sample::select(vec![0u32, 1, 2, 5, 33]),
                prop::sample::select(vec![0u32, 1, 3, 17]),
                prop_oneof![any::<i32>(), -40i32..40],
                prop_oneof![any::<i32>(), -40i32..40],
                any::<u8>(),
            ),
            1..4,
        ),
    ) {
        let imgs: Vec<IndexedImage> =
            frames.iter().map(|&(w, h, ox, oy, s)| img(w, h, ox, oy, s)).collect();
        // to_bytes may still refuse (a bottom row past i32::MAX), never panic
        if let Ok(dcc) = Dcc::from_frames(std::slice::from_ref(&imgs))
            && let Ok(out) = dcc.to_bytes()
        {
            let back = Dcc::parse(&out).unwrap();
            prop_assert_eq!(back.directions.len(), 1);
        }
    }
}

#[test]
fn fuzz_encoder_rejects_extreme_frames() {
    let ok = img(4, 4, 0, 0, 1);
    let cases: Vec<Vec<Vec<IndexedImage>>> = vec![
        // offsets at both ends of i32 in one direction: the box is too big
        vec![vec![
            img(1, 1, i32::MIN, i32::MIN, 1),
            img(1, 1, i32::MAX, i32::MAX, 2),
        ]],
        // one past the 2048 box
        vec![vec![img(2049, 1, 0, 0, 3)]],
        vec![vec![img(1, 2049, 0, 0, 3)]],
        // declared size without the pixels
        vec![vec![{
            let mut i = IndexedImage::new(0, 0, 0, 0);
            i.width = u32::MAX;
            i.height = u32::MAX;
            i
        }]],
        vec![vec![{
            let mut i = IndexedImage::new(0, 0, 0, 0);
            i.width = u32::MAX;
            i.height = 1;
            i
        }]],
        // too many directions / frames, ragged directions, nothing at all
        vec![vec![ok.clone()]; 33],
        vec![vec![ok.clone(); 1025]],
        vec![vec![ok.clone()], vec![ok.clone(), ok.clone()]],
        vec![],
        vec![vec![]],
    ];
    for (i, dirs) in cases.iter().enumerate() {
        assert!(
            Dcc::from_frames(dirs).is_err(),
            "case {i} should be rejected"
        );
        let model = Dcc {
            version: 6,
            frames_per_direction: dirs.first().map_or(0, |d| d.len() as u32),
            directions: dirs
                .iter()
                .map(|d| DccDirection {
                    frames: d.clone(),
                    bbox: DccBox::default(),
                })
                .collect(),
        };
        assert!(model.to_bytes().is_err(), "case {i} should not encode");
    }
    // the largest legal frames at the ends of the offset range do encode
    for (ox, oy) in [(i32::MIN, i32::MIN), (i32::MAX - 2047, i32::MAX - 2047)] {
        let dcc =
            Dcc::from_frames(&[vec![img(2048, 1, ox, oy, 5), img(1, 2048, ox, oy, 6)]]).unwrap();
        let back = Dcc::parse(&dcc.to_bytes().unwrap()).unwrap();
        assert_eq!(back.directions[0].bbox.x, ox);
    }
}

#[test]
fn fuzz_cell_spans_extremes_do_not_overflow_or_allocate() {
    for (a, b, len) in [
        (i64::MIN, i64::MAX, 5),
        (i64::MAX, i64::MIN, 5),
        (0, 0, i64::MAX),
        (0, 0, i64::MIN),
        (i64::MAX, 0, 10),
        (i64::MAX - 3, 0, 4),
        (0, 0, 1 << 40),
    ] {
        assert!(cell_spans(a, b, len).len() <= 1, "{a} {b} {len}");
    }
    assert_eq!(cell_spans(2, 0, 7), vec![(2, 2), (4, 5)]);
    assert_eq!(cell_spans(0, 0, -3), vec![(0, -3)]);
    assert_eq!(cell_spans(0, 0, 1 << 20).len(), 1 << 18);
}

#[test]
fn fuzz_corpus_dcc_mutations() {
    let seeds = corpus_seeds("dcc", 16, 64 * 1024);
    run_corpus_mutations("dcc", &seeds, fuzz_iters(16), exercise);
}
