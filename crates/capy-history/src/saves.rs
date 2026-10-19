//! `capybara\saves\<profile>\`: edited maps mirroring game paths, plus `CHANGED.txt`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use d2_assets::GamePath;

use crate::atomic::atomic_write;
use crate::error::{HistoryError, Result};

const CHANGED_FILE: &str = "CHANGED.txt";

/// Serialises every read-modify-write of a `CHANGED.txt` in this process:
/// the app saves from the main thread and from batch workers, and two
/// unsynchronised updates would each drop the other's entry.
static CHANGED_LOCK: Mutex<()> = Mutex::new(());

fn changed_lock() -> MutexGuard<'static, ()> {
    // A panic while holding the lock cannot leave the file half-written
    // (writes are atomic), so a poisoned lock is still usable.
    CHANGED_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The saves folder of one profile.
#[derive(Clone, Debug)]
pub struct SavesDir {
    root: PathBuf,
}

impl SavesDir {
    /// `saves_root` is `capybara\saves`; the profile gets its own subfolder.
    pub fn new(saves_root: &Path, profile_id: &str) -> SavesDir {
        SavesDir {
            root: saves_root.join(profile_id),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `root` + the game path's components (lowercase, as `GamePath` stores
    /// them). `BadGamePath` for `..`, drive letters, empty components or
    /// characters Windows does not allow in a file name.
    pub fn path_for(&self, game_path: &GamePath) -> Result<PathBuf> {
        let s = game_path.as_str();
        let bad = || HistoryError::BadGamePath(s.to_string());
        if s.is_empty() {
            return Err(bad());
        }
        let mut out = self.root.clone();
        for part in s.split('\\') {
            if !valid_component(part) {
                return Err(bad());
            }
            out.push(part);
        }
        Ok(out)
    }

    /// Creates `root` if needed and proves it is writable (writes and deletes a probe file).
    pub fn check_writable(&self) -> Result<()> {
        let not_writable = |path: &Path, e: std::io::Error| HistoryError::NotWritable {
            path: path.to_path_buf(),
            detail: e.to_string(),
        };
        fs::create_dir_all(&self.root).map_err(|e| not_writable(&self.root, e))?;
        let probe = self
            .root
            .join(format!(".capy-write-probe-{}", std::process::id()));
        fs::write(&probe, b"probe").map_err(|e| not_writable(&probe, e))?;
        fs::remove_file(&probe).map_err(|e| not_writable(&probe, e))?;
        Ok(())
    }

    /// `atomic_write` to `path_for(game_path)`, then add the game path to
    /// `CHANGED.txt` if it is not already listed (rewritten atomically).
    /// Safe to call from several threads at once: no entry is lost.
    pub fn save(&self, game_path: &GamePath, bytes: &[u8]) -> Result<PathBuf> {
        let target = self.path_for(game_path)?;
        atomic_write(&target, bytes)?;
        let _guard = changed_lock();
        let mut changed = self.changed()?;
        if !changed.contains(game_path) {
            changed.push(game_path.clone());
            self.write_changed(&changed)?;
        }
        Ok(target)
    }

    /// `CHANGED.txt` entries in insertion order (one canonical game path per line, CRLF).
    pub fn changed(&self) -> Result<Vec<GamePath>> {
        let path = self.changed_path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(HistoryError::io(&path, e)),
        };
        let mut out: Vec<GamePath> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let gp = GamePath::new(line);
            if !out.contains(&gp) {
                out.push(gp);
            }
        }
        Ok(out)
    }

    /// "Mark as packed": clears `CHANGED.txt` (atomically; file left empty).
    pub fn mark_packed(&self) -> Result<()> {
        let _guard = changed_lock();
        atomic_write(&self.changed_path(), b"")
    }

    fn changed_path(&self) -> PathBuf {
        self.root.join(CHANGED_FILE)
    }

    fn write_changed(&self, list: &[GamePath]) -> Result<()> {
        let mut text = String::new();
        for gp in list {
            text.push_str(gp.as_str());
            text.push_str("\r\n");
        }
        atomic_write(&self.changed_path(), text.as_bytes())
    }
}

fn valid_component(part: &str) -> bool {
    if part.is_empty() || part == "." || part == ".." {
        return false;
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return false;
    }
    !part
        .chars()
        .any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*' | '/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp(s: &str) -> GamePath {
        GamePath::new(s)
    }

    #[test]
    fn path_for_maps_components_and_rejects_escapes() {
        let dir = SavesDir::new(Path::new("saves"), "mxl");
        let p = dir.path_for(&gp("Data\\Global\\Tiles\\a.ds1")).unwrap();
        assert_eq!(
            p,
            Path::new("saves")
                .join("mxl")
                .join("data")
                .join("global")
                .join("tiles")
                .join("a.ds1")
        );
        for bad in [
            "data\\..\\x.ds1",
            "..\\x",
            "C:\\x.ds1",
            "c:x.ds1",
            "data\\.\\x",
            "",
            "a\\b?.ds1",
            "a\\b.",
        ] {
            assert!(
                matches!(dir.path_for(&gp(bad)), Err(HistoryError::BadGamePath(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn concurrent_saves_keep_every_changed_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = SavesDir::new(tmp.path(), "vanilla");
        std::thread::scope(|scope| {
            for t in 0..8 {
                let dir = &dir;
                scope.spawn(move || {
                    for i in 0..25 {
                        let path = gp(&format!(r"data\global\t{t}\f{i}.ds1"));
                        dir.save(&path, b"x").unwrap();
                    }
                });
            }
        });
        let changed = dir.changed().unwrap();
        assert_eq!(changed.len(), 8 * 25, "an entry was lost");
    }
}
