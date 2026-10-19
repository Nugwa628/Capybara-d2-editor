//! End-to-end loading screen: the real exe runs a `CAPY_SCRIPT` that pins
//! door frames 0, 4 and 9 and the plain fallback and takes window
//! screenshots, saves frame 4's view as a PNG, plays the splash as at
//! launch until it closes by itself, checks that a dialog closes it at once,
//! and screenshots the main window. A second run does frame 9 on Median XL.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1` (the Median XL run is skipped without that install).
//! Screenshots go to `target\tmp\release_app\`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn gpu_tests_enabled() -> bool {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return false;
    }
    true
}

fn shots() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release_app");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(home: &Path, profile: &str, script: &str) {
    let file = home.join("script.txt");
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(&file, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", profile)
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn door_frames_fallback_auto_close_and_dialogs() {
    if !gpu_tests_enabled() {
        return;
    }
    let vanilla = PathBuf::from(r"C:\Program Files (x86)\Diablo II");
    if !vanilla.join("d2data.mpq").is_file() {
        eprintln!("skipped: vanilla install not found");
        return;
    }
    let dir = shots();
    let shot = |name: &str| dir.join(name).display().to_string();
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "splash 0\n\
         screenshot {f0}\n\
         splash 4\n\
         screenshot {f4}\n\
         splash 9\n\
         screenshot {f9}\n\
         splash-save 4 2 {view}\n\
         splash plain\n\
         screenshot {plain}\n\
         splash off\n\
         splash auto\n\
         splash-wait-closed\n\
         splash 4\n\
         add-install\n\
         wait 3\n\
         splash-wait-closed\n\
         close-dialogs\n\
         open data\\global\\tiles\\act1\\town\\towne1.ds1\n\
         screenshot {main}\n",
        f0 = shot("splash-vanilla-0.png"),
        f4 = shot("splash-vanilla-4.png"),
        f9 = shot("splash-vanilla-9.png"),
        view = shot("splash-view-4.png"),
        plain = shot("splash-fallback.png"),
        main = shot("main-window.png"),
    );
    run(home.path(), "vanilla", &script);
    // The saved view is the 2x picture: black-bordered frame, gold title.
    let view = image::open(dir.join("splash-view-4.png"))
        .unwrap()
        .to_rgb8();
    let gold = (150..362)
        .flat_map(|x| (480..492).map(move |y| (x, y)))
        .filter(|&(x, y)| view.get_pixel(x, y).0 == [240, 200, 90])
        .count();
    assert_eq!(view.dimensions(), (512, 512));
    assert!(gold > 100, "{gold} gold title pixels on the plate");
    // Door frames differ (the door opens), the fallback differs from both.
    let bytes = |n: &str| std::fs::read(dir.join(n)).unwrap();
    assert_ne!(bytes("splash-vanilla-0.png"), bytes("splash-vanilla-9.png"));
    assert_ne!(bytes("splash-vanilla-4.png"), bytes("splash-fallback.png"));
}

#[test]
fn median_xl_door_frame() {
    if !gpu_tests_enabled() {
        return;
    }
    let mxl = std::env::var_os("CAPY_MXL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\games\median-xl"));
    if !mxl.join("d2data.mpq").is_file() {
        eprintln!("skipped: Median XL install not found");
        return;
    }
    let dir = shots();
    let home = tempfile::tempdir().unwrap();
    let script = format!(
        "splash 9\nscreenshot {}\n",
        dir.join("splash-mxl-9.png").display()
    );
    run(home.path(), "mxl", &script);
    assert!(dir.join("splash-mxl-9.png").is_file());
}
