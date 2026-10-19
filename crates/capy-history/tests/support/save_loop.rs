//! Crash-test helper: `save_loop <saves_root> <profile> <game_path>` saves two
//! distinct multi-MB payloads alternately, forever. Prints `ready` once the
//! first save has completed. The test kills it at random moments.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod payload;

use std::io::Write;

use capy_history::SavesDir;
use d2_assets::GamePath;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, root, profile, game_path] = args.as_slice() else {
        eprintln!("usage: save_loop <saves_root> <profile> <game_path>");
        std::process::exit(2);
    };
    let dir = SavesDir::new(std::path::Path::new(root), profile);
    let gp = GamePath::new(game_path);
    let a = payload::payload(1);
    let b = payload::payload(2);
    let mut first = true;
    loop {
        for bytes in [&a, &b] {
            dir.save(&gp, bytes).expect("save failed");
            if first {
                first = false;
                let mut out = std::io::stdout();
                writeln!(out, "ready").expect("stdout");
                out.flush().expect("flush");
            }
        }
    }
}
