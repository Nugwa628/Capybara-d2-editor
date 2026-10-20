//! End-to-end checks of the Map Editor's ds1-studio style shell (P8): the
//! real exe opens vanilla `townn1.ds1`, clicks every visible, enabled ribbon
//! button of every tab with real pointer events and presses the key of every
//! command that is enabled at that moment (`click-ribbon-all`,
//! `press-all-keys`); a second run takes a 1600x900 screenshot of the map
//! and one per built-in theme.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
//! Screenshots go to `<target tmp>/shell_e2e/`; the second run keeps its
//! asset index in `<target tmp>/shell_e2e_home/` so later runs reuse it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

const MAP: &str = "data\\global\\tiles\\act1\\town\\townn1.ds1";
const THEMES: [&str; 9] = [
    "studio",
    "sanctuary",
    "midnight",
    "graphite",
    "nord",
    "ember",
    "verdant",
    "parchment",
    "snow",
];

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

/// Runs the exe on `script` with `home`; returns (exit code, stderr).
fn run_script(home: &Path, script: &str, index: bool) -> (Option<i32>, String) {
    let file = home.join("script.txt");
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(&file, script).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"));
    cmd.env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_SCALE_FACTOR", "1")
        .env("CAPY_SCRIPT", &file);
    if !index {
        cmd.env("CAPY_NO_INDEX", "1");
    }
    let out = cmd.output().expect("start the app");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The number after `marker` in the log.
fn count_after(log: &str, marker: &str) -> usize {
    let at = log
        .find(marker)
        .unwrap_or_else(|| panic!("{marker:?} not in the log:\n{log}"));
    log[at + marker.len()..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no count after {marker:?}"))
}

#[test]
fn every_ribbon_button_and_default_key_works() {
    if !gpu_tests_enabled() {
        return;
    }
    if vanilla_dir().is_none() {
        eprintln!("skipped: vanilla install not found");
        return;
    }
    // A selection first, so Cut/Copy/Delete/Move and the flags are enabled.
    let script = format!(
        "open {MAP}\n\
         select-box floor0 5 5 8 8\n\
         click-ribbon-all\n\
         open {MAP}\n\
         select-box floor0 5 5 8 8\n\
         press-all-keys\n\
         press Ctrl+K\n\
         press Escape\n\
         ribbon-tab help\n\
         click help.shortcuts\n\
         close-dialogs\n\
         tool cmd file.exportPackage\n\
         tool cmd file.importPackage\n\
         tool cmd file.importDs1Dt1\n\
         tool cmd presets.save\n\
         wait 3\n\
         close-dialogs\n"
    );
    let home = tempfile::tempdir().unwrap();
    let (code, stderr) = run_script(home.path(), &script, false);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let clicked = count_after(&stderr, "click-ribbon:");
    let pressed = count_after(&stderr, "press-all-keys:");
    eprintln!("{clicked} ribbon buttons clicked, {pressed} keys pressed");
    assert!(clicked >= 80, "only {clicked} ribbon buttons were clicked");
    assert!(pressed >= 65, "only {pressed} keys were pressed");
    // Failing clicks must fail the script.
    let (code, stderr) = run_script(home.path(), &format!("open {MAP}\nclick no.such\n"), false);
    assert_eq!(code, Some(1), "{stderr}");
}

#[test]
fn screenshots_of_the_map_and_every_theme() {
    if !gpu_tests_enabled() {
        return;
    }
    if vanilla_dir().is_none() {
        eprintln!("skipped: vanilla install not found");
        return;
    }
    let shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shell_e2e");
    std::fs::create_dir_all(&shots).unwrap();
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shell_e2e_home");
    // Keep the index cache, start from default settings.
    let _ = std::fs::remove_file(home.join("capybara").join("config.toml"));
    let shot = |name: &str| shots.join(format!("{name}.png"));
    // The first screenshot waits until the map is drawn, so its tile set is
    // known for `browser-scope map`.
    let mut script = format!(
        "open {MAP}\n\
         zoom 3\n\
         screenshot {}\n\
         search type:wall\n\
         browser-scope map\n\
         search type:wall\n\
         theme studio\n\
         screenshot {}\n",
        shot("warm-up").display(),
        shot("overview").display()
    );
    for t in THEMES {
        script.push_str(&format!(
            "theme {t}\nscreenshot {}\n",
            shot(&format!("theme-{t}")).display()
        ));
    }
    let (code, stderr) = run_script(&home, &script, true);
    assert_eq!(code, Some(0), "script failed:\n{stderr}");
    let names =
        std::iter::once("overview".to_owned()).chain(THEMES.iter().map(|t| format!("theme-{t}")));
    for name in names {
        let (w, h) = image::image_dimensions(shot(&name)).unwrap();
        assert_eq!((w, h), (1600, 900), "{name}");
    }
    eprintln!("screenshots in {}", shots.display());
}
