//! Fuzz-style tests: random bytes, mutated synthetic files and mutated
//! private-corpus files must never panic `Dt1::parse` or tile rendering.
//! Knobs: `PROPTEST_CASES`, `CAPY_FUZZ_ITERS`.

use super::*;
use crate::ds1::fuzz_support::{Lcg, corpus_seeds, fuzz_iters, mutate, run_corpus_mutations};

/// Parses `bytes` and renders every tile, also with each block's format
/// swapped (the fields are public, so a caller can do that).
fn exercise(bytes: &[u8]) {
    let Ok(dt1) = Dt1::parse(bytes) else {
        return;
    };
    let palette = crate::palette::Palette {
        colors: [[1, 2, 3]; 256],
    };
    for tile in &dt1.tiles {
        let _ = tile.key();
        if let Ok(img) = tile.render() {
            check_image(&img);
            assert_eq!(img.to_rgba(&palette).len(), img.pixels.len() * 4);
        }
        let mut swapped = tile.clone();
        for b in &mut swapped.blocks {
            b.format = match b.format {
                BlockFormat::Isometric => BlockFormat::Rle,
                BlockFormat::Rle => BlockFormat::Isometric,
            };
        }
        if let Ok(img) = swapped.render() {
            check_image(&img);
        }
    }
}

fn check_image(img: &IndexedImage) {
    assert!(img.width <= 2048 && img.height <= 2048);
    assert_eq!(img.pixels.len(), img.width as usize * img.height as usize);
    assert_eq!(img.alpha.len(), img.pixels.len());
}

fn tiles_strategy() -> impl Strategy<Value = Vec<TileSpec>> {
    prop::collection::vec(
        (
            prop::collection::vec(block_strategy(), 0..4),
            any::<u8>(),
            -1i32..300,
        )
            .prop_map(|(mut blocks, seed, orientation)| {
                blocks.push(BlockSpec {
                    x: 0,
                    y: 0,
                    grid: (0, 0),
                    format: 1,
                    data: iso_data(seed),
                });
                TileSpec::new(orientation, 1, 2, blocks)
            }),
        0..4,
    )
}

proptest! {
    #[test]
    fn fuzz_random_bytes_up_to_8k(bytes in prop::collection::vec(any::<u8>(), 0..8192)) {
        exercise(&bytes);
    }

    #[test]
    fn fuzz_mutated_synthetic_files(
        tiles in tiles_strategy(),
        seed in any::<u64>(),
        rounds in 1usize..10,
    ) {
        let good = build(&tiles);
        exercise(&good);
        let mut rng = Lcg(seed);
        exercise(&mutate(&good, &mut rng, rounds));
    }

    #[test]
    fn fuzz_truncated_synthetic_files(
        tiles in tiles_strategy(),
        cut in any::<prop::sample::Index>(),
    ) {
        let good = build(&tiles);
        exercise(&good[..cut.index(good.len() + 1)]);
    }
}

#[test]
fn fuzz_header_fields_take_nasty_values() {
    let good = build(&[
        TileSpec::new(
            3,
            1,
            2,
            vec![
                BlockSpec {
                    x: -32,
                    y: 7,
                    grid: (1, 2),
                    format: 1,
                    data: iso_data(9),
                },
                BlockSpec {
                    x: 64,
                    y: -15,
                    grid: (0, 0),
                    format: 0,
                    data: vec![2, 3, 9, 9, 9, 0, 0, 1, 2, 7, 7],
                },
            ],
        ),
        TileSpec::new(0, 4, 5, Vec::new()),
    ]);
    exercise(&good);
    for at in (0..good.len().saturating_sub(3)).step_by(2) {
        for v in [
            0i32,
            -1,
            i32::MIN,
            i32::MAX,
            0x7FFF,
            -0x8000,
            0xFFFF,
            1 << 20,
        ] {
            let mut bad = good.clone();
            bad[at..at + 4].copy_from_slice(&v.to_le_bytes());
            exercise(&bad);
        }
    }
}

#[test]
fn fuzz_shared_block_headers_cannot_amplify() {
    // 1000 tiles that all claim the same 1000 zero-length sub-block headers:
    // a million `Block`s from a 116 KB file unless headers count against the
    // copy budget.
    let (tiles, blocks) = (1000usize, 1000usize);
    let mut out = vec![0u8; 276 + 96 * tiles];
    out[0..4].copy_from_slice(&7i32.to_le_bytes());
    out[4..8].copy_from_slice(&6i32.to_le_bytes());
    out[268..272].copy_from_slice(&(tiles as i32).to_le_bytes());
    out[272..276].copy_from_slice(&276i32.to_le_bytes());
    let ptr = out.len();
    for t in 0..tiles {
        let h = 276 + 96 * t;
        out[h + 72..h + 76].copy_from_slice(&(ptr as i32).to_le_bytes());
        out[h + 80..h + 84].copy_from_slice(&(blocks as i32).to_le_bytes());
    }
    out.resize(ptr + 20 * blocks, 0);
    assert!(Dt1::parse(&out).is_err());
    // a handful of tiles sharing them is still fine (well under the slack)
    out[268..272].copy_from_slice(&3i32.to_le_bytes());
    assert_eq!(Dt1::parse(&out).unwrap().tiles.len(), 3);
}

#[test]
fn fuzz_corpus_dt1_mutations() {
    let seeds = corpus_seeds("dt1", 16, 512 * 1024);
    run_corpus_mutations("dt1", &seeds, fuzz_iters(24), exercise);
}
