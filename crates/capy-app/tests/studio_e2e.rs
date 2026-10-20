//! End-to-end checks of the ds1-studio parity tools and views (P9/P10) on
//! vanilla `townn1.ds1`: Rectangle and Fill area paint/erase with undo, the
//! random mix, Find & replace, resizing by dragging a map edge, New map,
//! view-mode cycling (Tab, Shift+Tab, F1-F6), PNG export, PrintScreen and
//! the walkable-area badge against `capy_render::walk::walkable_area`.
//!
//! Needs a window (GPU) and the vanilla install: runs only with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_render::TileSet;
use d2_assets::{GamePath, MapCatalog, Profile};

const MAP: &str = "data\\global\\tiles\\act1\\town\\townn1.ds1";

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn gpu_tests_enabled() -> bool {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return false;
    }
    true
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

/// ds1-studio's walkable area of townn1 as `capy_render` computes it with
/// the tile set the catalog picks (what the badge must show).
fn expected_walkable(vanilla: &Path) -> f64 {
    let saves = tempfile::tempdir().unwrap();
    let profile = Profile::vanilla(vanilla);
    let version = profile.game_version;
    let overlay = profile.open(saves.path()).unwrap();
    let path = GamePath::new(MAP);
    let (catalog, _) = MapCatalog::build(&overlay, version, std::slice::from_ref(&path)).unwrap();
    let info = catalog.best(&path).expect("townn1 map info");
    let (tiles, _) = TileSet::load(&overlay, &info.dt1_files).unwrap();
    let ds1 = d2_formats::ds1::Ds1::parse(&overlay.read_bytes(&path).unwrap().unwrap()).unwrap();
    capy_render::walk::walkable_area(&ds1, &tiles)
}

#[test]
fn map_editing_tools_and_view_modes_work_end_to_end() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let area = expected_walkable(&vanilla);
    eprintln!("capy-render walkable area of townn1: {area:.2}");
    assert!(area > 0.0);
    let home = tempfile::tempdir().unwrap();
    let out = home.path().join("out");
    let sel_png = out.join("selection.png");
    let map_png = out.join("map.png");
    let script = format!(
        "open {MAP}\n\
         assert-walkable {area} 0.5\n\
         # Rectangle (U): paint, then erase, each one undo step\n\
         press U\n\
         set-brush floor0 0x017000c2\n\
         paint-rect floor0 2 2 5 4\n\
         assert-tile floor0 2 2 23 0\n\
         assert-tile floor0 5 4 23 0\n\
         # Fill area (L): the connected 23/0 block becomes 24/0\n\
         press L\n\
         set-brush floor0 0x018000c2\n\
         fill floor0 3 3\n\
         assert-tile floor0 2 2 24 0\n\
         assert-tile floor0 5 4 24 0\n\
         undo\n\
         assert-tile floor0 2 2 23 0\n\
         # Find & replace (Ctrl+H)\n\
         press Ctrl+H\n\
         close-dialogs\n\
         replace 0/23/0 0/25/1\n\
         assert-tile floor0 4 3 25 1\n\
         undo\n\
         assert-tile floor0 4 3 23 0\n\
         paint-rect floor0 2 2 5 4 erase\n\
         assert-tile floor0 3 3 empty\n\
         undo\n\
         undo\n\
         assert-dirty false\n\
         # Random mix (Ctrl+click two tiles)\n\
         mix-add floor0 0x017000c2\n\
         mix-add floor0 0x018000c2\n\
         paint-rect floor0 0 0 9 9\n\
         assert-dirty true\n\
         undo\n\
         assert-dirty false\n\
         # Drag the map edge\n\
         drag-edge right 3\n\
         assert-size 60 41\n\
         undo\n\
         assert-size 57 41\n\
         drag-edge top 2\n\
         assert-size 57 43\n\
         undo\n\
         assert-size 57 41\n\
         # View modes\n\
         assert-view tiles\n\
         press Tab\n\
         assert-view objects\n\
         press Tab\n\
         assert-view walk\n\
         press Tab\n\
         assert-view automap\n\
         press Tab\n\
         assert-view light\n\
         press Tab\n\
         assert-view roofs\n\
         press Tab\n\
         assert-view tiles\n\
         press Shift+Tab\n\
         assert-view roofs\n\
         press F3\n\
         assert-view walk\n\
         press F2\n\
         assert-view objects\n\
         press F1\n\
         assert-view tiles\n\
         press R\n\
         press Z\n\
         press Z\n\
         # Export and PrintScreen\n\
         select-box floor0 5 5 9 9\n\
         export-png {} selection\n\
         export-png {}\n\
         copy-view\n\
         # New map\n\
         new-map data\\global\\tiles\\act1\\town\\capye2e.ds1 20 16 1\n\
         assert-size 20 16\n\
         assert-dirty false\n",
        sel_png.display(),
        map_png.display()
    );
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let (sw, sh) = image::image_dimensions(&sel_png).unwrap();
    let (mw, mh) = image::image_dimensions(&map_png).unwrap();
    assert!(
        sw > 0 && sh > 0 && sw < mw && sh < mh,
        "{sw}x{sh} vs {mw}x{mh}"
    );
    assert!(stderr.contains("copy-view: "), "{stderr}");
    let saved = home
        .path()
        .join("capybara\\saves\\vanilla\\data\\global\\tiles\\act1\\town\\capye2e.ds1");
    assert!(saved.is_file(), "the new map is saved");
    let changed =
        std::fs::read_to_string(home.path().join("capybara\\saves\\vanilla\\CHANGED.txt")).unwrap();
    assert!(
        changed.to_ascii_lowercase().contains("capye2e.ds1"),
        "{changed}"
    );
}
