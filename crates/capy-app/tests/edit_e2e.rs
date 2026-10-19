//! End-to-end editing: the real exe runs a `CAPY_SCRIPT` on vanilla
//! `towne1.ds1` (move a Wall(0) selection to Wall(2), paint, copy, paste,
//! save), then this test reads the saved file back and checks it, plus
//! CHANGED.txt and the history snapshot. Failing scripts must exit with 1.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_history::SnapshotStore;
use d2_assets::{GamePath, Profile};
use d2_formats::ds1::{Cell, Ds1};

const MAP: &str = "data\\global\\tiles\\act1\\town\\towne1.ds1";

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

/// Runs the exe on `script` with a scratch home; returns (exit code, stderr).
fn run_script(home: &Path, script: &str) -> (Option<i32>, String) {
    let file = home.join("script.txt");
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(&file, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybaraMapEditor"))
        .env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn original(vanilla: &Path) -> Ds1 {
    let saves = tempfile::tempdir().unwrap();
    let overlay = Profile::vanilla(vanilla).open(saves.path()).unwrap();
    let bytes = overlay
        .read_bytes(&GamePath::new(MAP))
        .unwrap()
        .expect("towne1.ds1 in the vanilla install");
    Ds1::parse(&bytes).unwrap()
}

#[test]
fn scripted_edit_saves_a_correct_map() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let orig = original(&vanilla);
    let (w, h) = (orig.width, orig.height);
    assert!(orig.walls.len() < 3, "the test expects fewer than 3 walls");
    let wall0_cells = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| !orig.walls[0].tiles.get(x, y).unwrap().is_empty())
        .count();
    assert!(wall0_cells > 0);

    // Clip: floor cells (6..=15, 6..=13), pasted with its top-left at (30, 20).
    let (clip_x0, clip_y0, clip_x1, clip_y1) = (6u32, 6u32, 15u32, 13u32);
    let (at_x, at_y) = (30u32, 20u32);
    let painted = Cell(1).with_indices(3, 2);
    let objects_in_clip = orig
        .objects
        .iter()
        .filter(|o| {
            let (tx, ty) = (o.x / 5, o.y / 5);
            (clip_x0 as i32..=clip_x1 as i32).contains(&tx)
                && (clip_y0 as i32..=clip_y1 as i32).contains(&ty)
        })
        .count();

    let script = format!(
        "open {MAP}\n\
         select-box wall0 0 0 {x1} {y1}\n\
         assert-selection {wall0_cells}\n\
         move-layer wall0 wall2 overwrite\n\
         paint floor0 3 3 {painted:x}\n\
         select-box floor0 {clip_x0} {clip_y0} {clip_x1} {clip_y1}\n\
         copy\n\
         paste {at_x} {at_y} overwrite\n\
         assert-dirty true\n\
         save\n\
         assert-dirty false\n\
         undo\n\
         assert-dirty true\n\
         redo\n\
         assert-dirty false\n",
        x1 = w - 1,
        y1 = h - 1,
        painted = painted.0,
    );
    let home = tempfile::tempdir().unwrap();
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");

    let saves = home.path().join("capybara").join("saves").join("vanilla");
    let saved_path = saves
        .join("data")
        .join("global")
        .join("tiles")
        .join("act1")
        .join("town")
        .join("towne1.ds1");
    let bytes = std::fs::read(&saved_path).expect("the saved file");
    let saved = Ds1::parse(&bytes).expect("the saved file parses");
    assert_eq!((saved.width, saved.height), (w, h));

    // Wall(0) moved to Wall(2): same tiles and orientations, sources empty.
    assert_eq!(saved.walls.len(), 3, "Wall 2 and Wall 3 were added");
    for y in 0..h {
        for x in 0..w {
            let before = orig.walls[0].tiles.get(x, y).unwrap();
            if before.is_empty() {
                continue;
            }
            assert_eq!(saved.walls[2].tiles.get(x, y).unwrap(), before, "({x},{y})");
            assert_eq!(
                saved.walls[2].orientations.get(x, y),
                orig.walls[0].orientations.get(x, y),
                "orientation at ({x},{y})"
            );
            assert!(saved.walls[0].tiles.get(x, y).unwrap().is_empty());
        }
    }
    // The painted cell.
    assert_eq!(saved.floors[0].get(3, 3), Some(painted));
    // The pasted block: every clip cell now sits at (at + offset).
    let mut compared = 0;
    for dy in 0..=(clip_y1 - clip_y0) {
        for dx in 0..=(clip_x1 - clip_x0) {
            let src = orig.floors[0].get(clip_x0 + dx, clip_y0 + dy).unwrap();
            if src.is_empty() {
                continue;
            }
            assert_eq!(
                saved.floors[0].get(at_x + dx, at_y + dy),
                Some(src),
                "pasted cell +({dx},{dy})"
            );
            compared += 1;
        }
    }
    assert!(compared > 0);
    // Objects inside the clip came along with it.
    assert_eq!(saved.objects.len(), orig.objects.len() + objects_in_clip);

    // CHANGED.txt lists the path exactly once.
    let changed = std::fs::read_to_string(saves.join("CHANGED.txt")).unwrap();
    let lines: Vec<&str> = changed.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines, vec![MAP], "CHANGED.txt: {changed:?}");

    // A snapshot of the saved bytes exists.
    let store = SnapshotStore::open(&home.path().join("capybara").join("history"), "vanilla")
        .expect("history store");
    let timeline = store.timeline(&GamePath::new(MAP));
    assert_eq!(timeline.len(), 1, "one snapshot");
    assert_eq!(store.read(timeline[0].id).unwrap(), bytes);

    // Only `capybara\` was written: the game install has no CHANGED.txt etc.
    assert!(!vanilla.join("CHANGED.txt").exists());
}

#[test]
fn failing_scripts_exit_with_one_and_say_why() {
    if !gpu_tests_enabled() {
        return;
    }
    if vanilla_dir().is_none() {
        eprintln!("skipped: vanilla install not found");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let (code, stderr) = run_script(home.path(), "undo\nfrobnicate 1 2\n");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("frobnicate"), "{stderr}");

    let (code, stderr) = run_script(
        &home.path().join("two"),
        &format!("open {MAP}\nassert-dirty true\n"),
    );
    assert_eq!(code, Some(1));
    assert!(stderr.contains("assert-dirty"), "{stderr}");

    let (code, _) = run_script(
        &home.path().join("three"),
        &format!("open {MAP}\nassert-dirty false\n"),
    );
    assert_eq!(code, Some(0));
}
