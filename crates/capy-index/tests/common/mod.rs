//! Shared by the heavy integration tests: one persistent Median XL index
//! under `CARGO_TARGET_TMPDIR`, built once and reused by every later run.
//!
//! `real_data.rs` and `p3_queries.rs` are separate test binaries, so they can
//! run at the same time. A lock file (`build.lock`, created exclusively)
//! serialises "check the status, build when needed". Whoever gets the lock
//! second finds the index `UpToDate` and skips the build. A lock older than
//! [`STALE_LOCK`] belongs to a killed run and is taken over. A schema bump or
//! a changed fingerprint (game files, generator version) still rebuilds.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime};

use capy_index::{BuildReport, Index, IndexStatus};
use d2_assets::{Overlay, Profile};

/// A lock this old is left over from a run that was killed mid-build.
const STALE_LOCK: Duration = Duration::from_secs(30 * 60);
const RETRY_EVERY: Duration = Duration::from_millis(250);

/// Where the shared Median XL index lives.
pub fn shared_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("capy-index-mxl")
}

/// Holds `build.lock` until dropped.
struct Lock(PathBuf);

impl Lock {
    fn acquire(dir: &Path) -> Lock {
        let path = dir.join("build.lock");
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Lock(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok());
                    if age.is_some_and(|a| a > STALE_LOCK) {
                        println!("  removing a stale build lock ({age:?} old)");
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    assert!(
                        started.elapsed() < STALE_LOCK * 2,
                        "waited too long for {}",
                        path.display()
                    );
                    std::thread::sleep(RETRY_EVERY);
                }
                Err(e) => panic!("cannot create {}: {e}", path.display()),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub struct Shared {
    pub idx: Index,
    pub overlay: Overlay,
    /// `Some` when this run built the index, `None` when it reused it.
    pub report: Option<BuildReport>,
}

/// Open the shared Median XL index, building it first when it is missing or
/// stale. Prints `built` or `reused` with the time spent.
pub fn shared_mxl(profile: &Profile) -> Shared {
    let dir = shared_dir();
    fs::create_dir_all(&dir).unwrap();
    // The saves layer is empty and never indexed; it only has to exist.
    let saves = dir.join("saves");
    fs::create_dir_all(&saves).unwrap();
    let overlay = profile.open(&saves).unwrap();

    let started = Instant::now();
    let _lock = Lock::acquire(&dir);
    let waited = started.elapsed();
    let mut idx = Index::open(&dir).unwrap();
    let status = idx.status(&profile.id, &overlay).unwrap();
    if status == IndexStatus::UpToDate {
        println!(
            "mxl index: reused ({:?} to open and check, {waited:?} of it waiting for the lock) at {}",
            started.elapsed(),
            dir.display()
        );
        return Shared {
            idx,
            overlay,
            report: None,
        };
    }
    println!("mxl index: {status:?}, building");
    let build_started = Instant::now();
    let report = idx
        .build(profile, &overlay, &mut |_| {}, &AtomicBool::new(false))
        .unwrap();
    println!(
        "mxl index: built in {:?} ({waited:?} waiting for the lock): {} files, {} assets ({} tiles), {} warnings",
        build_started.elapsed(),
        report.files,
        report.assets,
        report.tiles,
        report.warnings.len()
    );
    for w in report.warnings.iter().take(12) {
        println!("  warning: {w}");
    }
    assert_eq!(
        idx.status(&profile.id, &overlay).unwrap(),
        IndexStatus::UpToDate
    );
    Shared {
        idx,
        overlay,
        report: Some(report),
    }
}
