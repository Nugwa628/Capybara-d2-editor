//! `presets` against the real installs (vanilla and Median XL), read-only.
//!
//! Skipped (with a `skipped:` line) when an install or the legacy ini corpus
//! is absent. `CAPY_VANILLA_DIR`, `CAPY_MXL_DIR` and `CAPY_LEGACY_INI_DIR`
//! override the default locations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::{dir_available, mxl_dir, vanilla_dir};
use d2_assets::tables::load_excel;
use d2_assets::{
    GamePath, GameVersion, KnownNames, MapCatalog, MapInfo, MapInfoSource, Overlay, Profile,
    parse_legacy_ini,
};
use d2_formats::excel::ExcelTableKind;
use d2_formats::tables;

const DEFAULT_LEGACY_DIR: &str =
    "C:\\Users\\ZF\\Desktop\\sugma\\capybara-mapeditor\\testdata\\private\\oracle\\assets";

fn legacy_dir() -> PathBuf {
    std::env::var_os("CAPY_LEGACY_INI_DIR")
        .map_or_else(|| PathBuf::from(DEFAULT_LEGACY_DIR), PathBuf::from)
}

struct Install {
    label: &'static str,
    version: GameVersion,
    _saves: tempfile::TempDir,
    overlay: Overlay,
    ds1: Vec<GamePath>,
}

fn open(label: &'static str, dir: &Path, profile: fn(&Path) -> Profile) -> Option<Install> {
    if !dir_available(dir) {
        return None;
    }
    let profile = profile(dir);
    let saves = tempfile::tempdir().unwrap();
    let overlay = profile.open(saves.path()).unwrap();
    let (names, _) = KnownNames::harvest(&overlay, profile.game_version).unwrap();
    let ds1 = names
        .iter()
        .filter(|p| p.extension() == Some("ds1"))
        .cloned()
        .collect();
    Some(Install {
        label,
        version: profile.game_version,
        _saves: saves,
        overlay,
        ds1,
    })
}

fn installs() -> Vec<Install> {
    [
        open("vanilla", &vanilla_dir(), Profile::vanilla),
        open("mxl", &mxl_dir(), Profile::median_xl),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// DS1 paths named by any LvlPrest File1..6, restricted to harvested names.
fn prest_named(install: &Install) -> BTreeSet<GamePath> {
    let table = load_excel(&install.overlay, ExcelTableKind::LvlPrest, install.version).unwrap();
    let known: BTreeSet<&GamePath> = install.ds1.iter().collect();
    let mut out = BTreeSet::new();
    for prest in tables::lvl_prests(&table).unwrap() {
        for file in prest.files.iter().flatten() {
            if let Some(path) = GamePath::from_tile_ref(file)
                && known.contains(&path)
            {
                out.insert(path);
            }
        }
    }
    out
}

fn catalog(install: &Install) -> MapCatalog {
    let (catalog, warnings) =
        MapCatalog::build(&install.overlay, install.version, &install.ds1).unwrap();
    println!("{}: build warnings: {}", install.label, warnings.len());
    for w in warnings.iter().take(20) {
        println!("  {w}");
    }
    catalog
}

fn describe(info: &MapInfo) -> String {
    format!(
        "{:?} type={:?} def={:?} slot={:?} act={} pal={} dt1={} missing={}",
        info.source,
        info.lvl_type,
        info.lvlprest_def,
        info.lvlprest_slot,
        info.act,
        info.palette_act,
        info.dt1_files.len(),
        info.missing_dt1.len()
    )
}

#[test]
fn source_counts_and_missing_dt1s() {
    for install in installs() {
        let catalog = catalog(&install);
        let mut per_source: BTreeMap<MapInfoSource, usize> = BTreeMap::new();
        let mut with_missing = Vec::new();
        let mut empty_lookup = Vec::new();
        for ds1 in &install.ds1 {
            let all = catalog.lookup(ds1);
            let Some(best) = all.first() else {
                empty_lookup.push(ds1.clone());
                continue;
            };
            *per_source.entry(best.source).or_default() += 1;
            if !best.missing_dt1.is_empty() {
                with_missing.push((ds1.clone(), best.missing_dt1.clone()));
            }
        }
        println!(
            "{}: {} ds1, by best source {:?}, {} with missing DT1s, {} lvltypes",
            install.label,
            install.ds1.len(),
            per_source,
            with_missing.len(),
            catalog.lvl_type_ids().len()
        );
        for (ds1, missing) in with_missing.iter().take(10) {
            println!("  missing: {ds1} -> {missing:?}");
        }
        assert!(empty_lookup.is_empty(), "{empty_lookup:?}");
    }
}

#[test]
fn vanilla_prest_named_ds1s_get_lvlprest_source() {
    let Some(install) = open("vanilla", &vanilla_dir(), Profile::vanilla) else {
        return;
    };
    let catalog = catalog(&install);
    let named = prest_named(&install);
    assert!(!named.is_empty());
    let bad: Vec<String> = named
        .iter()
        .filter_map(|ds1| {
            let best = catalog.best(ds1).unwrap();
            (best.source != MapInfoSource::LvlPrest).then(|| format!("{ds1}: {}", describe(&best)))
        })
        .collect();
    println!(
        "vanilla: {} LvlPrest-named ds1s, {} not LvlPrest",
        named.len(),
        bad.len()
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Vanilla crypt presets (LevelId 0) that Median XL left in LvlPrest but whose
/// tilesets it dropped: LvlTypes 4 now lists `mxl\quest\yshari` DT1s and no
/// LvlType names a file under `act1\crypt\`, so there is nothing to resolve.
const MXL_ORPHAN_PRESET_DIRS: &[&str] = &["data\\global\\tiles\\act1\\crypt\\"];

#[test]
fn mxl_prest_named_ds1s_resolve_fully() {
    let Some(install) = open("mxl", &mxl_dir(), Profile::median_xl) else {
        return;
    };
    let catalog = catalog(&install);
    let named = prest_named(&install);
    assert!(!named.is_empty());
    let mut problems: Vec<(GamePath, String)> = Vec::new();
    for ds1 in &named {
        for info in &catalog.lookup(ds1) {
            if info.source != MapInfoSource::LvlPrest {
                problems.push((ds1.clone(), format!("not LvlPrest: {}", describe(info))));
            } else if info.dt1_files.is_empty() {
                problems.push((ds1.clone(), format!("no dt1 files: {}", describe(info))));
            } else if !info.missing_dt1.is_empty() {
                problems.push((ds1.clone(), format!("missing {:?}", info.missing_dt1)));
            }
        }
    }
    let mut per_dir: BTreeMap<String, usize> = BTreeMap::new();
    for (ds1, _) in &problems {
        let dir = ds1.as_str().rsplit_once('\\').map_or("", |(d, _)| d);
        *per_dir.entry(dir.to_owned()).or_default() += 1;
    }
    println!(
        "mxl: {} LvlPrest-named ds1s, {} problems by directory: {per_dir:?}",
        named.len(),
        problems.len()
    );
    let unexpected: Vec<_> = problems
        .iter()
        .filter(|(ds1, _)| {
            !MXL_ORPHAN_PRESET_DIRS
                .iter()
                .any(|d| ds1.as_str().starts_with(d))
        })
        .collect();
    for (ds1, why) in unexpected.iter().take(40) {
        println!("  {ds1}: {why}");
    }
    assert!(
        unexpected.is_empty(),
        "{} unexpected problems",
        unexpected.len()
    );
}

#[test]
fn legacy_ini_import_and_agreement() {
    let dir = legacy_dir();
    if !dir_available(&dir) {
        return;
    }
    let Some(install) = open("vanilla", &vanilla_dir(), Profile::vanilla) else {
        return;
    };
    let mut ini_files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ini")))
        .collect();
    ini_files.sort();
    let mut entries = Vec::new();
    for file in &ini_files {
        let text = std::fs::read_to_string(file).unwrap();
        let (parsed, warnings) = parse_legacy_ini(&text);
        assert!(warnings.is_empty(), "{}: {warnings:?}", file.display());
        entries.extend(parsed);
    }
    println!(
        "legacy: {} ini files, {} entries",
        ini_files.len(),
        entries.len()
    );
    assert!(!entries.is_empty());

    let mut catalog = catalog(&install);
    let unmatched: Vec<_> = entries
        .iter()
        .filter(|e| !install.ds1.contains(&e.ds1))
        .collect();
    let matched = catalog.import_legacy(&entries);
    assert_eq!(matched, entries.len() - unmatched.len());
    println!(
        "legacy: {matched}/{} matched a known DS1; unmatched:",
        entries.len()
    );
    for e in &unmatched {
        println!("  {} {} {}", e.lvl_type, e.lvlprest_def, e.ds1);
    }
    assert!(
        matched * 100 >= entries.len() * 95,
        "only {matched} of {} legacy entries matched",
        entries.len()
    );

    // Informative: does the non-override best() pick the ini's (LvlType, Def)?
    let mut agree = 0usize;
    let mut disagree = Vec::new();
    for e in entries.iter().filter(|e| install.ds1.contains(&e.ds1)) {
        let best = catalog
            .lookup(&e.ds1)
            .into_iter()
            .find(|i| i.source != MapInfoSource::Override)
            .unwrap();
        if best.lvl_type == Some(e.lvl_type) && best.lvlprest_def == Some(e.lvlprest_def) {
            agree += 1;
        } else {
            disagree.push(format!(
                "{} ini=({}, {}) ours={}",
                e.ds1,
                e.lvl_type,
                e.lvlprest_def,
                describe(&best)
            ));
        }
    }
    let total = agree + disagree.len();
    println!(
        "legacy: agreement {agree}/{total} = {:.1}%",
        100.0 * agree as f64 / total.max(1) as f64
    );
    for d in disagree.iter().take(60) {
        println!("  differs: {d}");
    }
    // The import itself makes best() follow the ini.
    for e in entries.iter().filter(|e| install.ds1.contains(&e.ds1)) {
        let best = catalog.best(&e.ds1).unwrap();
        assert_eq!(best.source, MapInfoSource::Override);
    }
}
