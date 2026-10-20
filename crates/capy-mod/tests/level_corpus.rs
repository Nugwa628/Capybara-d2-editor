//! Level tools against the real vanilla and Median XL tables
//! (`CAPY_PRIVATE_DIR`, default the main checkout's `testdata\private`).
//! Skipped when the corpus is absent.

mod common;

use capy_mod::add_map::{AddMapInput, AddMode, add_map_to_game};
use capy_mod::change_type::{ChangeTypeInput, change_level_type, level_type_choices};
use capy_mod::level_tables::{LevelTables, TableName};
use capy_mod::type_package::{export_type_package, import_type_package, maps_of_type};
use common::{
    TestResult, best, catalog_with, corpus, load_tables, old_rows_unchanged, read_game_file,
};
use d2_formats::txt::TxtDoc;

const PROFILES: [&str; 2] = ["vanilla", "mxl"];

fn cell(doc: &TxtDoc, r: usize, col: &str) -> String {
    doc.column(col)
        .and_then(|c| doc.cell(r, c))
        .unwrap_or_default()
}

fn num(doc: &TxtDoc, r: usize, col: &str) -> i64 {
    cell(doc, r, col).trim().parse().unwrap_or(0)
}

/// A preset level of the corpus: a LvlPrest row with a LevelId whose level is
/// DrlgType 2, and whose map the catalog resolves from the preset. Returns
/// (ds1 path, level id, size).
fn preset_level(t: &LevelTables, root: &std::path::Path) -> Option<(String, u32, (u32, u32))> {
    for r in 0..t.prest.row_count() {
        let id = num(&t.prest, r, "LevelId");
        let file = cell(&t.prest, r, "File1");
        if id == 0 || file.trim().is_empty() || file == "0" {
            continue;
        }
        let Some(lr) = (0..t.levels.row_count()).find(|&l| num(&t.levels, l, "Id") == id) else {
            continue;
        };
        if num(&t.levels, lr, "DrlgType") != 2 {
            continue;
        }
        let full = format!(r"data\global\tiles\{}", file.replace('/', "\\"));
        if read_game_file(root, &full).is_err() {
            continue;
        }
        let (w, h) = (
            num(&t.levels, lr, "SizeX") + 1,
            num(&t.levels, lr, "SizeY") + 1,
        );
        return Some((full, id as u32, (w as u32, h as u32)));
    }
    None
}

fn tiles_rel(full: &str) -> String {
    full.trim_start_matches(r"data\global\tiles\")
        .replace('\\', "/")
}

#[test]
fn tables_start_as_the_game_reads_them() -> TestResult {
    for p in PROFILES {
        let Some(root) = corpus(p) else { continue };
        let t = load_tables(&root)?;
        // An unchanged round trip is byte-exact.
        assert_eq!(TxtDoc::parse(&t.levels.to_bytes()), t.levels, "{p}");
        // Registering any map first checks Id/Def/Id count up; the corpus passes.
        let (ds1, id, (w, h)) = preset_level(&t, &root).ok_or("no preset level")?;
        let out = add_map_to_game(
            &t,
            &AddMapInput {
                mode: AddMode::Existing,
                level_id: id,
                name: "x".into(),
                map_rel: tiles_rel(&ds1),
                width: w,
                height: h,
                used_dt1s: vec![],
                pop_count: 0,
                pal_act: None,
                new_type: false,
            },
        );
        assert!(out.is_ok(), "{p}: {:?}", out.err());
    }
    Ok(())
}

#[test]
fn add_map_new_level_resolves_through_the_presets_catalog() -> TestResult {
    for p in PROFILES {
        let Some(root) = corpus(p) else { continue };
        let t = load_tables(&root)?;
        let (template_ds1, template_level, (w, h)) =
            preset_level(&t, &root).ok_or("no preset level")?;
        let base = catalog_with(&root, &t, &[], &[&template_ds1])?;
        let info = best(&base, &template_ds1).ok_or("template unresolved")?;
        assert!(!info.dt1_files.is_empty(), "{p}: template has DT1s");
        let used: Vec<String> = info
            .dt1_files
            .iter()
            .map(|g| g.as_str().to_owned())
            .collect();

        // The new map is a copy of the template's DS1 under a new name.
        let new_ds1 = r"data\global\tiles\capy_test\newmap.ds1";
        let bytes = read_game_file(&root, &template_ds1)?;
        for new_type in [false, true] {
            let out = add_map_to_game(
                &t,
                &AddMapInput {
                    mode: AddMode::New,
                    level_id: template_level,
                    name: "Capy Test Map".into(),
                    map_rel: "capy_test/newmap.ds1".into(),
                    width: w,
                    height: h,
                    used_dt1s: used.clone(),
                    pop_count: 2,
                    pal_act: None,
                    new_type,
                },
            )?;
            let new_level = out.new_level_id.ok_or("no new level")?;
            assert_eq!(
                new_level as usize,
                (0..t.levels.row_count())
                    .filter(|&r| !cell(&t.levels, r, "Id").trim().is_empty())
                    .count(),
                "{p}: appended as the next record"
            );

            // Unchanged rows stay byte-identical; only the three tables grew.
            assert!(old_rows_unchanged(&t.levels, &out.tables.levels), "{p}");
            assert!(old_rows_unchanged(&t.prest, &out.tables.prest), "{p}");
            assert!(old_rows_unchanged(&t.types, &out.tables.types), "{p}");
            assert!(
                out.touched.contains(&TableName::Levels)
                    && out.touched.contains(&TableName::LvlPrest)
            );
            assert!(!out.changes.is_empty());

            let cat = catalog_with(&root, &out.tables, &[(new_ds1, bytes.clone())], &[new_ds1])?;
            let got = best(&cat, new_ds1).ok_or("new map unresolved")?;
            assert_eq!(got.level_id, Some(new_level), "{p}");
            assert_eq!(got.lvlprest_def, Some(out.preset_def), "{p}");
            assert_eq!(got.lvl_type, Some(out.type_id), "{p}");
            assert_eq!(
                got.dt1_files, info.dt1_files,
                "{p}: same DT1s in the same order"
            );
            if new_type {
                assert!(out.own_type && out.type_id != info.lvl_type.unwrap_or(u32::MAX));
            }
            // Act 5 level, sized from the DS1.
            let lr = (0..out.tables.levels.row_count())
                .find(|&r| num(&out.tables.levels, r, "Id") == i64::from(new_level))
                .ok_or("level row")?;
            assert_eq!(cell(&out.tables.levels, lr, "Act"), "4");
            assert_eq!(num(&out.tables.levels, lr, "SizeX"), i64::from(w) - 1);
            assert_eq!(cell(&out.tables.levels, lr, "LevelName"), "Capy Test Map");

            // Adding the same map as a new level again is refused (idempotence guard).
            let again = add_map_to_game(
                &out.tables,
                &AddMapInput {
                    mode: AddMode::New,
                    level_id: template_level,
                    name: "Capy Test Map".into(),
                    map_rel: "capy_test/newmap.ds1".into(),
                    width: w,
                    height: h,
                    used_dt1s: used.clone(),
                    pop_count: 2,
                    pal_act: None,
                    new_type,
                },
            );
            assert!(again.is_err(), "{p}");
            // Pointing the existing level at it a second time changes nothing.
            let mut input = AddMapInput {
                mode: AddMode::Existing,
                level_id: new_level,
                name: "Capy Test Map".into(),
                map_rel: "capy_test/newmap.ds1".into(),
                width: w,
                height: h,
                used_dt1s: used.clone(),
                pop_count: 2,
                pal_act: None,
                new_type: false,
            };
            let second = add_map_to_game(&out.tables, &input)?;
            assert!(second.changes.is_empty(), "{p}: {:?}", second.changes);
            assert_eq!(second.tables, out.tables, "{p}");
            input.pop_count = 0;
        }
    }
    Ok(())
}

#[test]
fn add_map_existing_is_idempotent_on_the_games_own_levels() -> TestResult {
    for p in PROFILES {
        let Some(root) = corpus(p) else { continue };
        let t = load_tables(&root)?;
        let (ds1, id, (w, h)) = preset_level(&t, &root).ok_or("no preset level")?;
        let base = catalog_with(&root, &t, &[], &[&ds1])?;
        let info = best(&base, &ds1).ok_or("unresolved")?;
        let used: Vec<String> = info
            .dt1_files
            .iter()
            .map(|g| g.as_str().to_owned())
            .collect();
        let out = add_map_to_game(
            &t,
            &AddMapInput {
                mode: AddMode::Existing,
                level_id: id,
                name: "x".into(),
                map_rel: tiles_rel(&ds1),
                width: w,
                height: h,
                used_dt1s: used,
                pop_count: 0,
                pal_act: None,
                new_type: false,
            },
        )?;
        assert!(out.changes.is_empty(), "{p}: {:?}", out.changes);
        assert_eq!(out.tables, t, "{p}");
        assert_eq!(out.tables.levels.to_bytes(), t.levels.to_bytes());
    }
    Ok(())
}

#[test]
fn change_level_type_resolves_to_the_new_type() -> TestResult {
    for p in PROFILES {
        let Some(root) = corpus(p) else { continue };
        let t = load_tables(&root)?;
        let (ds1, id, (w, h)) = preset_level(&t, &root).ok_or("no preset level")?;
        let base = catalog_with(&root, &t, &[], &[&ds1])?;
        let info = best(&base, &ds1).ok_or("unresolved")?;
        let used: Vec<String> = info
            .dt1_files
            .iter()
            .map(|g| g.as_str().to_owned())
            .collect();
        let map_rel = tiles_rel(&ds1);
        let _ = (id, w, h);

        let choices = level_type_choices(&t, &map_rel, &used, None)?;
        assert!(!choices.is_empty(), "{p}");
        let current = info.lvl_type.ok_or("type")?;

        // Using another type that already holds every library: the level moves, the map keeps its DT1s.
        if let Some(c) = choices
            .iter()
            .find(|c| c.id != current && c.copy_only.is_none() && c.files > 0)
        {
            let out = change_level_type(
                &t,
                &ChangeTypeInput {
                    map_rel: map_rel.clone(),
                    level_id: None,
                    target_type: c.id,
                    copy: false,
                    used_dt1s: used.clone(),
                    copy_name: None,
                },
            )?;
            assert_eq!(out.type_id, c.id);
            assert!(!out.copied || c.copy_only.is_some());
            assert!(old_rows_unchanged(&t.types, &out.tables.types), "{p}");
            assert_eq!(out.tables.levels.row_count(), t.levels.row_count());
            let changed = (0..t.levels.row_count())
                .filter(|&r| t.levels.row_cells(r) != out.tables.levels.row_cells(r))
                .count();
            assert_eq!(changed, 1, "{p}: only the level's own row changes");
            let cat = catalog_with(&root, &out.tables, &[], &[&ds1])?;
            let got = best(&cat, &ds1).ok_or("unresolved after")?;
            assert_eq!(got.lvl_type, Some(c.id), "{p}");
            for d in &info.dt1_files {
                assert!(got.dt1_files.contains(d), "{p}: {d:?} still loaded");
            }
            // Doing it again is refused: the level already uses it.
            assert!(
                change_level_type(
                    &out.tables,
                    &ChangeTypeInput {
                        map_rel: map_rel.clone(),
                        level_id: None,
                        target_type: c.id,
                        copy: false,
                        used_dt1s: used.clone(),
                        copy_name: None,
                    },
                )
                .is_err()
            );
        }

        // A copy: a new type of its own with exactly the map's libraries.
        let target = choices.first().ok_or("choice")?.id;
        let out = change_level_type(
            &t,
            &ChangeTypeInput {
                map_rel,
                level_id: None,
                target_type: target,
                copy: true,
                used_dt1s: used.clone(),
                copy_name: Some("Capy Copy".into()),
            },
        )?;
        assert!(out.copied);
        let cat = catalog_with(&root, &out.tables, &[], &[&ds1])?;
        let got = best(&cat, &ds1).ok_or("unresolved after copy")?;
        assert_eq!(got.lvl_type, Some(out.type_id), "{p}");
        assert_eq!(got.dt1_files, info.dt1_files, "{p}");
        assert!(old_rows_unchanged(&t.types, &out.tables.types), "{p}");
    }
    Ok(())
}

#[test]
fn type_package_round_trips_between_corpora_and_is_idempotent() -> TestResult {
    let (Some(van), Some(mxl)) = (corpus("vanilla"), corpus("mxl")) else {
        return Ok(());
    };
    let v = load_tables(&van)?;
    let m = load_tables(&mxl)?;
    // Same-corpus: export then import changes nothing.
    let mut tried = 0;
    for ty in 0..40u32 {
        if maps_of_type(&v, ty).is_empty() {
            continue;
        }
        let pkg = export_type_package(&v, ty)?;
        assert_eq!(pkg.type_id, ty);
        if pkg.dt1s.is_empty() {
            continue;
        }
        let json = serde_json::to_string(&pkg)?;
        assert_eq!(
            serde_json::from_str::<capy_mod::type_package::TypePackage>(&json)?,
            pkg
        );
        let same = import_type_package(&v, &pkg)?;
        assert!(same.changes.is_empty(), "type {ty}: {:?}", same.changes);
        assert_eq!(same.tables, v);

        // Into Median XL: the type, its levels and presets arrive, and again is a no-op.
        let Ok(first) = import_type_package(&m, &pkg) else {
            continue; // a type with no room for its libraries is refused, not half-written
        };
        let again = import_type_package(&first.tables, &pkg)?;
        assert!(again.changes.is_empty(), "type {ty}: {:?}", again.changes);
        // Every map of the package now loads at least the package's DT1s.
        let map = format!(r"data\global\tiles\{}", pkg.maps[0].replace('/', "\\"));
        let cat = catalog_with(&mxl, &first.tables, &[], &[&map])?;
        let got = best(&cat, &map).ok_or("map unresolved in the importer")?;
        let have: Vec<String> = got
            .dt1_files
            .iter()
            .map(|g| g.as_str().to_lowercase())
            .collect();
        // The DT1s the package's own preset for this map selects (its Dt1Mask bits).
        let preset = pkg
            .presets
            .iter()
            .find(|r| {
                r.value("File1")
                    .replace('\\', "/")
                    .eq_ignore_ascii_case(&pkg.maps[0])
            })
            .ok_or("package preset for the map")?;
        let pmask: u32 = preset.value("Dt1Mask").trim().parse().unwrap_or(0);
        let want = pkg
            .dt1s
            .iter()
            .filter(|d| (pmask >> (d.slot - 1)) & 1 == 1)
            .map(|d| format!(r"data\global\tiles\{}", d.path.replace('/', "\\")).to_lowercase());
        let in_type = want.filter(|w| !have.contains(w)).count();
        assert_eq!(
            in_type, 0,
            "type {ty}: package DT1s all loaded by its map; got {got:?}; imported type {}",
            first.type_id
        );
        tried += 1;
        if tried >= 3 {
            break;
        }
    }
    assert!(tried > 0, "at least one type imported into Median XL");
    Ok(())
}
