//! End-to-end "Convert to palette" and palette memory (P7): the real exe
//! runs a `CAPY_SCRIPT` on vanilla that converts an act 5 DT1 to Act 1 with
//! one override and saves it to its game path, converts a UI DC6 to "All
//! acts (shared colours only)" and saves it, shows the palette badge states
//! and the save warning; a second run checks the palettes are remembered.
//!
//! The test checks the saved files byte by byte against the plan table it
//! computes itself (`capy_paint::remap::plan` with the same override), the
//! CHANGED.txt entries, and that the install folder is untouched.
//! Screenshots go to `target\tmp\palette\`.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use capy_paint::remap::{RemapSettings, candidates, plan};
use d2_assets::{GamePath, Profile};
use d2_formats::dt1::Dt1;
use d2_formats::palette::{Palette, act_palette_path};
use d2_formats::remap::{SpriteKind, index_histogram, remap_indices};

const DT1: &str = "data\\global\\tiles\\expansion\\siege\\ground.dt1";
/// UI DC6 candidates (the first with act-specific colours is converted).
const DC6S: [&str; 4] = [
    "data\\global\\ui\\bigmenu\\radiobutton.dc6",
    "data\\global\\ui\\bigmenu\\gembutton.dc6",
    "data\\global\\ui\\bigmenu\\scrollbar.dc6",
    "data\\global\\ui\\bigmenu\\numberarrows.dc6",
];
const OTHER: &str = "data\\global\\ui\\panel\\arrow.dc6";

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

/// (relative path, size, mtime) of every file under `dir`.
fn listing(dir: &Path) -> BTreeMap<PathBuf, (u64, std::time::SystemTime)> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, (u64, std::time::SystemTime)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let m = e.metadata().unwrap();
            if m.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    (m.len(), m.modified().unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn saved(home: &Path, game: &str) -> PathBuf {
    let mut p = home.join("capybara").join("saves").join("vanilla");
    for part in game.split('\\') {
        p = p.join(part);
    }
    p
}

fn run(home: &Path, script: &str, name: &str) {
    let file = home.join(format!("{name}.txt"));
    std::fs::write(&file, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &file)
        .output()
        .expect("start the app");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{name} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn convert_save_and_remember_palettes() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let install_before = listing(&vanilla);
    let scratch = tempfile::tempdir().unwrap();
    let overlay = Profile::vanilla(&vanilla).open(scratch.path()).unwrap();
    let read = |p: &str| overlay.read_bytes(&GamePath::new(p)).unwrap().unwrap();
    let pal = |act: u8| Palette::parse_dat(&read(act_palette_path(act))).unwrap();
    let (act1, act5) = (pal(0), pal(4));

    // The DT1 plan the app must use: defaults plus one override on the
    // changed colour with the most pixels (its second-best candidate).
    let dt1_bytes = read(DT1);
    let hist = index_histogram(SpriteKind::Dt1, &dt1_bytes).unwrap();
    let defaults = RemapSettings::default();
    let base = plan(&act5, &act1, &hist, &defaults);
    let changed = base
        .entries
        .iter()
        .filter(|e| e.target != e.source)
        .max_by_key(|e| e.pixels)
        .expect("act5 -> act1 changes some colour of the DT1")
        .clone();
    let over_target = candidates(&act5, &act1, changed.source, &defaults, 4)
        .into_iter()
        .map(|(t, _)| t)
        .find(|&t| t != changed.target)
        .unwrap();
    let mut with_override = defaults.clone();
    with_override.overrides.insert(changed.source, over_target);
    let expected = plan(&act5, &act1, &hist, &with_override);
    assert_eq!(expected.table[usize::from(changed.source)], over_target);

    // The DC6: the first candidate that uses act-specific colours.
    let act_specific = |h: &[u64; 256]| (225..=254).map(|i| h[i]).sum::<u64>();
    let dc6 = DC6S
        .iter()
        .copied()
        .find(|p| act_specific(&index_histogram(SpriteKind::Dc6, &read(p)).unwrap()) > 0)
        .expect("a UI DC6 with indices 225..=254");
    let sibling = DC6S.iter().copied().find(|p| *p != dc6).unwrap();

    let shots = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("tmp")
        .join("palette");
    std::fs::create_dir_all(&shots).unwrap();
    let shot = |n: &str| shots.join(n).display().to_string();
    let home = tempfile::tempdir().unwrap();
    let s = changed.source;
    let script = format!(
        "mode sprites\n\
         convert-open {DT1}\n\
         convert-source act5\n\
         convert-target act1\n\
         convert-override {s} {over_target}\n\
         convert-highlight changed\n\
         wait 10\n\
         screenshot {convert}\n\
         convert-highlight off\n\
         convert-hover {s}\n\
         wait 5\n\
         screenshot {hover}\n\
         convert-hover off\n\
         convert-picker {s}\n\
         wait 5\n\
         screenshot {picker}\n\
         convert-picker off\n\
         convert-apply\n\
         convert-save-game\n\
         assert-palette-badge act1 remembered\n\
         convert-open {dc6}\n\
         convert-source act1\n\
         convert-target all-acts\n\
         convert-apply\n\
         convert-save-game\n\
         convert-close\n\
         assert-palette-badge act1 remembered\n\
         sprite-open {sibling}\n\
         assert-palette-badge act1 folder\n\
         wait 5\n\
         screenshot {badge_folder}\n\
         sprite-open {OTHER}\n\
         assert-palette-badge act1 none\n\
         wait 5\n\
         screenshot {badge_none}\n\
         palette act5\n\
         assert-palette-badge act5 remembered\n\
         wait 5\n\
         screenshot {badge_remembered}\n\
         sprite-open {sibling}\n\
         assert-palette-badge act5 folder-other\n\
         wait 5\n\
         screenshot {badge_folder_other}\n\
         sprite-open {dc6}\n\
         assert-palette-badge act1 remembered\n\
         sprite-edit\n\
         sprite-open {OTHER}\n\
         assert-palette-badge act5 remembered-other editor\n\
         sprite-tab editor\n\
         wait 5\n\
         screenshot {badge_editor}\n\
         sprite-tab viewer\n\
         sprite-save-game-checked {dc6}\n\
         wait 5\n\
         screenshot {warning}\n\
         palette-warning cancel\n",
        convert = shot("convert_dt1.png"),
        hover = shot("convert_dt1_hover.png"),
        picker = shot("override_picker.png"),
        badge_folder = shot("badge_folder.png"),
        badge_none = shot("badge_none.png"),
        badge_remembered = shot("badge_remembered.png"),
        badge_folder_other = shot("badge_folder_other.png"),
        badge_editor = shot("badge_editor_remembered_other.png"),
        warning = shot("save_warning.png"),
    );
    run(home.path(), &script, "run1");

    // 1. The DT1: exactly the library's remap with the expected table; only
    // pixel bytes differ, and each differing byte is table[old].
    let out = std::fs::read(saved(home.path(), DT1)).expect("the converted DT1 was saved");
    assert_eq!(
        out,
        remap_indices(SpriteKind::Dt1, &dt1_bytes, &expected.table).unwrap()
    );
    assert_eq!(out.len(), dt1_bytes.len());
    let mut diffs = 0;
    for (i, (a, b)) in dt1_bytes.iter().zip(&out).enumerate() {
        if a != b {
            diffs += 1;
            assert_eq!(*b, expected.table[usize::from(*a)], "byte {i:#x}");
        }
    }
    assert!(diffs > 0, "the conversion changed pixels");
    let (before, after) = (Dt1::parse(&dt1_bytes).unwrap(), Dt1::parse(&out).unwrap());
    assert_eq!(before.tiles.len(), after.tiles.len());
    let mut override_pixels = 0u64;
    for (a, b) in before.tiles.iter().zip(&after.tiles) {
        assert_eq!(a.key(), b.key());
        let (ia, ib) = (a.render().unwrap(), b.render().unwrap());
        assert_eq!(
            (ia.width, ia.height, ia.alpha.clone()),
            (ib.width, ib.height, ib.alpha.clone())
        );
        for ((pa, pb), al) in ia.pixels.iter().zip(&ib.pixels).zip(&ia.alpha) {
            if *al != 0 {
                assert_eq!(*pb, expected.table[usize::from(*pa)]);
                override_pixels += u64::from(*pa == s);
            }
        }
    }
    assert!(override_pixels > 0, "the override's colour is used");

    // 2. The DC6: no act-specific index remains.
    let dc6_out = std::fs::read(saved(home.path(), dc6)).expect("the converted DC6 was saved");
    let h = index_histogram(SpriteKind::Dc6, &dc6_out).unwrap();
    assert_eq!(act_specific(&h), 0, "indices 225..=254 remain in {dc6}");

    // 3. CHANGED.txt lists both; the install folder is untouched.
    let changed_txt = std::fs::read_to_string(
        home.path()
            .join("capybara")
            .join("saves")
            .join("vanilla")
            .join("CHANGED.txt"),
    )
    .unwrap();
    let lines: Vec<&str> = changed_txt.lines().map(str::trim).collect();
    assert!(lines.contains(&DT1), "{changed_txt}");
    assert!(lines.contains(&dc6), "{changed_txt}");
    assert!(!lines.contains(&OTHER), "{changed_txt}");

    // 4. After a restart the badge shows the remembered palettes.
    run(
        home.path(),
        &format!(
            "mode sprites\n\
             palette act3\n\
             sprite-open {DT1}\n\
             assert-palette-badge act1 remembered\n\
             wait 5\n\
             screenshot {dt1_shot}\n\
             sprite-open {OTHER}\n\
             assert-palette-badge act5 remembered\n\
             sprite-open {dc6}\n\
             assert-palette-badge act1 remembered\n",
            dt1_shot = shot("restart_dt1_badge.png")
        ),
        "run2",
    );
    assert_eq!(
        listing(&vanilla),
        install_before,
        "the install folder changed"
    );
    println!(
        "palette e2e ok: {DT1}: {diffs} pixel bytes changed, override {s} -> {over_target} \
         ({override_pixels} pixels), mean {:.2} worst {:.1}; {dc6}: no index 225..=254 left",
        expected.mean_error, expected.worst_error
    );
}
