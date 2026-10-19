//! Test helpers: locating the private (gitignored) corpus.

use std::path::{Path, PathBuf};

/// `CAPY_PRIVATE_DIR`, or the nearest `testdata/private` above this crate
/// (a worktree lives inside the main checkout, so the walk finds it).
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

/// `extracted/<name>` under the private root, if it exists and is non-empty.
pub fn extracted(name: &str) -> Option<PathBuf> {
    let dir = private_root()?.join("extracted").join(name);
    let non_empty = std::fs::read_dir(&dir).ok()?.next().is_some();
    non_empty.then_some(dir)
}

/// Every file under `base` (recursive) whose extension equals `ext`, ignoring
/// case, sorted by path.
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

/// Deterministic 1-in-`every` sample of `files`, or all of them when the env
/// var `CAPY_FULL_CORPUS` is set. Keeps routine `cargo test` runs fast; the
/// lead runs the full corpus before closing a phase.
pub fn sample(files: Vec<PathBuf>, every: usize) -> Vec<PathBuf> {
    if std::env::var_os("CAPY_FULL_CORPUS").is_some() || every <= 1 {
        return files;
    }
    files.into_iter().step_by(every).collect()
}

/// Iteration count for hand-rolled mutation loops: `CAPY_FUZZ_ITERS`, or
/// `default` when unset or unparsable. Prints what it uses so a long run
/// shows its size.
pub fn fuzz_iters(label: &str, default: usize) -> usize {
    let n = std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default);
    println!("{label}: {n} mutations per seed (CAPY_FUZZ_ITERS, default {default})");
    n
}

/// Small deterministic PRNG (xorshift64*) for mutation fuzzing.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // never zero, or xorshift gets stuck
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15 | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish in `0..n`; 0 when `n == 0`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// Values that tend to break size and offset arithmetic.
const INTERESTING_U32: [u32; 10] = [
    0,
    1,
    2,
    0x7F,
    0xFF,
    0xFFFF,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFE,
    0xFFFF_FFFF,
];

/// `base` with 1..=4 random edits: bit flips, interesting bytes, interesting
/// or length-relative little-endian words, truncation, deleted, inserted or
/// duplicated ranges.
pub fn mutate(base: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = base.to_vec();
    let edits = 1 + rng.below(4);
    for _ in 0..edits {
        let len = v.len();
        match rng.below(9) {
            0 | 1 if len > 0 => {
                let i = rng.below(len);
                v[i] ^= 1 << rng.below(8);
            }
            2 if len > 0 => {
                let i = rng.below(len);
                v[i] = [0u8, 1, 0x7F, 0x80, 0xFF, b'\t', b'\n', b'\r'][rng.below(8)];
            }
            3 if len >= 4 => {
                let i = rng.below(len - 3);
                let word = match rng.below(3) {
                    0 => INTERESTING_U32[rng.below(INTERESTING_U32.len())],
                    1 => (len as u32)
                        .wrapping_add(rng.below(9) as u32)
                        .wrapping_sub(4),
                    _ => rng.next_u64() as u32,
                };
                v[i..i + 4].copy_from_slice(&word.to_le_bytes());
            }
            4 => v.truncate(rng.below(len + 1)),
            5 if len > 0 => {
                let a = rng.below(len);
                let b = (a + 1 + rng.below(64)).min(len);
                v.drain(a..b);
            }
            6 => {
                let at = rng.below(len + 1);
                let n = 1 + rng.below(32);
                let bytes: Vec<u8> = (0..n).map(|_| rng.next_u64() as u8).collect();
                v.splice(at..at, bytes);
            }
            7 if len > 0 => {
                let a = rng.below(len);
                let b = (a + 1 + rng.below(256)).min(len);
                let chunk = v[a..b].to_vec();
                let at = rng.below(len + 1);
                v.splice(at..at, chunk);
            }
            _ => {
                if len > 0 {
                    let i = rng.below(len);
                    v[i] = rng.next_u64() as u8;
                }
            }
        }
    }
    v
}

/// Up to `max` files under the private `extracted/<profile>` trees (both
/// profiles) whose extension is `ext` and whose path contains `filter`
/// (case-insensitive), spread evenly over the sorted list. Empty when the
/// corpus is absent.
pub fn corpus_seeds(ext: &str, filter: &str, max: usize) -> Vec<PathBuf> {
    let filter = filter.to_ascii_lowercase();
    let mut all = Vec::new();
    for profile in ["vanilla", "mxl"] {
        if let Some(root) = extracted(profile) {
            all.extend(
                find_files(&root, ext)
                    .into_iter()
                    .filter(|p| p.to_string_lossy().to_ascii_lowercase().contains(&filter)),
            );
        }
    }
    if all.len() <= max || max == 0 {
        return all;
    }
    let step = all.len().div_ceil(max);
    all.into_iter().step_by(step).collect()
}

/// Runs `work` over `items` on every core; results come back in any order.
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
                    if let Ok(mut out) = results.lock() {
                        out.push(r);
                    }
                }
            });
        }
    });
    results.into_inner().unwrap_or_default()
}
