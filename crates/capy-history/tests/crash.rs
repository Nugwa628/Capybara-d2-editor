//! P4 acceptance: killing the process during a save never corrupts the target file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/payload.rs"]
mod payload;

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

use capy_history::SavesDir;
use d2_assets::GamePath;

const KILLS: usize = 40;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn killing_save_loop_never_corrupts_target() {
    let tmp = tempfile::tempdir().unwrap();
    let saves_root = tmp.path().join("saves");
    let game_path = "data\\global\\tiles\\mxl\\zone\\whimsy\\shire.ds1";
    let gp = GamePath::new(game_path);
    let dir = SavesDir::new(&saves_root, "mxl");
    let target = dir.path_for(&gp).unwrap();
    let a = payload::payload(1);
    let b = payload::payload(2);
    assert!(payload::checksum_ok(&a) && payload::checksum_ok(&b) && a != b);

    let mut rng = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345)
        | 1;
    let (mut got_a, mut got_b) = (0, 0);

    for i in 0..KILLS {
        let mut child = Command::new(env!("CARGO_BIN_EXE_save_loop"))
            .arg(&saves_root)
            .arg("mxl")
            .arg(game_path)
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn save_loop");
        // Wait for the first complete save, then let it run for a random time.
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "ready", "iteration {i}");
        // 0..150 ms, scaled by a second random factor so kills land in different save phases.
        let delay_us = (xorshift(&mut rng) % 150_000) * (xorshift(&mut rng) % 100 + 1) / 100;
        std::thread::sleep(Duration::from_micros(delay_us));
        child.kill().expect("kill");
        child.wait().expect("wait");

        let bytes = std::fs::read(&target).unwrap();
        if bytes == a {
            got_a += 1;
        } else if bytes == b {
            got_b += 1;
        } else {
            panic!(
                "iteration {i}: target is neither payload ({} bytes, checksum_ok={})",
                bytes.len(),
                payload::checksum_ok(&bytes)
            );
        }
        let changed_raw = std::fs::read_to_string(dir.root().join("CHANGED.txt")).unwrap();
        let lines: Vec<&str> = changed_raw.lines().collect();
        assert_eq!(lines, vec![game_path], "iteration {i}: CHANGED.txt");
        assert_eq!(dir.changed().unwrap(), vec![gp.clone()]);
    }
    eprintln!("crash test: {KILLS} kills; target was payload A {got_a}x, B {got_b}x");

    // A later write cleans whatever temp files the killed processes left behind.
    dir.save(&gp, b"final").unwrap();
    for folder in [target.parent().unwrap(), dir.root()] {
        let temps: Vec<_> = std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".capytmp-"))
            .collect();
        assert!(
            temps.is_empty(),
            "stale temps left in {folder:?}: {temps:?}"
        );
    }
}
