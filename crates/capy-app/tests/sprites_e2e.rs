//! End-to-end Sprites mode: the real exe runs a `CAPY_SCRIPT` that opens a
//! vanilla DCC by game path, exports its frames, imports them back as a DCC
//! and saves it to a game path, then outlines and gradients it in the editor
//! and saves again. The test checks the saved files (frames, CHANGED.txt,
//! snapshots, palette lock) and that `CapybaraMapEditor.exe render ...` (the
//! single-exe CLI) writes a PNG and exits 0.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use capy_history::SnapshotStore;
use capy_sprite::{Sprite, SpriteFormat};
use d2_assets::{GamePath, Profile};
use d2_formats::palette::{Palette, act_palette_path};

const DCC: &str = "data\\global\\monsters\\fa\\tr\\fatrlitwlhth.dcc";
const EDITED: &str = "data\\global\\monsters\\fa\\tr\\fatrlitnuhth.dcc";
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

fn exe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_CapybaraMapEditor"))
}

/// Opaque pixels in anchor space: (x, y) -> RGB under `palette`.
fn pixels(img: &d2_formats::IndexedImage, palette: &Palette) -> BTreeMap<(i32, i32), [u8; 3]> {
    let mut out = BTreeMap::new();
    for y in 0..img.height {
        for x in 0..img.width {
            let i = (y * img.width + x) as usize;
            if img.alpha[i] != 0 {
                let key = (img.offset_x + x as i32, img.offset_y + y as i32);
                out.insert(key, palette.colors[usize::from(img.pixels[i])]);
            }
        }
    }
    out
}

fn read_saved(home: &Path, game: &str) -> Vec<u8> {
    let mut p = home.join("capybara").join("saves").join("vanilla");
    for part in game.split('\\') {
        p = p.join(part);
    }
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn sprite_round_trip_edit_and_cli() {
    if !gpu_tests_enabled() {
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let overlay = Profile::vanilla(&vanilla).open(scratch.path()).unwrap();
    let original_bytes = overlay
        .read_bytes(&GamePath::new(DCC))
        .unwrap()
        .expect("the vanilla DCC");
    let original = Sprite::decode(&original_bytes, SpriteFormat::Dcc).unwrap();
    let palette = Palette::parse_dat(
        &overlay
            .read_bytes(&GamePath::new(act_palette_path(0)))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let (dirs, frames) = (original.directions.len(), original.frames_per_direction());

    let home = tempfile::tempdir().unwrap();
    let frames_dir = home.path().join("frames");
    let files: Vec<String> = (0..dirs)
        .flat_map(|d| (0..frames).map(move |f| (d, f)))
        .map(|(d, f)| {
            frames_dir
                .join(format!("d{d:02}_f{f:03}.png"))
                .display()
                .to_string()
        })
        .collect();
    let script = format!(
        "mode sprites\n\
         sprite-open {DCC}\n\
         sprite-export-frames {frames}\n\
         sprite-import {files} dirs {dirs} frames {per} dcc\n\
         sprite-save-game {DCC}\n\
         sprite-dir 1\n\
         sprite-frame 2\n\
         paint-outline auto outside 1\n\
         paint-gradient 0 0 30 40 ramp 140 bayer\n\
         paint-pencil 1 1 120\n\
         paint-undo\n\
         sprite-save-game {EDITED}\n",
        frames = frames_dir.display(),
        files = files.join(" "),
        per = frames,
    );
    let script_file = home.path().join("script.txt");
    std::fs::write(&script_file, script).unwrap();
    let out = exe()
        .env("CAPY_HOME", home.path().join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &script_file)
        .output()
        .expect("start the app");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "script failed:\n{stderr}");

    // 1. The export: one PNG per frame and sprite.json.
    assert!(frames_dir.join("sprite.json").is_file());
    assert_eq!(
        std::fs::read_dir(&frames_dir)
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "png"))
            .count(),
        dirs * frames
    );

    // 2-3. The re-imported DCC decodes to the same frames (same pixels at
    // the same anchor positions, same colours).
    let saved = Sprite::decode(&read_saved(home.path(), DCC), SpriteFormat::Dcc)
        .expect("the saved DCC decodes");
    assert_eq!(saved.directions.len(), dirs);
    assert_eq!(saved.frames_per_direction(), frames);
    for d in 0..dirs {
        for f in 0..frames {
            assert_eq!(
                pixels(&saved.directions[d][f], &palette),
                pixels(&original.directions[d][f], &palette),
                "dir {d} frame {f}"
            );
        }
    }
    let changed = std::fs::read_to_string(
        home.path()
            .join("capybara")
            .join("saves")
            .join("vanilla")
            .join("CHANGED.txt"),
    )
    .unwrap();
    let lines: Vec<&str> = changed.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(lines.contains(&DCC), "CHANGED.txt: {changed:?}");
    assert!(lines.contains(&EDITED), "CHANGED.txt: {changed:?}");
    let store = SnapshotStore::open(&home.path().join("capybara").join("history"), "vanilla")
        .expect("history");
    assert_eq!(store.timeline(&GamePath::new(DCC)).len(), 1, "one snapshot");
    assert_eq!(
        store.timeline(&GamePath::new(EDITED)).len(),
        1,
        "one snapshot"
    );

    // 4. The edited sprite: decodes, palette-locked (no opaque index 0), and
    // the outline grew the edited frame.
    let edited = Sprite::decode(&read_saved(home.path(), EDITED), SpriteFormat::Dcc)
        .expect("the edited DCC decodes");
    for (d, dir) in edited.directions.iter().enumerate() {
        for (f, img) in dir.iter().enumerate() {
            assert!(
                !img.pixels
                    .iter()
                    .zip(&img.alpha)
                    .any(|(&p, &a)| a != 0 && p == 0),
                "opaque index 0 in dir {d} frame {f}"
            );
        }
    }
    let before = pixels(&original.directions[1][2], &palette).len();
    let after = pixels(&edited.directions[1][2], &palette).len();
    assert!(after > before, "outline added pixels: {before} -> {after}");
    assert_eq!(
        pixels(&edited.directions[0][0], &palette),
        pixels(&original.directions[0][0], &palette),
        "other frames are untouched"
    );

    // 5. The single exe's CLI.
    let png = home.path().join("towne1.png");
    let out = exe()
        .env("CAPY_HOME", home.path().join("capybara"))
        .args([
            "render",
            MAP,
            "--profile",
            "vanilla",
            "--zoom",
            "1:4",
            "--out",
        ])
        .arg(&png)
        .output()
        .expect("run the CLI");
    assert_eq!(
        out.status.code(),
        Some(0),
        "render failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&png).expect("the PNG");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    println!(
        "e2e ok: {dirs}x{frames} DCC round trip identical; edited sprite palette-locked; \
         CLI render wrote {} bytes; stdout: {}",
        bytes.len(),
        String::from_utf8_lossy(&out.stdout).trim()
    );

    // Nothing was written into the game install.
    assert!(!vanilla.join("CHANGED.txt").exists());
}
