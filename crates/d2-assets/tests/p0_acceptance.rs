//! P0 acceptance tests: the harvested names, DS1 round trips, `.bin` vs
//! `.txt` and Median XL table references, all through [`Overlay`].
//!
//! Real-data tests open the installs read-only (`CAPY_VANILLA_DIR` /
//! `CAPY_MXL_DIR` override the defaults), point the saves layer at an empty
//! temp dir, and print `skipped: <path> missing` when an install is absent.
//! Each profile is harvested once and shared by the tests in this file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use common::{dir_available, mxl_dir, vanilla_dir};
use d2_assets::tables::load_excel;
use d2_assets::{
    AssetError, AssetSource, FolderSource, GamePath, GameVersion, HarvestStats, KnownNames,
    MpqArchive, Overlay, Profile,
};
use d2_formats::ds1::Ds1;
use d2_formats::excel::{self, ExcelTableKind};
use d2_formats::tables;
use d2_formats::txt::TxtTable;

// ---------------------------------------------------------------- fixtures

struct Fixture {
    dir: PathBuf,
    version: GameVersion,
    _saves: tempfile::TempDir,
    overlay: Overlay,
    names: KnownNames,
    stats: HarvestStats,
}

fn build(dir: PathBuf, profile: fn(&Path) -> Profile) -> Option<Fixture> {
    if !dir_available(&dir) {
        return None;
    }
    let profile = profile(&dir);
    let saves = tempfile::tempdir().expect("temp saves dir");
    let overlay = profile.open(saves.path()).expect("open profile");
    let (names, stats) =
        KnownNames::harvest(&overlay, profile.game_version).expect("harvest cannot fail");
    Some(Fixture {
        dir,
        version: profile.game_version,
        _saves: saves,
        overlay,
        names,
        stats,
    })
}

fn vanilla() -> Option<&'static Fixture> {
    static F: OnceLock<Option<Fixture>> = OnceLock::new();
    F.get_or_init(|| build(vanilla_dir(), Profile::vanilla))
        .as_ref()
}

fn mxl() -> Option<&'static Fixture> {
    static F: OnceLock<Option<Fixture>> = OnceLock::new();
    F.get_or_init(|| build(mxl_dir(), Profile::median_xl))
        .as_ref()
}

fn both() -> Vec<(&'static str, &'static Fixture)> {
    [("vanilla", vanilla()), ("mxl", mxl())]
        .into_iter()
        .filter_map(|(label, fx)| fx.map(|fx| (label, fx)))
        .collect()
}

/// Run `f` over `items` on a few threads; outputs come back in input order.
fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .min(8)
        .min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    out.lock().unwrap().push((i, r));
                }
            });
        }
    });
    let mut out = out.into_inner().unwrap();
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

// ------------------------------------------------- 1. DS1 round trip

/// DS1 files allowed to differ after parse -> to_bytes, with the reason.
/// Expected to stay empty: all 2,276 vanilla and 2,424 Median XL DS1 files
/// round-trip byte-identically (files with trailing bytes or short object
/// blocks are preserved by `Preserved`, not special-cased here).
const DS1_ROUND_TRIP_EXCEPTIONS: &[(&str, &str)] = &[];

#[test]
fn ds1_round_trip_is_byte_identical() {
    for (label, fx) in both() {
        let ds1: Vec<GamePath> = fx
            .names
            .iter()
            .filter(|p| p.extension() == Some("ds1"))
            .cloned()
            .collect();
        // (identical, trailing bytes, missing bytes)
        let results = parallel(&ds1, |path| -> Result<(usize, usize), String> {
            let bytes = fx
                .overlay
                .read_bytes(path)
                .map_err(|e| format!("read: {e}"))?
                .ok_or("resolves but cannot be read")?;
            let (parsed, report) =
                Ds1::parse_with_report(&bytes).map_err(|e| format!("parse: {e}"))?;
            let back = parsed.to_bytes().map_err(|e| format!("to_bytes: {e}"))?;
            if back == bytes {
                Ok((report.trailing_bytes, report.missing_bytes))
            } else {
                Err(format!(
                    "round trip differs ({} bytes in, {} out)",
                    bytes.len(),
                    back.len()
                ))
            }
        });
        let mut identical = 0;
        let mut trailing = 0;
        let mut short = 0;
        let mut exceptions: Vec<String> = Vec::new();
        for (path, result) in ds1.iter().zip(results) {
            match result {
                Ok((t, m)) => {
                    identical += 1;
                    trailing += usize::from(t > 0);
                    short += usize::from(m > 0);
                }
                Err(e) => exceptions.push(format!("{path}: {e}")),
            }
        }
        println!(
            "{label}: {} ds1 files, {identical} byte-identical ({trailing} with trailing bytes, {short} short), {} exceptions",
            ds1.len(),
            exceptions.len()
        );
        let unexplained: Vec<&String> = exceptions
            .iter()
            .filter(|e| {
                !DS1_ROUND_TRIP_EXCEPTIONS
                    .iter()
                    .any(|(name, _)| e.starts_with(&format!("{name}:")))
            })
            .collect();
        assert!(unexplained.is_empty(), "{label}: {unexplained:#?}");
        assert!(!ds1.is_empty(), "{label}: no ds1 harvested");
    }
}

// ------------------------------------------------- 2. known names resolve

#[test]
fn harvested_and_listed_names_resolve() {
    for (label, fx) in both() {
        for path in fx.names.iter() {
            assert!(
                fx.overlay.resolve(path).is_some(),
                "{label}: harvested name {path} does not resolve"
            );
        }
        let mut listed = 0usize;
        let mut missing: Vec<String> = Vec::new();
        for layer in fx.overlay.layers() {
            for path in layer.list().expect("list") {
                listed += 1;
                if fx.overlay.resolve(&path).is_none() {
                    missing.push(format!("{} (from {})", path, layer.name()));
                }
                assert!(
                    fx.names.contains(&path),
                    "{label}: {path} (from {}) is listed but not harvested",
                    layer.name()
                );
            }
        }
        assert!(missing.is_empty(), "{label}: {missing:#?}");
        let s = &fx.stats;
        println!(
            "{label}: {} known names ({listed} layer-listed entries); HarvestStats {{ from_listfiles: {}, from_seed: {}, from_tables: {}, from_ds1: {}, from_cof: {}, rounds: {}, unresolved: {} }}, {} warnings",
            fx.names.len(),
            s.from_listfiles,
            s.from_seed,
            s.from_tables,
            s.from_ds1,
            s.from_cof,
            s.rounds,
            s.unresolved,
            s.warnings.len()
        );
        for w in &s.warnings {
            println!("{label}: warning: {w}");
        }
        assert_eq!(s.from_cof, 0);
        assert!(s.warnings.is_empty(), "{label}: {:#?}", s.warnings);
    }
}

/// Patch_D2.mpq has no listfile; report how many of its files the other
/// layers' listfiles name, and how many the harvest names.
#[test]
fn patch_d2_coverage_before_and_after_harvest() {
    for (label, fx) in both() {
        let path = fx.dir.join("Patch_D2.mpq");
        let Ok(patch) = MpqArchive::open(&path) else {
            println!("{label}: {} missing", path.display());
            continue;
        };
        let has_listfile = patch.listfile().expect("listfile").is_some();
        // Before: names from the other layers' own listfiles.
        let mut before: BTreeSet<String> = BTreeSet::new();
        for layer in fx.overlay.layers() {
            if layer.name().eq_ignore_ascii_case("patch_d2.mpq") {
                continue;
            }
            for p in layer.list().expect("list") {
                before.insert(p.as_str().to_owned());
            }
        }
        let cov_before = patch.coverage(before.iter().map(String::as_str));
        // The same, plus the .bin sibling of every excel .txt name.
        let mut with_siblings = before.clone();
        for n in &before {
            if let Some(stem) = n
                .strip_prefix(r"data\global\excel\")
                .and_then(|_| n.strip_suffix(".txt"))
            {
                with_siblings.insert(format!("{stem}.bin"));
            }
        }
        let cov_siblings = patch.coverage(with_siblings.iter().map(String::as_str));
        let cov_after = patch.coverage(fx.names.iter().map(GamePath::as_str));
        println!(
            "{label} Patch_D2.mpq (listfile: {has_listfile}): {} files; named before harvest {}/{} ({} with excel .bin siblings), after harvest {}/{}, unnamed remainder {}",
            patch.file_count(),
            cov_before.named,
            cov_before.total,
            cov_siblings.named,
            cov_after.named,
            cov_after.total,
            cov_after.total - cov_after.named
        );
        assert_eq!(cov_before.total, patch.file_count());
        assert!(cov_after.named >= cov_before.named);
    }
}

// ------------------------------------------------- 3. .bin vs .txt

/// `.txt` rows without the `Expansion` separator the compiler skips.
fn record_rows(t: &TxtTable) -> Vec<usize> {
    t.rows()
        .filter(|r| {
            let cells = r.cells();
            !(cells.first().map(String::as_str) == Some("Expansion")
                && cells.iter().skip(1).all(String::is_empty))
        })
        .map(|r| r.index())
        .collect()
}

/// Every column the `.bin` decodes must equal the `.txt`; returns
/// (columns, cells compared) or the mismatches.
fn compare(label: &str, bin: &TxtTable, txt: &TxtTable) -> Result<(usize, usize), Vec<String>> {
    let rows = record_rows(txt);
    if bin.len() != rows.len() {
        return Err(vec![format!(
            "{label}: {} .bin records but {} .txt records",
            bin.len(),
            rows.len()
        )]);
    }
    let (mut columns, mut cells) = (0, 0);
    let mut failures = Vec::new();
    for (bi, header) in bin.headers().iter().enumerate() {
        let Some(ti) = txt.column(header) else {
            failures.push(format!(
                "{label}: decoded column {header:?} has no .txt column"
            ));
            continue;
        };
        // The .txt `Id` of Objects is informational (three vanilla rows
        // differ); the game addresses records by index.
        let skip_id = label == "objects" && header == "Id";
        columns += 1;
        for (k, &trow) in rows.iter().enumerate() {
            let b = &bin.row(k).unwrap().cells()[bi];
            let t = &txt.row(trow).unwrap().cells()[ti];
            let ok = b == t
                // the compiler writes 0 for a blank numeric cell
                || (t.is_empty() && b == "0")
                || skip_id
                // objects.bin keeps a two-character token; row 0 is 'NU0'
                || (label == "objects" && header == "Token" && t.starts_with(b.as_str()));
            if !ok {
                failures.push(format!(
                    "{label}: record {k} column {header:?}: bin {b:?} != txt {t:?}"
                ));
                break;
            }
            cells += 1;
        }
    }
    if failures.is_empty() {
        Ok((columns, cells))
    } else {
        Err(failures)
    }
}

#[test]
fn load_excel_bin_equals_txt_on_vanilla() {
    let Some(fx) = vanilla() else { return };
    let mut failures = Vec::new();
    for kind in [
        ExcelTableKind::LvlTypes,
        ExcelTableKind::LvlPrest,
        ExcelTableKind::LvlWarp,
        ExcelTableKind::Objects,
        ExcelTableKind::Levels,
    ] {
        let stem = kind.file_stem();
        // 1.13c prefers .bin, 1.14d prefers .txt.
        let bin = load_excel(&fx.overlay, kind, GameVersion::V113c).expect("load .bin");
        let txt = load_excel(&fx.overlay, kind, GameVersion::V114d).expect("load .txt");
        assert_ne!(
            bin.headers(),
            txt.headers(),
            "{stem}: both versions returned the same form"
        );
        match compare(stem, &bin, &txt) {
            Ok((columns, cells)) => println!(
                "vanilla {stem}: {} records, {columns} columns, {cells} cells equal (.bin vs .txt)",
                bin.len()
            ),
            Err(f) => failures.extend(f),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ------------------------------------------------- 4. tile references

/// References that are named by a table but shipped by no layer, with the
/// reason. Checked by exact `profile:path`.
const UNRESOLVED_ALLOW_LIST: &[(&str, &str)] = &[];

fn unresolved_tile_refs(fx: &Fixture) -> (Vec<String>, usize, usize) {
    let types = tables::lvl_types(
        &load_excel(&fx.overlay, ExcelTableKind::LvlTypes, fx.version).expect("lvltypes"),
    )
    .expect("parse lvltypes");
    let prests = tables::lvl_prests(
        &load_excel(&fx.overlay, ExcelTableKind::LvlPrest, fx.version).expect("lvlprest"),
    )
    .expect("parse lvlprest");
    let mut missing = BTreeSet::new();
    let (mut dt1, mut ds1) = (BTreeSet::new(), BTreeSet::new());
    for t in &types {
        for raw in t.files.iter().flatten() {
            if let Some(p) = GamePath::from_tile_ref(raw) {
                dt1.insert(p);
            }
        }
    }
    for p in &prests {
        for raw in p.files.iter().flatten() {
            if let Some(path) = GamePath::from_tile_ref(raw) {
                ds1.insert(path);
            }
        }
    }
    println!(
        "{} distinct dt1 named by {} LvlTypes, {} distinct ds1 named by {} LvlPrest",
        dt1.len(),
        types.len(),
        ds1.len(),
        prests.len()
    );
    for p in dt1.iter().chain(ds1.iter()) {
        if fx.overlay.resolve(p).is_none() {
            missing.insert(p.to_string());
        }
    }
    (missing.into_iter().collect(), dt1.len(), ds1.len())
}

#[test]
fn table_references_resolve() {
    for (label, fx) in both() {
        let (missing, dt1, ds1) = unresolved_tile_refs(fx);
        println!(
            "{label}: {} unresolved of {} dt1 + {} ds1 references",
            missing.len(),
            dt1,
            ds1
        );
        for m in &missing {
            println!("{label}: unresolved: {m}");
        }
        if label == "mxl" {
            let unexplained: Vec<&String> = missing
                .iter()
                .filter(|m| {
                    !UNRESOLVED_ALLOW_LIST
                        .iter()
                        .any(|(key, _)| *key == format!("{label}:{m}"))
                })
                .collect();
            assert!(unexplained.is_empty(), "{label}: {unexplained:#?}");
        }
    }
}

// ------------------------------------------------- 5. load_excel on synthetic folders

fn folder_overlay(files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, Overlay) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, bytes) in files {
        let p = GamePath::new(name).to_native(dir.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    let layer: Box<dyn AssetSource> = Box::new(FolderSource::new(dir.path(), "t"));
    (dir, Overlay::new(vec![layer]))
}

/// A `.bin` of `count` zeroed records whose first record carries `File 1`.
fn lvltypes_bin(file1: &str) -> Vec<u8> {
    let size = excel::record_size(ExcelTableKind::LvlTypes, excel::ExcelVersion::V113c);
    let mut out = 1u32.to_le_bytes().to_vec();
    let mut rec = vec![0u8; size];
    rec[..file1.len()].copy_from_slice(file1.as_bytes());
    out.extend_from_slice(&rec);
    out
}

fn lvltypes_txt(file1: &str) -> Vec<u8> {
    format!("Name\tId\tAct\tFile 1\r\nTown\t1\t1\t{file1}\r\n").into_bytes()
}

fn file1(table: &TxtTable) -> String {
    table.row(0).unwrap().get("File 1").unwrap().to_owned()
}

const BIN: &str = r"data\global\excel\lvltypes.bin";
const TXT: &str = r"data\global\excel\lvltypes.txt";

#[test]
fn load_excel_missing_table_is_a_clean_error() {
    let (_dir, overlay) = folder_overlay(&[(r"data\global\excel\other.txt", b"x".to_vec())]);
    for kind in ExcelTableKind::all() {
        for version in [GameVersion::V113c, GameVersion::V114d, GameVersion::D2r] {
            match load_excel(&overlay, *kind, version) {
                Err(AssetError::Profile { detail, .. }) => {
                    assert!(detail.contains(kind.file_stem()), "{detail}");
                }
                other => panic!("{kind:?} {version:?}: expected a Profile error, got {other:?}"),
            }
        }
    }
    // An overlay with no layers at all behaves the same.
    let empty = Overlay::new(Vec::new());
    assert!(load_excel(&empty, ExcelTableKind::Levels, GameVersion::V113c).is_err());
}

#[test]
fn load_excel_prefers_bin_for_1_13c_and_txt_for_1_14d() {
    let (_dir, overlay) = folder_overlay(&[
        (BIN, lvltypes_bin("bin.dt1")),
        (TXT, lvltypes_txt("txt.dt1")),
    ]);
    let kind = ExcelTableKind::LvlTypes;
    let get = |v| file1(&load_excel(&overlay, kind, v).unwrap());
    assert_eq!(get(GameVersion::V113c), "bin.dt1");
    assert_eq!(get(GameVersion::V114d), "txt.dt1");
    assert_eq!(get(GameVersion::D2r), "txt.dt1", "D2R reads .txt only");
}

#[test]
fn load_excel_falls_back_to_the_other_form() {
    let kind = ExcelTableKind::LvlTypes;
    let (_d, only_bin) = folder_overlay(&[(BIN, lvltypes_bin("bin.dt1"))]);
    for v in [GameVersion::V113c, GameVersion::V114d] {
        assert_eq!(file1(&load_excel(&only_bin, kind, v).unwrap()), "bin.dt1");
    }
    assert!(
        load_excel(&only_bin, kind, GameVersion::D2r).is_err(),
        "D2R never reads .bin"
    );
    let (_d, only_txt) = folder_overlay(&[(TXT, lvltypes_txt("txt.dt1"))]);
    for v in [GameVersion::V113c, GameVersion::V114d, GameVersion::D2r] {
        assert_eq!(file1(&load_excel(&only_txt, kind, v).unwrap()), "txt.dt1");
    }
}

#[test]
fn load_excel_does_not_hide_a_corrupt_preferred_form() {
    let (_dir, overlay) = folder_overlay(&[
        (BIN, vec![1, 0, 0, 0, 9, 9]),
        (TXT, lvltypes_txt("txt.dt1")),
    ]);
    let kind = ExcelTableKind::LvlTypes;
    assert!(matches!(
        load_excel(&overlay, kind, GameVersion::V113c),
        Err(AssetError::Format(_))
    ));
    assert_eq!(
        file1(&load_excel(&overlay, kind, GameVersion::V114d).unwrap()),
        "txt.dt1"
    );
}

#[test]
fn load_excel_levels_bin_needs_leveldefs() {
    let levels_size = excel::record_size(ExcelTableKind::Levels, excel::ExcelVersion::V113c);
    let defs_size = excel::leveldefs_record_size(excel::ExcelVersion::V113c);
    let record = |size: usize| {
        let mut out = 1u32.to_le_bytes().to_vec();
        out.extend(std::iter::repeat_n(0u8, size));
        out
    };
    let levels_bin = (r"data\global\excel\levels.bin", record(levels_size));
    let defs_bin = (r"data\global\excel\leveldefs.bin", record(defs_size));
    let levels_txt = (
        r"data\global\excel\levels.txt",
        b"Id\tLevelType\tDrlgType\r\n0\t7\t1\r\n".to_vec(),
    );
    let kind = ExcelTableKind::Levels;

    let (_d, both_bins) = folder_overlay(&[levels_bin.clone(), defs_bin]);
    let t = load_excel(&both_bins, kind, GameVersion::V113c).unwrap();
    assert!(t.column("LevelType").is_some() && t.column("DrlgType").is_some());

    // levels.bin alone is unusable: error naming leveldefs, or the .txt if there is one.
    let (_d, no_defs) = folder_overlay(std::slice::from_ref(&levels_bin));
    match load_excel(&no_defs, kind, GameVersion::V113c) {
        Err(AssetError::Profile { detail, .. }) => {
            assert!(detail.contains("leveldefs"), "{detail}")
        }
        other => panic!("expected a Profile error, got {other:?}"),
    }
    let (_d, no_defs_txt) = folder_overlay(&[levels_bin, levels_txt]);
    let t = load_excel(&no_defs_txt, kind, GameVersion::V113c).unwrap();
    assert_eq!(t.row(0).unwrap().get_i64("LevelType"), Some(7));
}
