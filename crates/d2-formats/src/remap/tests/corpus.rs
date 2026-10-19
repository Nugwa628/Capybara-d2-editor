//! Private-corpus checks (skipped when `testdata/private/extracted` is
//! absent; a sample unless `CAPY_FULL_CORPUS` is set): identity remaps are
//! byte-identical, an act5 -> act1 style table changes only pixel bytes and
//! the decoded images change exactly by the table, and DCC remaps decode to
//! the mapped frames.

use super::*;
use std::path::PathBuf;

fn corpus_files(ext: &str, every: usize) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            println!("skipped: extracted/{profile} missing");
            continue;
        };
        for p in testutil::sample(testutil::find_files(&root, ext), every) {
            let name = format!("{profile}:{}", p.strip_prefix(&root).unwrap().display());
            out.push((name, p));
        }
    }
    out
}

#[derive(Default)]
struct Outcome {
    /// Parse refused the file (and so did both entry points).
    rejected: Option<String>,
    problem: Option<String>,
    /// Bytes the act table changed.
    changed: usize,
    /// Pixel literal bytes (each once).
    pixel_bytes: usize,
}

/// Decoded images of a DT1 (one per tile; `None` when rendering refuses) or
/// DC6 (one per frame).
fn images(kind: SpriteKind, bytes: &[u8]) -> Vec<Option<IndexedImage>> {
    match kind {
        SpriteKind::Dt1 => Dt1::parse(bytes)
            .unwrap()
            .tiles
            .iter()
            .map(|t| t.render().ok())
            .collect(),
        _ => Dc6::parse(bytes)
            .unwrap()
            .frames
            .iter()
            .map(|f| f.render().ok())
            .collect(),
    }
}

fn check_patched_file(kind: SpriteKind, bytes: &[u8]) -> Result<Outcome, String> {
    let parsed = match kind {
        SpriteKind::Dt1 => Dt1::parse(bytes).map(drop),
        _ => Dc6::parse(bytes).map(drop),
    };
    if let Err(e) = parsed {
        if remap_indices(kind, bytes, &identity()).is_ok() || index_histogram(kind, bytes).is_ok() {
            return Err(format!("parse fails ({e}) but remap or histogram succeeds"));
        }
        return Ok(Outcome {
            rejected: Some(e.to_string()),
            ..Outcome::default()
        });
    }
    let same = remap_indices(kind, bytes, &identity()).map_err(|e| format!("identity: {e}"))?;
    if same != bytes {
        return Err("identity remap changed bytes".to_owned());
    }
    let mask = pixel_mask(kind, bytes);
    let before = index_histogram(kind, bytes).map_err(|e| format!("histogram: {e}"))?;
    let decoded = images(kind, bytes);
    let mut outcome = Outcome {
        pixel_bytes: mask.iter().filter(|&&m| m).count(),
        ..Outcome::default()
    };
    for (t, table) in [act_table(), random_table(bytes.len() as u64)]
        .iter()
        .enumerate()
    {
        let out = remap_indices(kind, bytes, table).map_err(|e| format!("remap {t}: {e}"))?;
        if out.len() != bytes.len() {
            return Err(format!("table {t}: length changed"));
        }
        for (i, ((&a, &b), &px)) in bytes.iter().zip(&out).zip(&mask).enumerate() {
            if px && b != table[usize::from(a)] {
                return Err(format!("table {t}: pixel byte {i:#x} not mapped"));
            }
            if !px && a != b {
                return Err(format!("table {t}: non-pixel byte {i:#x} changed"));
            }
        }
        if t == 0 {
            outcome.changed = bytes.iter().zip(&out).filter(|(a, b)| a != b).count();
        }
        let after = index_histogram(kind, &out).map_err(|e| format!("histogram after: {e}"))?;
        if after != mapped_hist(&before, table) {
            return Err(format!("table {t}: histogram not mapped"));
        }
        for (i, (a, b)) in decoded.iter().zip(images(kind, &out)).enumerate() {
            match (a, b) {
                (Some(a), Some(b)) if b == map_img(a, table) => {}
                (None, None) => {}
                _ => {
                    return Err(format!(
                        "table {t}: image {i} differs from the mapped original"
                    ));
                }
            }
        }
    }
    Ok(outcome)
}

fn run_patch_corpus(kind: SpriteKind, ext: &str) {
    let files = corpus_files(ext, 8);
    if files.is_empty() {
        return;
    }
    let results = testutil::parallel(&files, |(name, path)| {
        let bytes = std::fs::read(path).unwrap();
        match check_patched_file(kind, &bytes) {
            Ok(o) => Outcome {
                rejected: o.rejected.map(|r| format!("{name}: {r}")),
                ..o
            },
            Err(e) => Outcome {
                problem: Some(format!("{name}: {e}")),
                ..Outcome::default()
            },
        }
    });
    let rejected: Vec<&String> = results.iter().filter_map(|r| r.rejected.as_ref()).collect();
    let problems: Vec<&String> = results.iter().filter_map(|r| r.problem.as_ref()).collect();
    let ok = results.len() - rejected.len() - problems.len();
    let touched = results.iter().filter(|r| r.changed > 0).count();
    let changed: usize = results.iter().map(|r| r.changed).sum();
    let pixels: usize = results.iter().map(|r| r.pixel_bytes).sum();
    println!(
        "{ext} remap: {ok}/{} files ok (identity byte-identical, act table and a random \
         table change only pixel bytes, images mapped), {} rejected by parse; act table \
         changed {changed} bytes in {touched} files ({pixels} pixel bytes in all)",
        results.len(),
        rejected.len(),
    );
    for r in rejected.iter().take(10) {
        println!("  rejected: {r}");
    }
    for p in problems.iter().take(40) {
        println!("  {p}");
    }
    assert!(problems.is_empty(), "{} problems", problems.len());
    if kind == SpriteKind::Dt1 {
        // only the 4.1 files (six per install) are unreadable
        assert!(
            rejected.len() <= 12
                && rejected
                    .iter()
                    .all(|r| r.ends_with("unsupported version 4")),
            "{rejected:?}"
        );
    } else {
        assert!(rejected.is_empty(), "{rejected:?}");
    }
}

#[test]
fn corpus_dt1_remap_patches_only_pixels() {
    run_patch_corpus(SpriteKind::Dt1, "dt1");
}

#[test]
fn corpus_dc6_remap_patches_only_pixels() {
    run_patch_corpus(SpriteKind::Dc6, "dc6");
}

#[test]
fn corpus_dcc_remap_decodes_to_mapped_frames() {
    let files = corpus_files("dcc", 40);
    if files.is_empty() {
        return;
    }
    let table = act_table();
    let results = testutil::parallel(&files, |(name, path)| -> (bool, Option<String>) {
        let bytes = std::fs::read(path).unwrap();
        let Ok(original) = Dcc::parse(&bytes) else {
            let both_fail = remap_indices(SpriteKind::Dcc, &bytes, &table).is_err();
            return (
                false,
                (!both_fail).then(|| format!("{name}: parse fails, remap works")),
            );
        };
        let out = match remap_indices(SpriteKind::Dcc, &bytes, &table) {
            Ok(o) => o,
            Err(e) => return (true, Some(format!("{name}: {e}"))),
        };
        let back = match Dcc::parse(&out) {
            Ok(b) => b,
            Err(e) => return (true, Some(format!("{name}: re-parse: {e}"))),
        };
        let expected: Vec<Vec<IndexedImage>> = original
            .directions
            .iter()
            .map(|d| d.frames.iter().map(|f| map_img(f, &table)).collect())
            .collect();
        let got: Vec<Vec<IndexedImage>> =
            back.directions.iter().map(|d| d.frames.clone()).collect();
        if got != expected {
            return (
                true,
                Some(format!("{name}: frames differ from the mapped original")),
            );
        }
        (true, None)
    });
    let parsed = results.iter().filter(|r| r.0).count();
    let ok = results.iter().filter(|r| r.0 && r.1.is_none()).count();
    let problems: Vec<&String> = results.iter().filter_map(|r| r.1.as_ref()).collect();
    println!(
        "dcc remap: {ok}/{parsed} parsed files decode to the act-table-mapped frames \
         ({} files total)",
        results.len()
    );
    for p in problems.iter().take(40) {
        println!("  {p}");
    }
    assert!(problems.is_empty(), "{} problems", problems.len());
}
