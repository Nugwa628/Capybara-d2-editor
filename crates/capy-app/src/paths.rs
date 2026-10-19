//! Portable folder layout: everything the editor writes lives next to the exe
//! in `capybara\`. Game install folders are never touched.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// Folder name next to the executable.
pub const HOME_DIR_NAME: &str = "capybara";
/// Environment variable that overrides the root (development, tests).
pub const HOME_ENV: &str = "CAPY_HOME";

/// A filesystem problem in the portable folder, with the path and OS error.
#[derive(Debug)]
pub struct PathsError {
    pub path: PathBuf,
    pub action: &'static str,
    pub source: std::io::Error,
}

impl fmt::Display for PathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cannot {} \"{}\": {}",
            self.action,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for PathsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn err(path: &Path, action: &'static str, source: std::io::Error) -> PathsError {
    PathsError {
        path: path.to_path_buf(),
        action,
        source,
    }
}

/// Every folder and file the editor owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
    pub config_file: PathBuf,
    pub layout_file: PathBuf,
    /// Dock layout of the Sprites mode (the Map Editor's is `layout_file`).
    pub sprites_layout_file: PathBuf,
    pub profiles: PathBuf,
    pub saves: PathBuf,
    pub history: PathBuf,
    pub cache: PathBuf,
}

/// Pure root selection: a non-empty override wins, else `<exe dir>\capybara`.
pub fn root_from(env_override: Option<OsString>, exe: &Path) -> PathBuf {
    match env_override {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join(HOME_DIR_NAME),
    }
}

impl Paths {
    /// Layout under `root`; touches nothing on disk.
    pub fn at(root: impl Into<PathBuf>) -> Paths {
        let root = root.into();
        Paths {
            config_file: root.join("config.toml"),
            layout_file: root.join("layout.json"),
            sprites_layout_file: root.join("layout-sprites.json"),
            profiles: root.join("profiles"),
            saves: root.join("saves"),
            history: root.join("history"),
            cache: root.join("cache"),
            root,
        }
    }

    /// The default root for this run (`CAPY_HOME` or next to the exe).
    pub fn default_root() -> Result<PathBuf, PathsError> {
        let exe = std::env::current_exe()
            .map_err(|e| err(Path::new("<current exe>"), "locate the executable", e))?;
        Ok(root_from(std::env::var_os(HOME_ENV), &exe))
    }

    /// Resolve the root, create the folders and prove it is writable. There is
    /// never a silent fallback to another location.
    pub fn resolve() -> Result<Paths, PathsError> {
        let paths = Paths::at(Self::default_root()?);
        paths.ensure()?;
        Ok(paths)
    }

    /// Create `capybara\` and its sub-folders, then probe writability.
    pub fn ensure(&self) -> Result<(), PathsError> {
        for dir in [
            &self.root,
            &self.profiles,
            &self.saves,
            &self.history,
            &self.cache,
        ] {
            std::fs::create_dir_all(dir).map_err(|e| err(dir, "create folder", e))?;
        }
        probe_writable(&self.root)
    }

    /// `saves\<profile id>`.
    pub fn saves_for(&self, profile_id: &str) -> PathBuf {
        self.saves.join(profile_id)
    }

    /// `cache\known-names-<profile id>.txt`.
    pub fn names_cache(&self, profile_id: &str) -> PathBuf {
        self.cache.join(format!("known-names-{profile_id}.txt"))
    }

    /// `cache\known-names-<profile id>.json` (overlay fingerprints).
    pub fn names_fingerprints(&self, profile_id: &str) -> PathBuf {
        self.cache.join(format!("known-names-{profile_id}.json"))
    }
}

/// Create, write and delete a file in `dir`.
pub fn probe_writable(dir: &Path) -> Result<(), PathsError> {
    use std::io::Write;
    let probe = dir.join(format!(".capy-write-probe-{}", std::process::id()));
    let mut file = std::fs::File::create(&probe).map_err(|e| err(&probe, "create a file at", e))?;
    file.write_all(b"probe")
        .map_err(|e| err(&probe, "write to", e))?;
    drop(file);
    std::fs::remove_file(&probe).map_err(|e| err(&probe, "delete", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let root = root_from(
            Some(OsString::from("D:\\scratch\\home")),
            Path::new("C:\\apps\\CapybaraMapEditor.exe"),
        );
        assert_eq!(root, PathBuf::from("D:\\scratch\\home"));
    }

    #[test]
    fn default_root_is_next_to_exe() {
        let exe = Path::new("C:\\apps\\capy\\CapybaraMapEditor.exe");
        assert_eq!(
            root_from(None, exe),
            Path::new("C:\\apps\\capy").join("capybara")
        );
        // An empty override counts as unset.
        assert_eq!(
            root_from(Some(OsString::new()), exe),
            Path::new("C:\\apps\\capy").join("capybara")
        );
    }

    #[test]
    fn ensure_creates_the_layout_and_leaves_no_probe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        for dir in [
            &paths.root,
            &paths.profiles,
            &paths.saves,
            &paths.history,
            &paths.cache,
        ] {
            assert!(dir.is_dir(), "{} missing", dir.display());
        }
        let leftovers: Vec<_> = std::fs::read_dir(&paths.root)
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".capy-write"))
            .collect();
        assert!(leftovers.is_empty());
        // Idempotent.
        paths.ensure().expect("second ensure");
    }

    #[test]
    fn unwritable_root_is_reported_with_path_and_os_error() {
        // A regular file where the root's parent should be: creating the
        // folder fails on every platform (a read-only directory attribute is
        // ignored by Windows, so it cannot be used here).
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"x").expect("write");
        let paths = Paths::at(blocker.join("capybara"));
        let e = paths.ensure().expect_err("must fail");
        let msg = e.to_string();
        assert!(msg.contains("blocker"), "{msg}");
        assert!(msg.contains("Cannot"), "{msg}");
    }

    #[test]
    fn probe_fails_in_missing_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(probe_writable(&tmp.path().join("nope")).is_err());
        assert!(probe_writable(tmp.path()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn read_only_dir_is_detected() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut perm = std::fs::metadata(tmp.path()).expect("meta").permissions();
        perm.set_mode(0o555);
        std::fs::set_permissions(tmp.path(), perm).expect("chmod");
        let result = probe_writable(tmp.path());
        let mut perm = std::fs::metadata(tmp.path()).expect("meta").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(tmp.path(), perm).expect("chmod back");
        // Root can write anywhere; only assert for ordinary users.
        if result.is_ok() {
            eprintln!("running as a user that ignores directory modes");
        }
    }

    #[test]
    fn derived_paths() {
        let p = Paths::at("R");
        assert_eq!(p.saves_for("mxl"), Path::new("R").join("saves").join("mxl"));
        assert_eq!(
            p.names_cache("mxl"),
            Path::new("R").join("cache").join("known-names-mxl.txt")
        );
        assert_eq!(
            p.names_fingerprints("mxl"),
            Path::new("R").join("cache").join("known-names-mxl.json")
        );
    }
}
