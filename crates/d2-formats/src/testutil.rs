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
