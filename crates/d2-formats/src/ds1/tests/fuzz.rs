//! Fuzz-style tests: random bytes, mutated synthetic files and mutated
//! private-corpus files must never panic `Ds1::parse` or anything a parsed
//! model offers. Knobs: `PROPTEST_CASES`, `CAPY_FUZZ_ITERS`.

use super::*;
use crate::ds1::fuzz_support::{Lcg, corpus_seeds, fuzz_iters, mutate, run_corpus_mutations};

/// Parses `bytes` and, when that works, drives every model operation.
fn exercise(bytes: &[u8]) {
    let Ok((ds1, report)) = Ds1::parse_with_report(bytes) else {
        return;
    };
    assert_eq!(report.missing_bytes, ds1.preserved.missing);
    assert_eq!(
        ds1.to_bytes().expect("a parsed model writes back"),
        bytes,
        "parse/write round trip"
    );
    let _ = ds1.effective_act();
    let _ = ds1.workspace_entries().count() + ds1.tileset_refs().count();
    for w in 0..ds1.walls.len() + 1 {
        let _ = ds1.orientation(w, 0, 0);
        let _ = ds1.orientation(w, ds1.width.wrapping_sub(1), ds1.height.wrapping_sub(1));
        let _ = ds1.orientation(w, u32::MAX, u32::MAX);
    }
    let mut m = ds1.clone();
    let _ = m.add_wall_layer();
    let _ = m.add_floor_layer();
    let _ = m.to_bytes();
    m.version = m.min_version_for_model();
    if let Ok(out) = m.to_bytes() {
        let _ = Ds1::parse_with_report(&out);
    }
    for v in [i32::MIN, -1, 0, 1, 3, 7, 12, 13, 18, i32::MAX] {
        let mut m = ds1.clone();
        m.version = v;
        let _ = m.min_version_for_model();
        if let Ok(out) = m.to_bytes() {
            let _ = Ds1::parse_with_report(&out);
        }
    }
}

fn header(version: i32, stored_w: i32, stored_h: i32) -> Vec<u8> {
    let mut b = Vec::new();
    for x in [version, stored_w, stored_h] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

proptest! {
    #[test]
    fn fuzz_random_bytes_up_to_4k(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        exercise(&bytes);
    }

    #[test]
    fn fuzz_plausible_header_random_body(
        version in prop::sample::select(VERSIONS.to_vec()),
        w in -1i32..6,
        h in -1i32..6,
        body in prop::collection::vec(any::<u8>(), 0..2048),
    ) {
        let mut bytes = header(version, w, h);
        bytes.extend_from_slice(&body);
        exercise(&bytes);
    }

    #[test]
    fn fuzz_mutated_synthetic_files(
        version in prop::sample::select(VERSIONS.to_vec()),
        tag_type in -1i32..4,
        w in 0u32..7,
        h in 0u32..7,
        seed in any::<u64>(),
        rounds in 1usize..10,
    ) {
        let mut spec = Spec::new(version).with_tag(tag_type);
        spec.w = w;
        spec.h = h;
        let good = spec.encode();
        exercise(&good);
        let mut rng = Lcg(seed);
        exercise(&mutate(&good, &mut rng, rounds));
    }

    #[test]
    fn fuzz_truncated_synthetic_files(
        version in prop::sample::select(VERSIONS.to_vec()),
        tag_type in 0i32..3,
        cut in any::<prop::sample::Index>(),
    ) {
        let good = Spec::new(version).with_tag(tag_type).encode();
        exercise(&good[..cut.index(good.len() + 1)]);
    }
}

#[test]
fn fuzz_hostile_dimensions_and_counts() {
    // zero-area maps with the largest legal dimension on the other axis
    for (w, h) in [
        (i32::MAX, -1),
        (-1, i32::MAX),
        (-1, -1),
        (i32::MAX, 0),
        (0, i32::MAX),
    ] {
        for version in [1, 3, 4, 13, 16, 18, 1000] {
            let mut b = header(version, w, h);
            b.resize(b.len() + 64, 0);
            exercise(&b);
            for count in [i32::MAX, i32::MIN, -1, 1 << 20] {
                for at in (12..b.len()).step_by(4) {
                    let mut bad = b.clone();
                    bad[at..at + 4].copy_from_slice(&count.to_le_bytes());
                    exercise(&bad);
                }
            }
        }
    }
}

#[test]
fn fuzz_npc_paths_against_many_objects_stay_linear() {
    // 30k objects and 30k paths: the owner check used to be paths x objects
    let mut spec = Spec::new(18);
    spec.w = 1;
    spec.h = 1;
    spec.objects = (0..30_000).map(|i| [1, 1, i, i, 0]).collect();
    spec.paths = (0..30_000)
        .map(|i| {
            if i == 0 {
                (-5, -5, Vec::new())
            } else {
                (i, i, Vec::new())
            }
        })
        .collect();
    let bytes = spec.encode();
    let start = std::time::Instant::now();
    let (ds1, report) = Ds1::parse_with_report(&bytes).unwrap();
    assert_eq!(ds1.npc_paths.len(), 30_000);
    // every path but the first sits on exactly one object
    assert_eq!(
        report
            .warnings
            .iter()
            .filter(|w| w.starts_with("npc path"))
            .count(),
        1
    );
    assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
}

#[test]
fn fuzz_extreme_models_error_instead_of_panicking() {
    let mut m = Ds1::new(0, 0, 0);
    for (w, h) in [
        (u32::MAX, 0),
        (0, u32::MAX),
        (1 << 31, 0),
        ((1 << 31) + 1, 0),
    ] {
        m.width = w;
        m.height = h;
        for wall in &mut m.walls {
            wall.tiles = Grid::from_vec(w, h, Vec::new()).unwrap();
            wall.orientations = Grid::from_vec(w, h, Vec::new()).unwrap();
        }
        for f in &mut m.floors {
            *f = Grid::from_vec(w, h, Vec::new()).unwrap();
        }
        m.shadow = Grid::from_vec(w, h, Vec::new()).unwrap();
        let _ = m.min_version_for_model();
        let _ = m.add_wall_layer();
        match m.to_bytes() {
            Ok(out) => exercise(&out),
            Err(e) => assert!(matches!(e, FormatError::NotRepresentable { .. }), "{e}"),
        }
    }
    // layers that disagree with the declared size
    let mut m = Ds1::new(3, 3, 0);
    m.width = 4;
    assert!(m.to_bytes().is_err());
    let mut m = Ds1::new(3, 3, 0);
    m.version = i32::MIN;
    let _ = m.min_version_for_model();
    let _ = m.to_bytes();
}

#[test]
fn fuzz_corpus_ds1_mutations() {
    let seeds = corpus_seeds("ds1", 24, 256 * 1024);
    run_corpus_mutations("ds1", &seeds, fuzz_iters(40), exercise);
}
