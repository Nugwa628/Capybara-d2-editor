//! End-to-end check of "Add to this map": the real exe runs `CAPY_SCRIPT`
//! lines against the vanilla install (tile keys of barset.dt1 are read from
//! the private corpus), adds tiles to a town map, paints one, saves, reopens
//! and undoes. Needs a window and the install, so it only runs with
//! `CAPY_GPU_TESTS=1`; `CAPY_VANILLA_DIR` / `CAPY_PRIVATE_DIR` override the
//! install and corpus folders.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;

const MAP: &str = "data\\global\\tiles\\act1\\town\\townn1.ds1";
const SRC: &str = "data\\global\\tiles\\act1\\barracks\\barset.dt1";

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn corpus_barset() -> Option<Dt1> {
    let root = std::env::var_os("CAPY_PRIVATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\zffit\Capybara-d2-editor\testdata\private"));
    let p = root.join("extracted").join("vanilla").join(SRC);
    Dt1::parse(&std::fs::read(p).ok()?).ok()
}

fn enabled() -> bool {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return false;
    }
    vanilla_dir().is_some()
}

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

fn read_saved(home: &Path, game: &str) -> Vec<u8> {
    std::fs::read(saves(home).join(game)).unwrap_or_else(|e| panic!("{game}: {e}"))
}

/// Tile key of the first tile of `barset.dt1` with this orientation.
fn key_of(dt1: &Dt1, orientation: i32) -> Option<String> {
    let t = dt1.tiles.iter().find(|t| t.orientation == orientation)?;
    Some(format!(
        "{}#{}/{}/{}/0",
        SRC.to_ascii_lowercase(),
        t.orientation,
        t.main_index,
        t.sub_index
    ))
}

fn txt_cell(bytes: &[u8], row_name: &str, col: &str) -> String {
    let doc = d2_formats::txt::TxtDoc::parse(bytes);
    let c = doc.column(col).unwrap_or_else(|| panic!("no column {col}"));
    let nc = doc.column("Name").unwrap();
    (0..doc.row_count())
        .find(|&r| doc.cell(r, nc).as_deref() == Some(row_name))
        .and_then(|r| doc.cell(r, c))
        .unwrap_or_default()
}

#[test]
fn add_to_this_map_registers_paints_reloads_and_undoes() {
    if !enabled() {
        return;
    }
    let Some(barset) = corpus_barset() else {
        eprintln!("skipped: private corpus not found");
        return;
    };
    let orients: std::collections::BTreeSet<i32> =
        barset.tiles.iter().map(|t| t.orientation).collect();
    let floor = key_of(&barset, 1).unwrap_or_else(|| panic!("no wall in barset: {orients:?}"));
    let wall = key_of(&barset, 2).expect("a second wall in barset");
    let home = tempfile::tempdir().unwrap();

    // 1. add a floor and a wall, paint the floor, save
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool addmap {floor} {wall}\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 3 3\n\
         save\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "add failed:\n{stderr}");

    let types = read_saved(home.path(), "data\\global\\excel\\lvltypes.txt");
    let prest = read_saved(home.path(), "data\\global\\excel\\lvlprest.txt");
    let doc = d2_formats::txt::TxtDoc::parse(&types);
    let (slot_col, own_rel) = (0..doc.row_count())
        .flat_map(|r| (0..doc.headers().len()).map(move |c| (r, c)))
        .filter_map(|(r, c)| doc.cell(r, c).map(|v| (c, v)))
        .find(|(_, v)| v.to_ascii_lowercase().ends_with("_custom.dt1"))
        .expect("a File slot lists the own-tiles DT1");
    let slot: u32 = doc.headers()[slot_col]
        .trim_start_matches("File")
        .trim()
        .parse()
        .unwrap();
    let own_game = format!("data\\global\\tiles\\{}", own_rel.replace('/', "\\"));
    let own = Dt1::parse(&read_saved(home.path(), &own_game)).unwrap();
    assert!(own.tiles.len() >= 2, "{} tiles", own.tiles.len());
    assert!(
        txt_cell(&prest, "Act 1 - Town 1", "Dt1Mask")
            .parse::<u32>()
            .is_ok_and(|m| m & (1 << (slot - 1)) != 0),
        "Dt1Mask"
    );

    // the DS1 lists the DT1 and the painted cell holds the floor's new number
    let ds1 = Ds1::parse(&read_saved(home.path(), MAP)).unwrap();
    assert!(
        ds1.files.iter().any(|f| String::from_utf8_lossy(f)
            .to_ascii_lowercase()
            .contains("_custom.tg1")),
        "DS1 file list"
    );
    let floor_tile = own.tiles.iter().find(|t| t.orientation == 1).unwrap();
    let cell = ds1.walls[0].tiles.get(3, 3).unwrap();
    assert_eq!(
        (i32::from(cell.main_index()), i32::from(cell.sub_index())),
        (floor_tile.main_index, floor_tile.sub_index)
    );

    // 2. reopen: the catalog loads the DT1 for the map, so the tiles are no longer missing
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool addmap-assert-loaded {own_game} true\n\
         assert-dirty false\n"
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "reopen failed:\n{stderr}");

    // 3. a fresh profile: add, paint, undo both; the map is as it was
    let fresh = tempfile::tempdir().unwrap();
    let script = format!(
        "open {MAP}\n\
         tool wait-catalog\n\
         tool addmap {floor}\n\
         tool addmap-confirm\n\
         tool wait-catalog\n\
         paint-brush 3 3\n\
         assert-dirty true\n\
         undo\n\
         undo\n\
         assert-dirty false\n"
    );
    let (code, stderr) = run_script(fresh.path(), &script);
    assert_eq!(code, Some(0), "undo failed:\n{stderr}");
}
