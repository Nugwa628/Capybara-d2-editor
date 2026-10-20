#![allow(clippy::unwrap_used, clippy::panic)]
//! Checker, fix and automap behaviour on the private corpus
//! (`CAPY_PRIVATE_DIR/extracted/{vanilla,mxl}`); skipped when it is absent.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use capy_mod::automap::{self, AutomapTable, Scope};
use capy_mod::compat::{
    Dt1Candidate, Env, Finding, FixAction, LoadedDt1, MapInput, Severity, Tables, check_map,
};
use d2_formats::ds1::Ds1;
use d2_formats::dt1::{Dt1, TileKey};
use d2_formats::txt::TxtDoc;

fn root(kind: &str) -> Option<PathBuf> {
    let base = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../testdata/private"));
    let p = base.join("extracted").join(kind);
    if p.is_dir() {
        Some(p)
    } else {
        println!("skipped: {} missing", p.display());
        None
    }
}

fn read_table(root: &Path, name: &str) -> Option<TxtDoc> {
    std::fs::read(root.join("data/global/excel").join(name))
        .ok()
        .map(|b| TxtDoc::parse(&b))
}

fn tables_of(root: &Path) -> Tables {
    Tables {
        prest: read_table(root, "lvlprest.txt"),
        levels: read_table(root, "levels.txt"),
        types: read_table(root, "lvltypes.txt"),
        automap: read_table(root, "automap.txt"),
    }
}

struct Dt1Cache {
    root: PathBuf,
    cache: HashMap<String, Option<HashSet<TileKey>>>,
}

impl Dt1Cache {
    fn tiles(&mut self, rel: &str) -> Option<HashSet<TileKey>> {
        let key = rel.replace('\\', "/").to_lowercase();
        let root = self.root.clone();
        self.cache
            .entry(key.clone())
            .or_insert_with(|| {
                let bytes = std::fs::read(root.join("data/global/tiles").join(&key)).ok()?;
                let dt1 = Dt1::parse(&bytes).ok()?;
                Some(dt1.tiles.iter().filter_map(|t| t.key()).collect())
            })
            .clone()
    }
}

struct Case {
    rel: String,
    ds1: Ds1,
    loaded: Vec<LoadedDt1>,
}

/// Maps built from LvlPrest rows with a LevelId: the DS1 and the libraries
/// the game loads for it (level type files selected by Dt1Mask).
fn cases(root: &Path, tables: &Tables, limit: usize) -> Vec<Case> {
    let (prest, levels, types) = (
        tables.prest.as_ref().unwrap(),
        tables.levels.as_ref().unwrap(),
        tables.types.as_ref().unwrap(),
    );
    let mut dt1s = Dt1Cache {
        root: root.to_path_buf(),
        cache: HashMap::new(),
    };
    let mut out = Vec::new();
    for pr in 0..prest.row_count() {
        let get = |d: &TxtDoc, r: usize, c: &str| {
            d.column(c)
                .and_then(|i| d.cell(r, i))
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        let level_id: i64 = get(prest, pr, "LevelId").parse().unwrap_or(0);
        let file1 = get(prest, pr, "File1");
        if level_id <= 0 || file1.is_empty() || file1 == "0" {
            continue;
        }
        let rel = file1.replace('\\', "/");
        let Ok(bytes) = std::fs::read(root.join("data/global/tiles").join(&rel)) else {
            continue;
        };
        let Ok(ds1) = Ds1::parse(&bytes) else {
            continue;
        };
        // Level -> level type -> files by mask.
        let data: Vec<usize> = (0..levels.row_count())
            .filter(|&r| !get(levels, r, "Id").is_empty())
            .collect();
        let Some(&lr) = data.get(level_id as usize) else {
            continue;
        };
        let tid: usize = get(levels, lr, "LevelType").parse().unwrap_or(0);
        let tdata: Vec<usize> = (0..types.row_count())
            .filter(|&r| !get(types, r, "Id").is_empty())
            .collect();
        let Some(&tr) = tdata.get(tid) else { continue };
        let mask: u64 = get(prest, pr, "Dt1Mask").parse().unwrap_or(0);
        let mut loaded = Vec::new();
        for i in 0..32 {
            let f = get(types, tr, &format!("File {}", i + 1));
            if f.is_empty() || f == "0" || (mask >> i) & 1 == 0 {
                continue;
            }
            let tiles = dt1s.tiles(&f);
            loaded.push(LoadedDt1 {
                path: f,
                found: tiles.is_some(),
                tiles: tiles.unwrap_or_default(),
            });
        }
        out.push(Case { rel, ds1, loaded });
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn run(case: &Case, tables: &Tables) -> Vec<Finding> {
    // Which loaded library each placed tile comes from (the first that has it).
    let mut used: Vec<(String, usize)> = Vec::new();
    for (_, k) in capy_mod::compat::placed_tiles(&case.ds1) {
        let hit = case.loaded.iter().find(|l| {
            l.tiles.contains(&k)
                || (k.orientation == 3
                    && l.tiles.contains(&TileKey {
                        orientation: 4,
                        ..k
                    }))
                || (k.orientation == 4
                    && l.tiles.contains(&TileKey {
                        orientation: 3,
                        ..k
                    }))
        });
        if let Some(l) = hit {
            match used.iter_mut().find(|u| u.0 == l.path) {
                Some(u) => u.1 += 1,
                None => used.push((l.path.clone(), 1)),
            }
        }
    }
    let candidates: Vec<Dt1Candidate> = Vec::new();
    let input = MapInput {
        path: &case.rel,
        ds1: &case.ds1,
        loaded: &case.loaded,
        candidates: &candidates,
        used_dt1s: &used,
    };
    let env = Env::default();
    check_map(&input, tables, &env)
}

/// Five shipped maps (CryptCountess1, Duriel, Temple0, MephComp, Fortress) place
/// a few tiles with main index 49 that no library the level loads has; the only
/// library holding such tiles is the unused expansion/siege/temptile.dt1. They
/// are leftovers (invisible in game), so the "no graphic" error is true but
/// harmless there. Everything else missing would still fail the test.
fn legacy_only(c: &Case) -> bool {
    let have: HashSet<TileKey> = c
        .loaded
        .iter()
        .flat_map(|l| l.tiles.iter().copied())
        .collect();
    capy_mod::compat::placed_tiles(&c.ds1)
        .into_iter()
        .filter(|(_, k)| !have.contains(k))
        .all(|(_, k)| k.main == 49)
}

#[test]
fn vanilla_maps_have_no_false_errors() {
    let Some(root) = root("vanilla") else { return };
    let tables = tables_of(&root);
    let cs = cases(&root, &tables, usize::MAX);
    assert!(
        cs.len() > 20,
        "expected the vanilla preset levels, got {}",
        cs.len()
    );
    let mut errors = Vec::new();
    let mut legacy = 0;
    let mut other = HashMap::<String, usize>::new();
    for c in &cs {
        for f in run(c, &tables) {
            match f.severity {
                Severity::Error
                    if f.title.ends_with("placed tiles have no graphic") && legacy_only(c) =>
                {
                    legacy += 1;
                }
                Severity::Error => errors.push(format!("{}: {}", c.rel, f.title)),
                Severity::Warning | Severity::Info => {
                    *other
                        .entry(format!("{:?}: {}", f.severity, f.key()))
                        .or_default() += 1;
                }
                Severity::Ok => {}
            }
        }
    }
    let mut other: Vec<_> = other.into_iter().collect();
    other.sort();
    println!("{} vanilla maps checked; non-error findings:", cs.len());
    for (k, n) in other {
        println!("  {n:4} x {k}");
    }
    assert_eq!(legacy, 5, "the five maps with leftover main-49 tiles");
    assert!(
        errors.is_empty(),
        "false errors on vanilla maps:\n{}",
        errors.join("\n")
    );
}

/// Applies every edit fix of every finding twice and checks the second pass
/// changes nothing.
fn fixes_are_idempotent(c: &Case, tables: &Tables) -> usize {
    let findings = run(c, tables);
    let mut n = 0;
    for f in &findings {
        for fix in &f.fixes {
            if !matches!(fix.action, FixAction::Edits(_)) {
                continue;
            }
            let mut t = tables.clone();
            let mut ds1 = c.ds1.clone();
            fix.apply(&mut t, Some(&mut ds1)).unwrap();
            let once = (
                [&t.prest, &t.levels, &t.types, &t.automap]
                    .map(|d| d.as_ref().map(TxtDoc::to_bytes)),
                ds1.to_bytes().unwrap(),
            );
            let changed_again = fix.apply(&mut t, Some(&mut ds1)).unwrap();
            let twice = (
                [&t.prest, &t.levels, &t.types, &t.automap]
                    .map(|d| d.as_ref().map(TxtDoc::to_bytes)),
                ds1.to_bytes().unwrap(),
            );
            assert!(
                !changed_again,
                "fix reported a change the second time: {}",
                fix.label
            );
            assert!(once == twice, "fix not idempotent: {}", fix.label);
            n += 1;
        }
    }
    n
}

#[test]
fn fixes_are_idempotent_on_vanilla_and_mxl() {
    let mut total = 0;
    for kind in ["vanilla", "mxl"] {
        let Some(root) = root(kind) else { continue };
        let tables = tables_of(&root);
        for c in cases(&root, &tables, 80) {
            total += fixes_are_idempotent(&c, &tables);
        }
    }
    println!("{total} fixes applied twice");
}

#[test]
fn mxl_findings_are_reported_without_panics() {
    let Some(root) = root("mxl") else { return };
    let tables = tables_of(&root);
    let cs = cases(&root, &tables, 150);
    let mut counts = HashMap::<String, usize>::new();
    for c in &cs {
        for f in run(c, &tables) {
            if f.severity == Severity::Error {
                *counts.entry(f.key()).or_default() += 1;
            }
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort();
    println!("{} mxl maps; error kinds:", cs.len());
    for (k, n) in v {
        println!("  {n:4} x {k}");
    }
}

#[test]
fn broken_tables_are_found_and_fixed() {
    let Some(root) = root("vanilla") else { return };
    let tables = tables_of(&root);
    let cs = cases(&root, &tables, usize::MAX);
    // A non-town preset level (Act 1 Cave): AutoMap 1, wrong size, bad layer.
    let c = cs
        .iter()
        .find(|c| c.rel.to_lowercase().contains("act1/cave"))
        .or(cs.get(5))
        .unwrap();
    let mut t = tables.clone();
    let prest = t.prest.as_mut().unwrap();
    let pr = (0..prest.row_count())
        .find(|&r| {
            let i = prest.column("File1").unwrap();
            prest.cell(r, i).unwrap_or_default().replace('\\', "/") == c.rel
        })
        .unwrap();
    let col = prest.column("AutoMap").unwrap();
    prest.set_cell(pr, col, "1").unwrap();
    let before = run(c, &t);
    let hit = before
        .iter()
        .find(|f| f.title.contains("AutoMap is 1"))
        .expect("AutoMap 1 reported");
    assert_eq!(hit.severity, Severity::Error);
    for fix in &hit.fixes {
        fix.apply(&mut t, None).unwrap();
    }
    let after = run(c, &t);
    assert!(!after.iter().any(|f| f.title.contains("AutoMap is 1")));
    // Bytes outside that cell are untouched.
    let orig = tables.prest.as_ref().unwrap().to_bytes();
    assert_eq!(t.prest.as_ref().unwrap().to_bytes(), orig);
}

#[test]
fn automap_edits_keep_unchanged_rows_byte_identical() {
    let Some(root) = root("vanilla") else { return };
    let bytes = std::fs::read(root.join("data/global/excel/automap.txt")).unwrap();
    let lines = |b: &[u8]| -> Vec<Vec<u8>> {
        b.split_inclusive(|&c| c == b'\n')
            .map(<[u8]>::to_vec)
            .collect()
    };
    let original = lines(&bytes);
    let doc = TxtDoc::parse(&bytes);
    assert_eq!(doc.to_bytes(), bytes);
    let table = AutomapTable::parse(&doc);
    assert!(table.rules.len() > 1000);
    assert!(table.unknown_levels().is_empty());
    assert!(automap::safety::invalid_rows(&doc).unwrap().is_empty());

    // New rule for an unused tile of a level: exactly one new line appears.
    let mut d = doc.clone();
    let s = automap::set_cel(&mut d, "1 Town", 1, 40, 7, 5, Scope::Sequence).unwrap();
    assert_eq!(s.changes.len(), 1);
    let new = lines(&d.to_bytes());
    assert_eq!(new.len(), original.len() + 1);
    let kept: Vec<_> = new
        .iter()
        .filter(|l| !l.starts_with(b"1 Town\twl\t40\t7\t7"))
        .cloned()
        .collect();
    assert_eq!(kept, original);
    // Idempotent.
    let again = automap::set_cel(&mut d, "1 Town", 1, 40, 7, 5, Scope::Sequence).unwrap();
    assert!(again.changes.is_empty());
    // Clearing it removes exactly that line again.
    automap::set_cel(&mut d, "1 Town", 1, 40, 7, -1, Scope::Sequence).unwrap();
    assert_eq!(d.to_bytes(), bytes);

    // Splitting a range changes only the rows involved.
    let row = table
        .rules
        .iter()
        .find(|r| r.start >= 0 && r.end > r.start + 1 && r.style >= 0)
        .unwrap()
        .clone();
    let mid = row.start + 1;
    let orientation = automap::AUTOMAP_CODES
        .iter()
        .position(|c| *c == row.code)
        .unwrap();
    let mut d = doc.clone();
    automap::set_cel(
        &mut d,
        &row.level,
        orientation,
        row.style,
        mid,
        -1,
        Scope::Sequence,
    )
    .unwrap();
    let new = lines(&d.to_bytes());
    assert_eq!(new.len(), original.len() + 1);
    let differing = new.iter().filter(|l| !original.contains(l)).count();
    assert_eq!(differing, 2, "only the two halves differ");
    let table2 = AutomapTable::parse(&d);
    assert!(
        table2
            .find_rule(&row.level, orientation, row.style, mid)
            .is_none()
    );
    assert!(
        table2
            .find_rule(&row.level, orientation, row.style, row.start)
            .is_some()
    );
    assert!(
        table2
            .find_rule(&row.level, orientation, row.style, row.end)
            .is_some()
    );
    // The result still passes the save-time check.
    automap::safety::validate_save("data\\global\\excel\\AutoMap.txt", &d.to_bytes()).unwrap();
}
