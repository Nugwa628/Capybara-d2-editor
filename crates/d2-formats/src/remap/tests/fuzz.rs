//! Fuzz-style tests: random bytes, plausible headers and mutated synthetic
//! and private-corpus files must never panic `index_histogram` or
//! `remap_indices`, and whenever a DT1/DC6 remap succeeds the patch contract
//! holds. Knobs: `PROPTEST_CASES`, `CAPY_FUZZ_ITERS`.

use super::*;

const KINDS: [SpriteKind; 3] = [SpriteKind::Dt1, SpriteKind::Dc6, SpriteKind::Dcc];

/// Runs both entry points on `bytes` as every kind (DCC only when `dcc`:
/// re-encoding is the slow part) and checks what can be checked.
fn exercise(bytes: &[u8], seed: u64, dcc: bool) {
    let table = random_table(seed);
    let mut bad = identity();
    bad[1 + (seed % 255) as usize] = 0;
    for kind in KINDS {
        if kind == SpriteKind::Dcc && !dcc {
            continue;
        }
        assert!(remap_indices(kind, bytes, &bad).is_err());
        let hist = index_histogram(kind, bytes);
        let out = remap_indices(kind, bytes, &table);
        match kind {
            SpriteKind::Dt1 | SpriteKind::Dc6 => match out {
                Ok(out) => {
                    assert!(hist.is_ok());
                    let mask = pixel_mask(kind, bytes);
                    check_patch(kind, bytes, &out, &table, &mask);
                    assert_eq!(remap_indices(kind, bytes, &identity()).unwrap(), bytes);
                }
                // overlapping pixels and headers parse fine but cannot be
                // patched; anything else is a parse failure
                Err(FormatError::Invalid {
                    what: "pixel data", ..
                }) => assert!(hist.is_ok()),
                Err(_) => assert!(hist.is_err()),
            },
            SpriteKind::Dcc => {
                if let Ok(out) = out {
                    let back = Dcc::parse(&out).unwrap();
                    let total: u64 = index_histogram(kind, &out).unwrap().iter().sum();
                    let opaque: u64 = back
                        .directions
                        .iter()
                        .flat_map(|d| &d.frames)
                        .map(|f| f.alpha.iter().filter(|&&a| a != 0).count() as u64)
                        .sum();
                    assert_eq!(total, opaque);
                    assert_eq!(
                        index_histogram(kind, &out).unwrap(),
                        mapped_hist(&hist.unwrap(), &table)
                    );
                }
            }
        }
    }
}

proptest! {
    #[test]
    fn fuzz_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096), seed in any::<u64>()) {
        exercise(&bytes, seed, true);
    }

    #[test]
    fn fuzz_plausible_dt1_header(
        count in 0i32..4,
        body in prop::collection::vec(any::<u8>(), 0..2048),
        seed in any::<u64>(),
    ) {
        let mut bytes = vec![0u8; 276];
        bytes[0..4].copy_from_slice(&7i32.to_le_bytes());
        bytes[268..272].copy_from_slice(&count.to_le_bytes());
        bytes[272..276].copy_from_slice(&276i32.to_le_bytes());
        bytes.extend(body);
        exercise(&bytes, seed, false);
    }

    #[test]
    fn fuzz_plausible_dc6_header(
        dirs in 0u32..3,
        frames in 0u32..3,
        body in prop::collection::vec(any::<u8>(), 0..1024),
        seed in any::<u64>(),
    ) {
        let mut bytes = Vec::new();
        for v in [6u32, 1, 0, 0xEEEE_EEEE, dirs, frames] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend(body);
        exercise(&bytes, seed, false);
    }

    #[test]
    fn fuzz_mutated_fixtures(seed in any::<u64>()) {
        let mut rng = testutil::Rng::new(seed);
        let dt1 = Dt1Fixture::new().bytes;
        let dc6 = dc6_fixture().0;
        let dcc = dcc_fixture();
        exercise(&testutil::mutate(&dt1, &mut rng), seed, false);
        exercise(&testutil::mutate(&dc6, &mut rng), seed, false);
        exercise(&testutil::mutate(&dcc, &mut rng), seed, true);
    }
}

/// Seeded mutations of a few small private-corpus files of each kind
/// (skipped when the corpus is absent).
#[test]
fn fuzz_mutated_corpus_files() {
    let iters = testutil::fuzz_iters("remap corpus mutations", 40);
    let mut seeds = Vec::new();
    for ext in ["dt1", "dc6", "dcc"] {
        let small: Vec<_> = testutil::corpus_seeds(ext, "", 400)
            .into_iter()
            .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= 64 * 1024))
            .collect();
        let step = small.len().div_ceil(4).max(1);
        seeds.extend(small.into_iter().step_by(step).map(|p| (ext, p)));
    }
    if seeds.is_empty() {
        println!("skipped: no private corpus");
        return;
    }
    let runs = testutil::parallel(&seeds, |(ext, path)| {
        let bytes = std::fs::read(path).unwrap();
        let mut rng = testutil::Rng::new(bytes.len() as u64);
        exercise(&bytes, 1, *ext == "dcc");
        for i in 0..iters {
            exercise(&testutil::mutate(&bytes, &mut rng), i as u64, *ext == "dcc");
        }
        1usize
    });
    println!(
        "remap corpus mutations: {} seed files x {iters} mutations",
        runs.len()
    );
}
