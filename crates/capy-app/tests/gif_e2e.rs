//! End-to-end Export GIF window: the real exe runs a `CAPY_SCRIPT` that opens
//! a vanilla DCC, opens the window, sets a frame range, directions, speed and
//! scale, and exports; the test decodes the GIFs.
//!
//! Needs a window (GPU) and the vanilla install, so it only runs with
//! `CAPY_GPU_TESTS=1`. `CAPY_VANILLA_DIR` overrides the install folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use capy_sprite::{Sprite, SpriteFormat};
use d2_assets::{GamePath, Profile};

const DCC: &str = "data\\global\\monsters\\fa\\tr\\fatrlitwlhth.dcc";

fn vanilla_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CAPY_VANILLA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Diablo II"));
    dir.is_dir().then_some(dir)
}

fn gif_frames(path: &Path) -> (u16, u16, Vec<u16>) {
    let mut o = gif::DecodeOptions::new();
    o.set_color_output(gif::ColorOutput::Indexed);
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut d = o.read_info(file).unwrap();
    let (w, h) = (d.width(), d.height());
    let mut delays = Vec::new();
    while let Some(f) = d.read_next_frame().unwrap() {
        delays.push(f.delay);
    }
    (w, h, delays)
}

#[test]
fn export_gif_window_writes_ranges_per_direction_and_sequences() {
    if std::env::var("CAPY_GPU_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CAPY_GPU_TESTS=1 (needs a GPU and the game install)");
        return;
    }
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let overlay = Profile::vanilla(&vanilla).open(scratch.path()).unwrap();
    let bytes = overlay
        .read_bytes(&GamePath::new(DCC))
        .unwrap()
        .expect("the vanilla DCC");
    let sprite = Sprite::decode(&bytes, SpriteFormat::Dcc).unwrap();
    assert!(sprite.directions.len() > 3 && sprite.frames_per_direction() > 5);

    let home = tempfile::tempdir().unwrap();
    let out = home.path().join("gifs");
    std::fs::create_dir_all(&out).unwrap();
    let script = format!(
        "mode sprites\n\
         sprite-open {DCC}\n\
         sprite-gif-open\n\
         sprite-gif-set range=2-5 dirs=0,3 output=per scale=2 fps=12.5\n\
         sprite-gif-export {per}\n\
         sprite-gif-set output=seq loop=off scale=1 fps=25\n\
         sprite-gif-export {seq}\n\
         sprite-edit\n\
         sprite-gif-open\n\
         sprite-gif-set range=1-3 dirs=cur bg=102030\n\
         sprite-gif-export {ed}\n",
        per = out.join("walk.gif").display(),
        seq = out.join("seq.gif").display(),
        ed = out.join("edited.gif").display(),
    );
    let script_file = home.path().join("script.txt");
    std::fs::write(&script_file, script).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", home.path().join("capybara"))
        .env("CAPY_PROFILE", "vanilla")
        .env("CAPY_NO_INDEX", "1")
        .env("CAPY_SCRIPT", &script_file)
        .output()
        .expect("start the app");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(0), "script failed:\n{stderr}");

    let (w2, h2, d2) = gif_frames(&out.join("walk_d00.gif"));
    assert_eq!(d2, vec![8; 4], "frames 2-5 at 12.5 fps");
    let (w3, h3, d3) = gif_frames(&out.join("walk_d03.gif"));
    assert_eq!(d3.len(), 4);
    assert!(w3 > 0 && h3 > 0);
    assert!(!out.join("walk.gif").exists(), "per-direction names only");
    let (w1, h1, seq) = gif_frames(&out.join("seq.gif"));
    assert_eq!(seq, vec![4; 8], "two directions in sequence at 25 fps");
    assert!(w2 >= w1 && h2 >= h1, "scale 2 is not smaller");
    let (_, _, ed) = gif_frames(&out.join("edited.gif"));
    assert_eq!(ed.len(), 3);
}
