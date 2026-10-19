//! Canonical game-relative paths.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Canonical game-relative path: ASCII-lowercase, `\` separators, no leading
/// separator. Example: `data\global\tiles\act1\town\floor.dt1`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GamePath(String);

/// Lowercase, turn `/` into `\`, collapse repeated separators, drop leading ones.
fn canonical(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        let ch = if ch == '/' {
            '\\'
        } else {
            ch.to_ascii_lowercase()
        };
        if ch == '\\' && (out.is_empty() || out.ends_with('\\')) {
            continue;
        }
        out.push(ch);
    }
    out
}

impl GamePath {
    pub fn new(path: &str) -> GamePath {
        GamePath(canonical(path))
    }

    /// For DS1 file lists and LvlTypes/LvlPrest `File` columns: strips `C:`,
    /// a leading `\d2\` and leading separators, maps `.tg1` to `.dt1`; a path
    /// that does not start with `data\` is taken under `data\global\tiles\`.
    /// `None` for empty or "0".
    pub fn from_tile_ref(raw: &str) -> Option<GamePath> {
        let mut s = canonical(raw.trim());
        if s.is_empty() || s == "0" {
            return None;
        }
        // Drive letter ("c:"), then any separators that follow it.
        let bytes = s.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            s = canonical(&s[2..]);
        }
        if let Some(rest) = s.strip_prefix("d2\\") {
            s = canonical(rest);
        }
        if s.is_empty() {
            return None;
        }
        if let Some(stem) = s.strip_suffix(".tg1") {
            s = format!("{stem}.dt1");
        }
        if !s.starts_with("data\\") {
            s = format!("data\\global\\tiles\\{s}");
        }
        Some(GamePath(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part after the last separator.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('\\').next().unwrap_or(&self.0)
    }

    /// Extension without the dot, if the file name has one.
    pub fn extension(&self) -> Option<&str> {
        self.file_name().rsplit_once('.').map(|(_, ext)| ext)
    }

    /// `root` joined with the path's components (for loose folders and saves).
    pub fn to_native(&self, root: &Path) -> PathBuf {
        let mut out = root.to_path_buf();
        for part in self.0.split('\\').filter(|p| !p.is_empty()) {
            out.push(part);
        }
        out
    }
}

impl fmt::Display for GamePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for GamePath {
    fn from(value: &str) -> GamePath {
        GamePath::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_canonicalises() {
        assert_eq!(
            GamePath::new("/Data/Global/X.DT1").as_str(),
            "data\\global\\x.dt1"
        );
        assert_eq!(GamePath::new("\\\\Data//a\\\\B").as_str(), "data\\a\\b");
    }

    #[test]
    fn tile_refs() {
        let t = |s: &str| GamePath::from_tile_ref(s).map(|p| p.to_string());
        assert_eq!(
            t("\\d2\\data\\global\\tiles\\act1\\town\\floor.tg1").as_deref(),
            Some("data\\global\\tiles\\act1\\town\\floor.dt1")
        );
        assert_eq!(
            t("C:\\D2\\data\\global\\tiles\\x.dt1").as_deref(),
            Some("data\\global\\tiles\\x.dt1")
        );
        assert_eq!(
            t("ACT1\\TOWN\\floor.dt1").as_deref(),
            Some("data\\global\\tiles\\act1\\town\\floor.dt1")
        );
        assert_eq!(
            t("data\\global\\tiles\\a.tg1").as_deref(),
            Some("data\\global\\tiles\\a.dt1")
        );
        assert_eq!(t("0"), None);
        assert_eq!(t(""), None);
        assert_eq!(t("  "), None);
        assert_eq!(t("C:\\"), None);
    }

    #[test]
    fn parts() {
        let p = GamePath::new("data\\global\\excel\\Levels.bin");
        assert_eq!(p.file_name(), "levels.bin");
        assert_eq!(p.extension(), Some("bin"));
        assert_eq!(GamePath::new("readme").extension(), None);
        assert_eq!(
            p.to_native(Path::new("root")),
            Path::new("root")
                .join("data")
                .join("global")
                .join("excel")
                .join("levels.bin")
        );
    }
}
