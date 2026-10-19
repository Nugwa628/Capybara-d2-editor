//! Crash-safe file replacement.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{HistoryError, Result};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to a temp file in the same directory, flush + sync it, then
/// rename over `path` (replace-existing). Creates parent directories. A crash
/// at any point leaves either the old file or the new file, never a mix.
/// Stale temp files from a crash are named `.<file>.capytmp-*` and are
/// cleaned by the next write to the same file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            HistoryError::io(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name"),
            )
        })?;
    let dir: PathBuf = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&dir).map_err(|e| HistoryError::io(&dir, e))?;

    let prefix = format!(".{file_name}.capytmp-");
    remove_stale_temps(&dir, &prefix);

    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!("{prefix}{}-{n}", std::process::id()));
    if let Err(e) = write_synced(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(HistoryError::io(&tmp, e));
    }
    if let Err(e) = rename_replace(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(HistoryError::io(path, e));
    }
    sync_dir(&dir);
    Ok(())
}

fn write_synced(tmp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(tmp)?;
    f.write_all(bytes)?;
    f.flush()?;
    f.sync_all()
}

/// `fs::rename` replaces an existing file on both Windows and Unix. On
/// Windows a transient sharing violation (antivirus, indexer) is retried.
fn rename_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && attempt < 8 => {
                if e.kind() != std::io::ErrorKind::PermissionDenied {
                    return Err(e);
                }
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(10 * attempt));
            }
            Err(e) => return Err(e),
        }
    }
}

fn remove_stale_temps(dir: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Persist the rename itself (directory entry) where the platform allows it.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwrites_and_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("b").join("f.bin");
        atomic_write(&p, b"one").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"one");
        atomic_write(&p, b"two-longer").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two-longer");
        atomic_write(&p, b"x").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"x");
    }

    #[test]
    fn cleans_stale_temps_for_same_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        fs::write(dir.path().join(".f.bin.capytmp-1-1"), b"stale").unwrap();
        fs::write(dir.path().join(".g.bin.capytmp-1-1"), b"other").unwrap();
        atomic_write(&p, b"new").unwrap();
        assert!(!dir.path().join(".f.bin.capytmp-1-1").exists());
        assert!(dir.path().join(".g.bin.capytmp-1-1").exists());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers.len(), 2, "{leftovers:?}");
    }

    #[test]
    fn empty_payload_and_missing_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e");
        atomic_write(&p, b"").unwrap();
        assert_eq!(fs::read(&p).unwrap().len(), 0);
        assert!(atomic_write(Path::new(".."), b"x").is_err());
    }
}
