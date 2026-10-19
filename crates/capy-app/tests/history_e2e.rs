//! End-to-end history: the real exe runs a `CAPY_SCRIPT` on vanilla
//! `towne1.ds1`: paint, autosave, checkpoint, paint more, select the
//! checkpoint and check the diff, restore it, save, make milestones and
//! prune, with screenshots of the History panel, the diff overlay, the
//! snapshot view, the Settings dialog and Add game install. The test then
//! checks the files: CHANGED.txt changed only by the save, the saved map
//! equals the checkpoint, retention kept the checkpoint, one milestone.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_history::{Milestones, SnapshotKind, SnapshotStore};
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

/// Every file directly in `dir` with its size and modification time.
fn listing(dir: &Path) -> Vec<(String, u64, Option<std::time::SystemTime>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            let m = e.metadata().unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                m.len(),
                m.modified().ok(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn history_timeline_diff_restore_milestones_and_prune() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let install_before = listing(&vanilla);
    let orig = original(&vanilla);
    let first = Cell(1).with_indices(3, 2);
    let more = Cell(1).with_indices(5, 1);
    // A block of cells whose original floor tile differs from `more`.
    let wall_cells: Vec<(u32, u32)> = (12..18)
        .flat_map(|y| (14..22).map(move |x| (x, y)))
        .filter(|&(x, y)| orig.floors[0].get(x, y) != Some(more) && (x, y) != (3, 3))
        .collect();
    assert!(wall_cells.len() >= 20, "the test needs cells to paint");
    let paint_more: String = wall_cells
        .iter()
        .map(|(x, y)| format!("paint floor0 {x} {y} {:x}\n", more.0))
        .collect();
    let n = wall_cells.len();

    let shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("history_e2e");
    std::fs::create_dir_all(&shots).unwrap();
    let shot = |name: &str| shots.join(name).display().to_string();
    let script = format!(
        "open {MAP}\n\
         setting autosave_minutes 0\n\
         assert-changed 0\n\
         paint floor0 3 3 {first:x}\n\
         autosave-now\n\
         assert-history 1 autosave\n\
         assert-changed 0\n\
         checkpoint before-wall\n\
         assert-history 2\n\
         assert-history 1 checkpoint\n\
         assert-changed 0\n\
         {paint_more}\
         history-select before-wall\n\
         assert-diff floor0 {n}\n\
         assert-diff wall0 0\n\
         history-tab\n\
         screenshot {panel}\n\
         zoom 2\n\
         history-jump floor0\n\
         screenshot {overlay}\n\
         show-snapshot on\n\
         screenshot {snapshot}\n\
         show-snapshot off\n\
         restore before-wall\n\
         assert-dirty true\n\
         assert-history 1 before-restore\n\
         assert-diff floor0 0\n\
         assert-changed 0\n\
         save\n\
         assert-changed 1\n\
         assert-history 1 save\n\
         milestone first\n\
         assert-milestones 1\n\
         milestone nothing new\n\
         assert-milestones 1\n\
         prune 1\n\
         assert-history 1 checkpoint\n\
         assert-history 2\n\
         settings open\n\
         screenshot {settings}\n\
         settings close\n\
         add-install {install}\n\
         screenshot {add_install}\n\
         close-dialogs\n",
        first = first.0,
        panel = shot("history_panel_diff.png"),
        overlay = shot("diff_overlay_zoomed.png"),
        snapshot = shot("snapshot_view.png"),
        settings = shot("settings.png"),
        add_install = shot("add_game_install.png"),
        install = vanilla.display(),
    );
    let home = tempfile::tempdir().unwrap();
    let (code, stderr) = run_script(home.path(), &script);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");

    let capy = home.path().join("capybara");
    let saves = capy.join("saves").join("vanilla");
    let saved_path = saves
        .join("data")
        .join("global")
        .join("tiles")
        .join("act1")
        .join("town")
        .join("towne1.ds1");
    let saved_bytes = std::fs::read(&saved_path).expect("the saved file");
    let saved = Ds1::parse(&saved_bytes).unwrap();
    // The restored checkpoint: the first paint, none of the later ones.
    assert_eq!(saved.floors[0].get(3, 3), Some(first));
    for &(x, y) in &wall_cells {
        assert_eq!(
            saved.floors[0].get(x, y),
            orig.floors[0].get(x, y),
            "({x},{y})"
        );
    }

    // CHANGED.txt lists the map once: written by the save only (the script
    // asserted it was empty after the autosave, the checkpoint and the restore).
    let changed = std::fs::read_to_string(saves.join("CHANGED.txt")).unwrap();
    let lines: Vec<&str> = changed.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines, vec![MAP], "CHANGED.txt: {changed:?}");

    // After `prune 1`: the checkpoint survived, plus the newest save.
    let store = SnapshotStore::open(&capy.join("history"), "vanilla").unwrap();
    let timeline = store.timeline(&GamePath::new(MAP));
    let kinds: Vec<&SnapshotKind> = timeline.iter().map(|s| &s.kind).collect();
    assert_eq!(
        kinds,
        vec![
            &SnapshotKind::Checkpoint {
                name: "before-wall".into()
            },
            &SnapshotKind::Save
        ]
    );
    let checkpoint = store.read(timeline[0].id).unwrap();
    assert_eq!(
        saved_bytes, checkpoint,
        "the save wrote the restored checkpoint"
    );
    assert_eq!(store.read(timeline[1].id).unwrap(), saved_bytes);

    // One milestone (the second was a no-op).
    let milestones = Milestones::open_or_init(&capy.join("saves")).unwrap();
    let log = milestones.log(10).unwrap();
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0].message.trim(), "first");

    for name in [
        "history_panel_diff.png",
        "diff_overlay_zoomed.png",
        "snapshot_view.png",
        "settings.png",
        "add_game_install.png",
    ] {
        assert!(shots.join(name).is_file(), "{name}");
    }
    eprintln!("screenshots in {}", shots.display());

    // The install folder is untouched; the profiles folder holds only the
    // first-run profile(s).
    assert_eq!(listing(&vanilla), install_before, "the install changed");
    let profiles: Vec<String> = std::fs::read_dir(capy.join("profiles"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        profiles.contains(&"vanilla.toml".to_owned()),
        "{profiles:?}"
    );
}
