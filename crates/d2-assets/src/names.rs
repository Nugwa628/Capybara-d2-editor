//! Known file names, for archives that carry no listfile (e.g. `Patch_D2.mpq`).
//!
//! Wave 1: the container only. Harvesting names from tables, DS1 and COF
//! files arrives with wave 2.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{AssetError, Result};
use crate::path::GamePath;

/// Embedded well-known paths, one per line; `#` comments and blank lines ignored.
const SEED: &str = include_str!("../../../assets/known-names/seed.txt");

#[derive(Clone, Debug, Default)]
pub struct KnownNames {
    names: BTreeSet<GamePath>,
}

impl KnownNames {
    pub fn new() -> KnownNames {
        KnownNames::default()
    }

    /// The embedded well-known paths.
    pub fn seed() -> KnownNames {
        let mut names = KnownNames::new();
        names.extend_from_listfile(SEED);
        names
    }

    /// Returns `true` when the name was not known yet.
    pub fn insert(&mut self, path: GamePath) -> bool {
        self.names.insert(path)
    }

    /// Add every name in `text` (split on `;`, CR and LF; empty entries,
    /// `#` comments and MPQ-internal `(…)` entries are skipped). Returns how
    /// many names were new.
    pub fn extend_from_listfile(&mut self, text: &str) -> usize {
        let mut added = 0;
        for entry in text.split([';', '\r', '\n']) {
            let entry = entry.trim();
            if entry.is_empty() || entry.starts_with('#') || entry.starts_with('(') {
                continue;
            }
            let path = GamePath::new(entry);
            if !path.as_str().is_empty() && self.names.insert(path) {
                added += 1;
            }
        }
        added
    }

    pub fn contains(&self, path: &GamePath) -> bool {
        self.names.contains(path)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Names in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &GamePath> {
        self.names.iter()
    }

    /// Read a names file (one name per line).
    pub fn load(path: &Path) -> Result<KnownNames> {
        let bytes = std::fs::read(path).map_err(|e| AssetError::io(path, e))?;
        let mut names = KnownNames::new();
        names.extend_from_listfile(&String::from_utf8_lossy(&bytes));
        Ok(names)
    }

    /// Write the names, one per line, sorted. Written to a temp file and
    /// renamed, so a crash never leaves a half-written list.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut text = String::new();
        for name in &self.names {
            text.push_str(name.as_str());
            text.push('\n');
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| AssetError::io(parent, e))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, text).map_err(|e| AssetError::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| AssetError::io(path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_embedded() {
        let seed = KnownNames::seed();
        assert!(seed.len() > 100, "seed has {} names", seed.len());
        assert!(seed.contains(&GamePath::new(r"data\global\excel\lvltypes.bin")));
        assert!(seed.contains(&GamePath::new(r"data\global\palette\act1\pal.dat")));
        assert!(seed.iter().all(|p| !p.as_str().starts_with('#')));
    }

    #[test]
    fn container_basics() {
        let mut names = KnownNames::new();
        assert!(names.is_empty());
        assert!(names.insert(GamePath::new("Data\\A.txt")));
        assert!(!names.insert(GamePath::new("data/a.TXT")));
        let added = names.extend_from_listfile(
            "# comment\r\ndata\\b.txt\r\n\r\n(listfile);DATA\\A.TXT;data\\c.ds1\n",
        );
        assert_eq!(added, 2);
        assert_eq!(names.len(), 3);
        assert!(names.contains(&GamePath::new("data\\c.ds1")));
        let order: Vec<_> = names.iter().map(GamePath::as_str).collect();
        assert_eq!(order, ["data\\a.txt", "data\\b.txt", "data\\c.ds1"]);
        assert!(KnownNames::seed().len() == KnownNames::seed().iter().count());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sub").join("names.txt");
        let mut names = KnownNames::new();
        names.extend_from_listfile("data\\z.ds1\ndata\\a.dt1");
        names.save(&file).expect("save");
        let back = KnownNames::load(&file).expect("load");
        assert_eq!(
            back.iter().collect::<Vec<_>>(),
            names.iter().collect::<Vec<_>>()
        );
        assert!(KnownNames::load(&dir.path().join("missing.txt")).is_err());
    }
}
