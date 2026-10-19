//! Asset sources: one layer of game data (MPQ archive, loose folder, CASC).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::{AssetError, Result};
use crate::mpq::MpqArchive;
use crate::path::GamePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    Mpq,
    Folder,
    Casc,
}

/// Cache key for an archive or folder. Folders aggregate (file count, total
/// size, newest mtime).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub path: PathBuf,
    pub size: u64,
    pub modified_unix_ms: i64,
    pub entries: u64,
}

pub trait AssetSource: Send + Sync {
    /// Display name, e.g. `medianxl-dGlsZXM.mpq` or `saves\mxl`.
    fn name(&self) -> &str;
    fn kind(&self) -> SourceKind;
    fn contains(&self, path: &GamePath) -> bool;
    /// File contents, `None` when the source does not have the file.
    fn read(&self, path: &GamePath) -> Result<Option<Vec<u8>>>;
    /// Names the source can enumerate on its own (listfile, directory walk).
    fn list(&self) -> Result<Vec<GamePath>>;
    fn fingerprint(&self) -> Result<SourceFingerprint>;
}

fn unix_ms(time: SystemTime) -> i64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |v| -v),
    }
}

/// An MPQ archive as a layer.
pub struct MpqSource {
    archive: MpqArchive,
    name: String,
}

impl MpqSource {
    pub fn open(path: &Path) -> Result<MpqSource> {
        let archive = MpqArchive::open(path)?;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Ok(MpqSource { archive, name })
    }

    pub fn archive(&self) -> &MpqArchive {
        &self.archive
    }
}

impl AssetSource for MpqSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Mpq
    }

    fn contains(&self, path: &GamePath) -> bool {
        self.archive.contains(path.as_str())
    }

    fn read(&self, path: &GamePath) -> Result<Option<Vec<u8>>> {
        self.archive.read(path.as_str())
    }

    /// Existing files named by the archive's `(listfile)`; empty without one.
    fn list(&self) -> Result<Vec<GamePath>> {
        let Some(names) = self.archive.listfile()? else {
            return Ok(Vec::new());
        };
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if name.starts_with('(') || !self.archive.contains(&name) {
                continue;
            }
            let path = GamePath::new(&name);
            if seen.insert(path.clone()) {
                out.push(path);
            }
        }
        Ok(out)
    }

    fn fingerprint(&self) -> Result<SourceFingerprint> {
        let path = self.archive.path();
        let meta = fs::metadata(path).map_err(|e| AssetError::io(path, e))?;
        Ok(SourceFingerprint {
            path: path.to_path_buf(),
            size: meta.len(),
            modified_unix_ms: meta.modified().map(unix_ms).unwrap_or(0),
            entries: self.archive.file_count() as u64,
        })
    }
}

/// Loose folder. A missing root is an empty source, not an error.
pub struct FolderSource {
    root: PathBuf,
    name: String,
}

impl FolderSource {
    pub fn new(root: &Path, name: impl Into<String>) -> FolderSource {
        FolderSource {
            root: root.to_path_buf(),
            name: name.into(),
        }
    }

    /// Native path for `path`, refusing anything that could leave the root.
    fn native(&self, path: &GamePath) -> Option<PathBuf> {
        let escapes = path
            .as_str()
            .split('\\')
            .any(|part| part == "." || part == ".." || part.contains(':'));
        (!escapes && !path.as_str().is_empty()).then(|| path.to_native(&self.root))
    }

    /// Every file under the root as `(game path, size, mtime)`.
    fn walk(&self) -> Result<Vec<(GamePath, u64, i64)>> {
        let mut out = Vec::new();
        let mut stack: Vec<(PathBuf, String)> = vec![(self.root.clone(), String::new())];
        while let Some((dir, prefix)) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(AssetError::io(&dir, e)),
            };
            for entry in entries {
                let entry = entry.map_err(|e| AssetError::io(&dir, e))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let meta = entry
                    .metadata()
                    .map_err(|e| AssetError::io(entry.path(), e))?;
                let rel = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}\\{name}")
                };
                if meta.is_dir() {
                    stack.push((entry.path(), rel));
                } else if meta.is_file() {
                    let mtime = meta.modified().map(unix_ms).unwrap_or(0);
                    out.push((GamePath::new(&rel), meta.len(), mtime));
                }
            }
        }
        Ok(out)
    }
}

impl AssetSource for FolderSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Folder
    }

    fn contains(&self, path: &GamePath) -> bool {
        self.native(path).is_some_and(|p| p.is_file())
    }

    fn read(&self, path: &GamePath) -> Result<Option<Vec<u8>>> {
        let Some(native) = self.native(path) else {
            return Ok(None);
        };
        match fs::read(&native) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(None)
            }
            // A directory with the file's name is not a file.
            Err(_) if native.is_dir() => Ok(None),
            Err(e) => Err(AssetError::io(native, e)),
        }
    }

    fn list(&self) -> Result<Vec<GamePath>> {
        let mut out: Vec<GamePath> = self.walk()?.into_iter().map(|(p, _, _)| p).collect();
        out.sort();
        Ok(out)
    }

    fn fingerprint(&self) -> Result<SourceFingerprint> {
        let files = self.walk()?;
        Ok(SourceFingerprint {
            path: self.root.clone(),
            size: files.iter().map(|f| f.1).sum(),
            modified_unix_ms: files.iter().map(|f| f.2).max().unwrap_or(0),
            entries: files.len() as u64,
        })
    }
}

/// D2R CASC. Stub until a D2R install exists.
// TODO(casc): integration test against a real D2R install.
pub struct CascSource;

impl CascSource {
    pub fn open(_root: &Path) -> Result<CascSource> {
        Err(AssetError::Unsupported("CASC"))
    }
}

impl AssetSource for CascSource {
    fn name(&self) -> &str {
        "casc"
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Casc
    }

    fn contains(&self, _path: &GamePath) -> bool {
        false
    }

    fn read(&self, _path: &GamePath) -> Result<Option<Vec<u8>>> {
        Err(AssetError::Unsupported("CASC"))
    }

    fn list(&self) -> Result<Vec<GamePath>> {
        Err(AssetError::Unsupported("CASC"))
    }

    fn fingerprint(&self) -> Result<SourceFingerprint> {
        Err(AssetError::Unsupported("CASC"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casc_is_unsupported() {
        assert!(matches!(
            CascSource::open(Path::new("x")),
            Err(AssetError::Unsupported("CASC"))
        ));
    }

    #[test]
    fn folder_missing_root_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = FolderSource::new(&dir.path().join("nope"), "saves\\x");
        let p = GamePath::new("data\\a.ds1");
        assert!(!src.contains(&p));
        assert_eq!(src.read(&p).expect("read"), None);
        assert!(src.list().expect("list").is_empty());
        let fp = src.fingerprint().expect("fp");
        assert_eq!((fp.size, fp.entries, fp.modified_unix_ms), (0, 0, 0));
    }

    #[test]
    fn folder_reads_lists_and_refuses_escapes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        fs::create_dir_all(root.join("data").join("global")).expect("mkdir");
        fs::write(root.join("data").join("global").join("a.txt"), b"hello").expect("write");
        fs::write(dir.path().join("secret.txt"), b"secret").expect("write");
        let src = FolderSource::new(&root, "t");
        let a = GamePath::new("Data\\Global\\A.TXT");
        assert!(src.contains(&a));
        assert_eq!(src.read(&a).expect("read"), Some(b"hello".to_vec()));
        assert_eq!(
            src.list().expect("list"),
            vec![GamePath::new("data\\global\\a.txt")]
        );
        assert!(!src.contains(&GamePath::new("data")));
        assert_eq!(src.read(&GamePath::new("data")).expect("dir"), None);
        let escape = GamePath::new("..\\secret.txt");
        assert!(!src.contains(&escape));
        assert_eq!(src.read(&escape).expect("escape"), None);
        let fp = src.fingerprint().expect("fp");
        assert_eq!((fp.size, fp.entries), (5, 1));
        assert!(fp.modified_unix_ms > 0);
    }
}
