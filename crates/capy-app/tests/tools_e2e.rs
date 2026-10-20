//! End-to-end checks of the tile-library, game-data and package tools
//! (P11-P13): the real exe runs `CAPY_SCRIPT` lines (`tool ...`) against the
//! vanilla install and the results are read back from the profile's saves.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

const MAP: &str = "data\\global\\tiles\\act1\\town\\townn1.ds1";
const DT1: &str = "data\\global\\tiles\\act1\\town\\floor.dt1";

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn enabled() -> bool {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return false;
    }
    if vanilla_dir().is_none() {
        eprintln!("skipped: vanilla install not found");
        return false;
    }
    true
}

/// Runs the exe on `script` with `home`; returns (exit code, stderr).
fn run_script(home: &Path, script: &str) -> (Option<i32>, String) {
    let file = home.join("script.txt");
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(&file, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_SCALE_FACTOR", "1")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn saves(home: &Path) -> PathBuf {
    home.join("capybara").join("saves").join("vanilla")
}

fn changed(home: &Path) -> String {
    std::fs::read_to_string(saves(home).join("CHANGED.txt")).unwrap_or_default()
}

#[test]
fn recolouring_a_dt1_lands_in_the_saves_with_a_changed_entry() {
    if !enabled() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         tool dt1-open {DT1}\n\
         tool dt1-assert-dirty false\n\
         tool dt1-recolor 90 all\n\
         tool dt1-assert-dirty true\n\
         tool dt1-save\n\
         tool assert-saved {DT1}\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let file = saves(home.path()).join(DT1);
    let bytes = std::fs::read(&file).expect("the DT1 is in the saves");
    let dt1 = d2_formats::dt1::Dt1::parse(&bytes).expect("the saved DT1 parses");
    assert!(!dt1.tiles.is_empty());
    assert!(
        changed(home.path())
            .to_ascii_lowercase()
            .contains("floor.dt1"),
        "CHANGED.txt: {}",
        changed(home.path())
    );
}

#[test]
fn a_map_added_to_the_game_is_listed_and_opens_again() {
    if !enabled() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let new_map = "data\\global\\tiles\\act1\\capytest\\newmap.ds1";
    let script = format!(
        "open {MAP}\n\
         tool add-to-game new 1 act1/capytest/newmap.ds1 Capy Test Map\n\
         tool wait-catalog\n\
         tool assert-saved {new_map}\n\
         tool assert-saved data\\global\\excel\\levels.txt\n\
         tool assert-saved data\\global\\excel\\lvlprest.txt\n\
         tool assert-map-listed {new_map}\n\
         open {new_map}\n\
         tool wait-catalog\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let list = changed(home.path()).to_ascii_lowercase();
    for want in ["newmap.ds1", "levels.txt", "lvlprest.txt"] {
        assert!(list.contains(want), "{want} not in CHANGED.txt:\n{list}");
    }
}

#[test]
fn a_preset_is_saved_placed_and_undone() {
    if !enabled() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         select-box floor0 5 5 8 8\n\
         tool preset-save Patch Test\n\
         assert-dirty false\n\
         tool preset-place 20 20 Patch\n\
         assert-dirty true\n\
         undo\n\
         assert-dirty false\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let dir = saves(home.path()).join("data\\ds1studio\\presets");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the presets folder")
        .flatten()
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(changed(home.path()).contains("ds1studio"));
}

#[test]
fn collision_flags_are_painted_and_saved() {
    if !enabled() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         tool dt1-open {DT1}\n\
         tool dt1-flags set 05 0,1,2\n\
         tool dt1-assert-dirty true\n\
         tool dt1-save\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let bytes = std::fs::read(saves(home.path()).join(DT1)).expect("saved DT1");
    let dt1 = d2_formats::dt1::Dt1::parse(&bytes).unwrap();
    assert_eq!(&dt1.tiles[0].subtile_flags[..3], &[5, 5, 5]);
}

#[test]
fn a_table_cell_edit_is_saved() {
    if !enabled() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let script = "tool table-open lvlprest.txt\n\
         tool table-set 1 Pops 7\n\
         tool table-assert 1 Pops 7\n\
         tool table-save\n\
         tool assert-saved data\\global\\excel\\lvlprest.txt\n";
    let (code, stderr) = run_script(home.path(), script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let saved =
        std::fs::read(saves(home.path()).join("data\\global\\excel\\lvlprest.txt")).unwrap();
    let doc = d2_formats::txt::TxtDoc::parse(&saved);
    let col = doc.column("Pops").expect("Pops column");
    assert_eq!(doc.cell(1, col).as_deref(), Some("7"));
}

#[test]
fn the_compatibility_check_runs_on_a_vanilla_map() {
    if !enabled() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let script = format!("open {MAP}\ntool wait-catalog\ntool compat\n");
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    assert!(stderr.contains("compat:"), "no result line:\n{stderr}");
}

#[test]
fn a_map_package_exports_and_imports_into_another_profile_home() {
    if !enabled() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let zip = work.path().join("town.zip");
    let (code, stderr) = run_script(
        &work.path().join("a"),
        &format!(
            "open {MAP}\ntool wait-catalog\ntool package-export {}\n",
            zip.display()
        ),
    );
    assert_eq!(code, Some(0), "export failed:\n{stderr}");
    assert!(zip.is_file(), "no package written");
    let bytes = std::fs::read(&zip).unwrap();
    let pkg = capy_history::read_package(&bytes).expect("the package reads back");
    assert_eq!(pkg.manifest.map, MAP.replace('\\', "/"));
    let home_b = work.path().join("b");
    let (code, stderr) = run_script(
        &home_b,
        &format!(
            "tool package-import {}\ntool assert-saved {MAP}\n",
            zip.display()
        ),
    );
    assert_eq!(code, Some(0), "import failed:\n{stderr}");
    // The imported map is the exported one, byte for byte.
    let imported = std::fs::read(saves(&home_b).join(MAP)).unwrap();
    let in_zip = pkg
        .files
        .iter()
        .find(|(p, _)| p.as_str() == MAP)
        .map(|(_, b)| b.clone())
        .expect("map in the package");
    assert_eq!(imported, in_zip);
}
