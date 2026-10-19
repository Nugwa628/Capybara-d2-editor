//! Test helpers: the private corpus, synthetic palettes and sprites.
#![allow(clippy::unwrap_used, clippy::panic, dead_code)]

use std::path::{Path, PathBuf};

use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use crate::model::Sprite;

/// `CAPY_PRIVATE_DIR`, or the nearest `testdata/private` above this crate.
pub fn private_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CAPY_PRIVATE_DIR") {
        let dir = PathBuf::from(dir);
        return dir.is_dir().then_some(dir);
    }
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    start
        .ancestors()
        .map(|a| a.join("testdata").join("private"))
        .find(|p| p.is_dir())
}

pub fn extracted(name: &str) -> Option<PathBuf> {
    let dir = private_root()?.join("extracted").join(name);
    let non_empty = std::fs::read_dir(&dir).ok()?.next().is_some();
    non_empty.then_some(dir)
}

pub fn find_files(base: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 1-in-`every` sample, or everything when `CAPY_FULL_CORPUS` is set.
pub fn sample(files: Vec<PathBuf>, every: usize) -> Vec<PathBuf> {
    if std::env::var_os("CAPY_FULL_CORPUS").is_some() || every <= 1 {
        return files;
    }
    files.into_iter().step_by(every).collect()
}

/// The Act 1 palette from the vanilla corpus, if present.
pub fn act1_palette() -> Option<Palette> {
    let path = extracted("vanilla")?.join("data/global/palette/act1/pal.dat");
    Palette::parse_dat(&std::fs::read(path).ok()?).ok()
}

/// A palette with 256 distinct colours (index 0 black).
pub fn synthetic_palette() -> Palette {
    let mut colors = [[0u8; 3]; 256];
    for (i, c) in colors.iter_mut().enumerate().skip(1) {
        let i = i as u8;
        *c = [
            i.wrapping_mul(37).wrapping_add(11) | 1,
            i.wrapping_mul(73).wrapping_add(5),
            255 - i.wrapping_mul(11),
        ];
        if *c == [0, 0, 0] {
            c[0] = 1;
        }
    }
    Palette { colors }
}

/// Runs `work` over `items` on all cores.
pub fn parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = work(item);
                    results.lock().unwrap().push(r);
                }
            });
        }
    });
    results.into_inner().unwrap()
}

pub fn image(w: u32, h: u32, ox: i32, oy: i32, px: Vec<u8>) -> IndexedImage {
    let alpha = px.iter().map(|&p| if p == 0 { 0 } else { 255 }).collect();
    IndexedImage {
        width: w,
        height: h,
        offset_x: ox,
        offset_y: oy,
        pixels: px,
        alpha,
    }
}

/// A small deterministic sprite: `dirs` x `frames`, each frame a different
/// size/offset, colours from at most `colours` palette entries.
pub fn small_sprite(dirs: usize, frames: usize, colours: u8) -> Sprite {
    let directions = (0..dirs)
        .map(|d| {
            (0..frames)
                .map(|f| {
                    let (w, h) = (6 + (d + f) as u32 % 5, 5 + f as u32 % 4);
                    let px = (0..w * h)
                        .map(|i| {
                            let (x, y) = (i % w, i / w);
                            if (x + y + d as u32 + f as u32).is_multiple_of(4) {
                                0
                            } else {
                                1 + ((x / 2 + y / 2) as u8 % colours.max(1))
                            }
                        })
                        .collect();
                    image(w, h, -(w as i32) / 2 + f as i32, -(h as i32) + d as i32, px)
                })
                .collect()
        })
        .collect();
    Sprite { directions }
}
