//! Shared helpers for the image/map format fuzz tests (DS1, DT1, DC6, DCC):
//! a seeded mutator, case-count knobs and private-corpus seeds.
//!
//! Knobs: `PROPTEST_CASES` (proptest case counts, see [`cases`]) and
//! `CAPY_FUZZ_ITERS` (mutations per corpus seed in the hand-rolled loops, see
//! [`fuzz_iters`]).

use crate::testutil;

/// `PROPTEST_CASES` when set, else `default`; for proptest blocks that want a
/// smaller default than proptest's 256 but must still honour the env var.
pub fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// `CAPY_FUZZ_ITERS` when set, else `default`.
pub fn fuzz_iters(default: usize) -> usize {
    std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A small deterministic generator (proptest or the loop supplies the seed).
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    /// Uniform-ish in `0..n` (`0` when `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Dword values that tend to break size and offset arithmetic.
const NASTY: [u32; 14] = [
    0,
    1,
    2,
    3,
    4,
    0x7F,
    0x80,
    0xFF,
    0xFFFF,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    0xFFFF_FFFE,
    0x0010_0000,
];

/// Applies `1..=rounds` random mutations: bit flips, nasty dwords, random
/// bytes, zeroed or duplicated ranges, truncation and appended junk.
pub fn mutate(bytes: &[u8], rng: &mut Lcg, rounds: usize) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..1 + rng.below(rounds.max(1)) {
        let len = out.len();
        match rng.below(9) {
            0 | 1 => {
                if len > 0 {
                    let at = rng.below(len);
                    out[at] ^= 1 << rng.below(8);
                }
            }
            2 => {
                if len > 0 {
                    let at = rng.below(len);
                    out[at] = rng.next() as u8;
                }
            }
            3 | 4 => {
                // a nasty dword, dword-aligned half the time (most headers are)
                if len >= 4 {
                    let mut at = rng.below(len - 3);
                    if rng.below(2) == 0 {
                        at &= !3;
                    }
                    let v = NASTY[rng.below(NASTY.len())];
                    let v = if rng.below(4) == 0 {
                        v ^ (rng.next() as u32 & 0xFF)
                    } else {
                        v
                    };
                    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            5 => {
                if len > 0 {
                    let at = rng.below(len);
                    let n = (1 + rng.below(64)).min(len - at);
                    out[at..at + n].fill(if rng.below(2) == 0 { 0 } else { 0xFF });
                }
            }
            6 => {
                if len > 0 {
                    let at = rng.below(len);
                    let n = (1 + rng.below(64)).min(len - at);
                    let chunk = out[at..at + n].to_vec();
                    let to = rng.below(len + 1);
                    out.splice(to..to, chunk);
                }
            }
            7 => {
                out.truncate(rng.below(len + 1));
            }
            _ => {
                for _ in 0..1 + rng.below(16) {
                    out.push(rng.next() as u8);
                }
            }
        }
    }
    out
}

/// Up to `max_files` private-corpus files with extension `ext` (both
/// profiles, evenly spread, at most `max_len` bytes each), as
/// `(name, bytes)`. Empty when the corpus is absent.
pub fn corpus_seeds(ext: &str, max_files: usize, max_len: u64) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for profile in ["vanilla", "mxl"] {
        let Some(root) = testutil::extracted(profile) else {
            continue;
        };
        let files: Vec<_> = testutil::find_files(&root, ext)
            .into_iter()
            .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= max_len))
            .collect();
        let step = (files.len() / max_files.max(1)).max(1);
        for path in files.iter().step_by(step).take(max_files) {
            if let Ok(bytes) = std::fs::read(path) {
                let name = path.strip_prefix(&root).unwrap_or(path).display();
                out.push((format!("{profile}:{name}"), bytes));
            }
        }
    }
    out
}

/// Feeds `iters` mutations of each corpus seed to `exercise` (which must not
/// panic) and prints what ran. Skips with a message when the corpus is absent.
pub fn run_corpus_mutations(
    what: &str,
    seeds: &[(String, Vec<u8>)],
    iters: usize,
    exercise: impl Fn(&[u8]),
) {
    if seeds.is_empty() {
        println!("skipped: no {what} corpus seeds (extracted corpus missing)");
        return;
    }
    let mut rng = Lcg(0xC0FF_EE00 ^ what.len() as u64);
    for (name, bytes) in seeds {
        exercise(bytes);
        for i in 0..iters {
            let bad = mutate(bytes, &mut rng, 4);
            // name the input if it panics
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| exercise(&bad)));
            assert!(r.is_ok(), "{what}: mutation {i} of {name} panicked");
        }
    }
    println!(
        "{what}: {} corpus seeds x {iters} mutations (CAPY_FUZZ_ITERS)",
        seeds.len()
    );
}
