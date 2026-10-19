//! Tests against the real vanilla 1.14d and Median XL 1.13c installs.
//!
//! Directories come from `CAPY_VANILLA_DIR` / `CAPY_MXL_DIR` (defaults in
//! `common`). A missing directory prints `skipped: <path> missing`.
//! Everything is opened read-only.
//!
//! Debug-mode runtime is a couple of minutes; `cargo test --release` is
//! several times faster.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{dir_available, mpqs_in, mxl_dir, vanilla_dir};
use d2_assets::MpqArchive;

fn is_wav(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".wav")
}

fn method_label(flags: u32, mask: Option<u8>) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if flags & 0x100 != 0 {
        parts.push("implode(flag 0x100)");
    }
    if let Some(mask) = mask.filter(|_| flags & 0x200 != 0) {
        for (bit, name) in [
            (0x01u8, "huffman"),
            (0x02, "zlib"),
            (0x08, "pklib"),
            (0x10, "bzip2"),
            (0x40, "adpcm-mono"),
            (0x80, "adpcm-stereo"),
        ] {
            if mask & bit != 0 {
                parts.push(name);
            }
        }
    }
    if flags & 0x300 == 0 {
        parts.push("stored");
    } else if flags & 0x100 == 0 && mask.is_none() {
        parts.push("compress-flag, first unit stored raw");
    }
    if flags & 0x1_0000 != 0 {
        parts.push("+encrypted");
    }
    if flags & 0x2_0000 != 0 {
        parts.push("+fixkey");
    }
    if flags & 0x100_0000 != 0 {
        parts.push("+single-unit");
    }
    parts.join(" ")
}

#[derive(Default)]
struct ArchiveReport {
    file: String,
    has_listfile: bool,
    listed: usize,
    existing: usize,
    read_ok: usize,
    skipped_wav: usize,
    bytes: u64,
    failures: Vec<String>,
    tally: BTreeMap<String, usize>,
}

fn check_archive(path: &Path) -> ArchiveReport {
    let mut report = ArchiveReport {
        file: path.file_name().unwrap().to_string_lossy().into_owned(),
        ..ArchiveReport::default()
    };
    let archive = match MpqArchive::open(path) {
        Ok(a) => a,
        Err(e) => {
            report.failures.push(format!("open: {e}"));
            return report;
        }
    };
    let list = match archive.listfile() {
        Ok(l) => l,
        Err(e) => {
            report.failures.push(format!("listfile: {e}"));
            return report;
        }
    };
    let Some(list) = list else {
        return report;
    };
    report.has_listfile = true;
    let mut seen = HashSet::new();
    for name in list {
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        report.listed += 1;
        let Some((size, flags)) = archive.block_info(&name) else {
            continue;
        };
        report.existing += 1;
        if is_wav(&name) {
            report.skipped_wav += 1;
            continue;
        }
        let mask = archive.compression_mask(&name).ok().flatten();
        match archive.read(&name) {
            Ok(Some(data)) => {
                if data.len() as u64 == u64::from(size) {
                    report.read_ok += 1;
                    report.bytes += data.len() as u64;
                    *report.tally.entry(method_label(flags, mask)).or_default() += 1;
                } else {
                    report.failures.push(format!(
                        "{name}: length {} != block size {size}",
                        data.len()
                    ));
                }
            }
            Ok(None) => report
                .failures
                .push(format!("{name}: contains() but read() is None")),
            Err(e) => report.failures.push(format!("{name}: {e}")),
        }
    }
    report
}

/// Check all archives on a small thread pool; reports come back sorted by file.
fn check_all(paths: &[PathBuf]) -> Vec<ArchiveReport> {
    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(path) = paths.get(i) else { break };
                    let report = check_archive(path);
                    out.lock().unwrap().push(report);
                }
            });
        }
    });
    let mut reports = out.into_inner().unwrap();
    reports.sort_by(|a, b| a.file.cmp(&b.file));
    reports
}

fn run_install(
    label: &str,
    dir: PathBuf,
    expected_archives: usize,
) -> Option<BTreeMap<String, usize>> {
    if !dir_available(&dir) {
        return None;
    }
    let paths = mpqs_in(&dir);
    println!("== {label}: {} archives in {}", paths.len(), dir.display());
    assert!(
        paths.len() >= expected_archives,
        "{label}: expected at least {expected_archives} archives, found {}",
        paths.len()
    );
    let reports = check_all(&paths);
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let (mut listed, mut read_ok, mut bytes) = (0usize, 0usize, 0u64);
    for r in &reports {
        println!(
            "{:<32} listfile={:<5} listed={:<6} existing={:<6} read_ok={:<6} wav_skipped={:<5} bytes={}",
            r.file, r.has_listfile, r.listed, r.existing, r.read_ok, r.skipped_wav, r.bytes
        );
        listed += r.listed;
        read_ok += r.read_ok;
        bytes += r.bytes;
        for (k, v) in &r.tally {
            *tally.entry(k.clone()).or_default() += v;
        }
        for f in &r.failures {
            failures.push(format!("{}: {f}", r.file));
        }
    }
    println!("{label} totals: listed={listed} read_ok={read_ok} bytes={bytes}");
    println!("{label} storage/compression tally:");
    for (k, v) in &tally {
        println!("  {k:<48} {v}");
    }
    assert!(
        failures.is_empty(),
        "{label}: {} failures, first: {:#?}",
        failures.len(),
        &failures[..failures.len().min(20)]
    );
    Some(tally)
}

#[test]
fn vanilla_every_listed_file_reads_with_correct_length() {
    let Some(tally) = run_install("vanilla", vanilla_dir(), 11) else {
        return;
    };
    assert!(
        tally
            .keys()
            .any(|k| k.contains("implode") || k.contains("pklib")),
        "no implode/PKWARE file seen on vanilla: {tally:?}"
    );
}

#[test]
fn median_xl_every_listed_file_reads_with_correct_length() {
    if let Some(tally) = run_install("median-xl", mxl_dir(), 68) {
        assert!(
            tally.keys().any(|k| k.contains("implode(flag 0x100)")),
            "no IMPLODE-flag file seen on Median XL: {tally:?}"
        );
    }
}

fn medianxl_archives() -> Vec<PathBuf> {
    mpqs_in(&mxl_dir())
        .into_iter()
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                n.to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("medianxl-")
            })
        })
        .collect()
}

#[test]
fn median_xl_listfiles_use_fix_key_and_do_not_overlap() {
    let dir = mxl_dir();
    if !dir_available(&dir) {
        return;
    }
    let archives = medianxl_archives();
    println!("medianxl-* archives: {}", archives.len());
    assert_eq!(archives.len(), 56, "expected 56 medianxl-* archives");
    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    let mut overlaps = Vec::new();
    let mut listed_total = 0usize;
    let mut existing_total = 0usize;
    let mut fix_key_listfiles = 0usize;
    for path in &archives {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let archive = MpqArchive::open(path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let list = archive
            .listfile()
            .unwrap_or_else(|e| panic!("{file}: listfile: {e}"))
            .unwrap_or_else(|| panic!("{file}: no listfile"));
        let (_, flags) = archive.block_info("(listfile)").expect("listfile block");
        if flags & 0x2_0000 != 0 && flags & 0x1_0000 != 0 {
            fix_key_listfiles += 1;
        }
        let names: BTreeSet<String> = list
            .iter()
            .map(|n| n.to_ascii_lowercase().replace('/', "\\"))
            .collect();
        listed_total += names.len();
        for name in names {
            if archive.contains(&name) {
                existing_total += 1;
                if let Some(prev) = owner.insert(name.clone(), file.clone()) {
                    overlaps.push(format!("{name}: {prev} and {file}"));
                }
            }
        }
    }
    println!("medianxl-* listfile entries (unique per archive): {listed_total}");
    println!("medianxl-* listed files that exist: {existing_total}");
    println!("medianxl-* listfiles with ENCRYPTED+FIX_KEY: {fix_key_listfiles}");
    assert!(
        overlaps.is_empty(),
        "{} overlaps, first: {:?}",
        overlaps.len(),
        &overlaps[..overlaps.len().min(10)]
    );
    assert_eq!(
        existing_total, 8528,
        "total listed+existing files across medianxl-* (listed entries: {listed_total})"
    );
}

#[test]
fn patch_d2_has_no_listfile_and_coverage() {
    let dir = vanilla_dir();
    if !dir_available(&dir) {
        return;
    }
    let patch = MpqArchive::open(&dir.join("Patch_D2.mpq")).expect("open Patch_D2");
    assert!(patch.listfile().expect("listfile").is_none());

    // Union of all vanilla listfile names, plus .bin siblings of excel .txt names.
    let mut names: BTreeSet<String> = BTreeSet::new();
    for path in mpqs_in(&dir) {
        let archive = MpqArchive::open(&path).expect("open");
        if let Some(list) = archive.listfile().expect("listfile") {
            names.extend(list.into_iter().map(|n| n.to_ascii_lowercase()));
        }
    }
    let siblings: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with("data\\global\\excel\\") && n.ends_with(".txt"))
        .map(|n| format!("{}bin", &n[..n.len() - 3]))
        .collect();
    names.extend(siblings);
    let cov = patch.coverage(names.iter().map(String::as_str));
    println!(
        "Patch_D2.mpq: {} files, {} named by vanilla listfiles + excel .bin siblings ({} candidate names), {} unnamed",
        cov.total,
        cov.named,
        names.len(),
        cov.total - cov.named
    );
    assert_eq!(cov.total, patch.file_count());
    assert!(
        cov.named > 0,
        "no Patch_D2 file is reachable by vanilla names"
    );
    assert!(cov.named <= cov.total);
}

#[test]
fn concurrent_reads_are_identical() {
    let dir = vanilla_dir();
    if !dir_available(&dir) {
        return;
    }
    let archive = MpqArchive::open(&dir.join("d2data.mpq")).expect("open");
    let names: Vec<String> = archive
        .listfile()
        .expect("listfile")
        .expect("d2data has a listfile")
        .into_iter()
        .filter(|n| !is_wav(n) && archive.contains(n))
        .take(300)
        .collect();
    assert!(!names.is_empty());
    let expected: Vec<Vec<u8>> = names
        .iter()
        .map(|n| archive.read(n).unwrap().unwrap())
        .collect();
    std::thread::scope(|s| {
        for t in 0..8usize {
            let (archive, names, expected) = (&archive, &names, &expected);
            s.spawn(move || {
                for round in 0..2 {
                    for k in 0..names.len() {
                        // Different orders per thread so reads interleave.
                        let i = (k * (2 * t + 1) + round * 7) % names.len();
                        let got = archive.read(&names[i]).unwrap().unwrap();
                        assert!(got == expected[i], "thread {t}: {} differs", names[i]);
                    }
                }
            });
        }
    });
    println!(
        "concurrent_reads: {} files x 8 threads identical",
        names.len()
    );
}
