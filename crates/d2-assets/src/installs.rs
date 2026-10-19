//! Game installs and the default profiles: what kind of install a folder is,
//! where the Windows registry says Diablo II lives (read-only), first-run
//! seeding of `capybara\profiles\<id>.toml`, the start-up profile choice, and
//! writing a new profile for an install the user adds.
//!
//! Shared by the GUI and the CLI so either one can be the first to run on a
//! clean `capybara\` folder. Install folders are only ever read (directory
//! listings); the only files written are profile TOMLs in the profiles folder
//! the caller names, and never inside an install.

use std::path::{Path, PathBuf};

use crate::error::{AssetError, Result};
use crate::profile::{GameVersion, LayerSpec, Profile};

/// Where vanilla Diablo II installs by default (the 32-bit installer's
/// location on 64-bit Windows). Used whenever it is a folder.
pub const DEFAULT_VANILLA_INSTALL: &str = r"C:\Program Files (x86)\Diablo II";
/// A Median XL location probed on first run. It is only an auto-detect
/// candidate (Median XL has no standard install folder).
pub const DEFAULT_MXL_INSTALL: &str = r"C:\games\median-xl";

/// The registry keys holding Diablo II's `InstallPath` (32-bit installer on
/// 64-bit Windows, then the per-user key).
pub const REGISTRY_KEYS: [(RegistryRoot, &str); 2] = [
    (
        RegistryRoot::LocalMachine,
        r"SOFTWARE\WOW6432Node\Blizzard Entertainment\Diablo II",
    ),
    (
        RegistryRoot::CurrentUser,
        r"Software\Blizzard Entertainment\Diablo II",
    ),
];
/// The registry value naming the install folder.
pub const REGISTRY_VALUE: &str = "InstallPath";

/// A registry hive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryRoot {
    LocalMachine,
    CurrentUser,
}

impl RegistryRoot {
    /// `HKLM` / `HKCU`.
    pub fn label(self) -> &'static str {
        match self {
            RegistryRoot::LocalMachine => "HKLM",
            RegistryRoot::CurrentUser => "HKCU",
        }
    }
}

/// What a folder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstallKind {
    /// `d2data.mpq` and `d2exp.mpq`.
    Vanilla,
    /// At least one `medianxl-*.mpq`.
    MedianXl,
    /// Neither: the user has to say.
    Unknown,
}

impl InstallKind {
    pub fn label(self) -> &'static str {
        match self {
            InstallKind::Vanilla => "Diablo II (vanilla)",
            InstallKind::MedianXl => "Median XL",
            InstallKind::Unknown => "unknown",
        }
    }

    /// The id a new profile of this kind starts from.
    pub fn base_id(self) -> &'static str {
        match self {
            InstallKind::MedianXl => "mxl",
            InstallKind::Vanilla | InstallKind::Unknown => "vanilla",
        }
    }
}

/// Looks at the file names directly inside `dir` (case-insensitive). Median
/// XL wins because it also ships its own `d2data.mpq` and `d2exp.mpq`. A
/// missing or unreadable folder is [`InstallKind::Unknown`].
pub fn detect_kind(dir: &Path) -> InstallKind {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return InstallKind::Unknown;
    };
    let names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
        .collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    if names
        .iter()
        .any(|n| n.starts_with("medianxl-") && n.ends_with(".mpq"))
    {
        InstallKind::MedianXl
    } else if has("d2data.mpq") && has("d2exp.mpq") {
        InstallKind::Vanilla
    } else {
        InstallKind::Unknown
    }
}

/// Cleans a registry `InstallPath` string: trims NULs, whitespace, quotes
/// and a trailing backslash. Empty or relative values are no path.
pub fn install_path_from_value(value: &str) -> Option<PathBuf> {
    let t = value
        .trim_matches(|c: char| c == '\0' || c.is_whitespace() || c == '"')
        .trim_end_matches(['\\', '/']);
    if t.is_empty() {
        return None;
    }
    // "C:" alone would mean "the current folder of drive C".
    let t = if t.len() == 2 && t.ends_with(':') {
        format!("{t}\\")
    } else {
        t.to_owned()
    };
    let path = PathBuf::from(t);
    path.is_absolute().then_some(path)
}

/// One place an install may be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    /// Where it came from, e.g. `registry (HKLM)` or `default location`.
    pub source: String,
}

/// The `InstallPath` values of the Diablo II registry keys ([`REGISTRY_KEYS`]),
/// opened for reading only. Missing keys and values are skipped; duplicates
/// ([`same_path`]) are dropped. Always empty off Windows.
#[cfg(windows)]
pub fn registry_install_paths() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for (root, key) in REGISTRY_KEYS {
        let hive = match root {
            RegistryRoot::LocalMachine => windows_registry::LOCAL_MACHINE,
            RegistryRoot::CurrentUser => windows_registry::CURRENT_USER,
        };
        // `Key::open` asks for read access only (KEY_READ).
        let Ok(k) = hive.open(key) else { continue };
        let Ok(value) = k.get_string(REGISTRY_VALUE) else {
            continue;
        };
        if let Some(path) = install_path_from_value(&value)
            && !out.iter().any(|c| same_path(&c.path, &path))
        {
            out.push(Candidate {
                path,
                source: format!("registry ({})", root.label()),
            });
        }
    }
    out
}

/// The `InstallPath` values of the Diablo II registry keys: none off Windows.
#[cfg(not(windows))]
pub fn registry_install_paths() -> Vec<Candidate> {
    Vec::new()
}

/// Case-insensitive path equality, ignoring a trailing separator and the
/// separator style.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// First-run candidates: the default vanilla location, then the registry's
/// paths; and the default Median XL location.
pub fn first_run_candidates(
    default_vanilla: &Path,
    default_mxl: &Path,
    registry: &[Candidate],
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut vanilla = vec![default_vanilla.to_path_buf()];
    for c in registry {
        if !vanilla.iter().any(|v| same_path(v, &c.path)) {
            vanilla.push(c.path.clone());
        }
    }
    (vanilla, vec![default_mxl.to_path_buf()])
}

/// Where first-run seeding looks: the two default locations and the registry
/// candidates. [`InstallSearch::system`] is the real machine; tests build one
/// by hand so they never depend on the registry or on installed games.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallSearch {
    /// Used for `vanilla.toml` whenever it is a folder.
    pub default_vanilla: PathBuf,
    /// Used for `mxl.toml` whenever it is a folder.
    pub default_mxl: PathBuf,
    /// Used for `vanilla.toml` (after the default) when the folder really
    /// holds vanilla's archives ([`detect_kind`]).
    pub registry: Vec<Candidate>,
}

impl InstallSearch {
    /// [`DEFAULT_VANILLA_INSTALL`], [`DEFAULT_MXL_INSTALL`] and
    /// [`registry_install_paths`] (read-only).
    pub fn system() -> InstallSearch {
        InstallSearch {
            default_vanilla: PathBuf::from(DEFAULT_VANILLA_INSTALL),
            default_mxl: PathBuf::from(DEFAULT_MXL_INSTALL),
            registry: registry_install_paths(),
        }
    }

    /// The `(vanilla, mxl)` lists [`seed_profiles`] takes
    /// ([`first_run_candidates`]).
    pub fn candidates(&self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        first_run_candidates(&self.default_vanilla, &self.default_mxl, &self.registry)
    }
}

/// Whether `dir` holds at least one profile (`*.toml` file, any case), the
/// files [`Profile::load_dir`] reads. A missing folder holds none.
pub fn has_profile_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.filter_map(|e| e.ok()).any(|e| {
            let p = e.path();
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
                && p.is_file()
        })
    })
}

/// First run: write `vanilla.toml` / `mxl.toml` for the first candidate of
/// each that exists. The first vanilla candidate (the default location) is
/// used whenever it is a folder; later ones (the registry's) only when they
/// really hold vanilla's archives. The first Median XL candidate that is a
/// folder is used. Existing files are never overwritten, and nothing is
/// written when the profiles folder lies inside the chosen install. The
/// profiles folder is created when something is written. Returns the files
/// written.
pub fn seed_profiles(
    profiles_dir: &Path,
    vanilla: &[PathBuf],
    mxl: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let pick_vanilla = vanilla
        .iter()
        .enumerate()
        .find(|(i, p)| p.is_dir() && (*i == 0 || detect_kind(p) == InstallKind::Vanilla))
        .map(|(_, p)| p.clone());
    let pick_mxl = mxl.iter().find(|p| p.is_dir()).cloned();
    let candidates = [
        pick_vanilla.map(|p| ("vanilla", Profile::vanilla(&p), p)),
        pick_mxl.map(|p| ("mxl", Profile::median_xl(&p), p)),
    ];
    for (id, profile, install) in candidates.into_iter().flatten() {
        let file = profiles_dir.join(format!("{id}.toml"));
        if file.exists() || is_inside(profiles_dir, &install) {
            continue;
        }
        std::fs::create_dir_all(profiles_dir).map_err(|e| AssetError::io(profiles_dir, e))?;
        write_atomic(&file, profile.to_toml().as_bytes())?;
        written.push(file);
    }
    Ok(written)
}

/// [`seed_profiles`] for a folder that has no profile yet
/// ([`has_profile_files`]): `search` is called (so the registry is read)
/// only then. Returns the files written; empty when profiles already exist
/// or no install was found. This is what the CLI does before every verb
/// that uses the `capybara\` folder.
pub fn seed_default_profiles(
    profiles_dir: &Path,
    search: impl FnOnce() -> InstallSearch,
) -> Result<Vec<PathBuf>> {
    if has_profile_files(profiles_dir) {
        return Ok(Vec::new());
    }
    let (vanilla, mxl) = search().candidates();
    seed_profiles(profiles_dir, &vanilla, &mxl)
}

/// The profile opened at start-up: the wanted one (last session or
/// `CAPY_PROFILE`) when it exists, else `vanilla`, else the first by id.
pub fn startup_profile(profiles: &[Profile], wanted: Option<&str>) -> Option<String> {
    if let Some(w) = wanted
        && profiles.iter().any(|p| p.id == w)
    {
        return Some(w.to_owned());
    }
    if profiles.iter().any(|p| p.id == "vanilla") {
        return Some("vanilla".to_owned());
    }
    profiles.iter().map(|p| p.id.clone()).min()
}

/// `base`, else `base-2`, `base-3`, ... whichever is not taken
/// (case-insensitive).
pub fn unique_id(taken: &[String], base: &str) -> String {
    let free = |id: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(id));
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| free(id))
        .unwrap_or_else(|| base.to_owned())
}

/// Ids of the profiles that already read archives or folders in `dir`.
pub fn profiles_using(profiles: &[Profile], dir: &Path) -> Vec<String> {
    let inside = |p: &Path| same_path(p, dir) || p.parent().is_some_and(|q| same_path(q, dir));
    profiles
        .iter()
        .filter(|p| {
            p.layers.iter().any(|l| match l {
                LayerSpec::Mpq { path, .. }
                | LayerSpec::Folder { path }
                | LayerSpec::Casc { path } => inside(path),
                LayerSpec::MpqGlob { dir: d, .. } => same_path(d, dir),
                LayerSpec::Saves => false,
            })
        })
        .map(|p| p.id.clone())
        .collect()
}

/// A profile id must be `[a-z0-9_-]+` (it names folders).
pub fn valid_id(id: &str) -> bool {
    crate::profile::valid_id(id)
}

/// The profile for an install of `kind` in `dir`: [`Profile::median_xl`] for
/// Median XL, else [`Profile::vanilla`], with the given id, name (kept when
/// blank) and version.
pub fn profile_for(
    kind: InstallKind,
    dir: &Path,
    id: &str,
    name: &str,
    version: GameVersion,
) -> Profile {
    let mut profile = match kind {
        InstallKind::MedianXl => Profile::median_xl(dir),
        InstallKind::Vanilla | InstallKind::Unknown => Profile::vanilla(dir),
    };
    profile.id = id.to_owned();
    if !name.trim().is_empty() {
        profile.name = name.trim().to_owned();
    }
    profile.game_version = version;
    profile
}

/// Writes `profiles_dir\<id>.toml`, creating the folder. Refuses an invalid
/// id, an existing file, a profile that does not load back unchanged, and a
/// profiles folder inside the install folder itself
/// ([`AssetError::Profile`]).
pub fn write_profile(
    profiles_dir: &Path,
    install_dir: &Path,
    profile: &Profile,
) -> Result<PathBuf> {
    let refuse = |detail: String| AssetError::Profile {
        profile: profile.id.clone(),
        detail,
    };
    if !valid_id(&profile.id) {
        return Err(refuse(format!(
            "profile id {:?} may only hold a-z, 0-9, _ and -",
            profile.id
        )));
    }
    let file = profiles_dir.join(format!("{}.toml", profile.id));
    if is_inside(profiles_dir, install_dir) {
        return Err(refuse(format!(
            "{} is inside the game install; the editor never writes there",
            profiles_dir.display()
        )));
    }
    if file.exists() {
        return Err(refuse(format!("{} already exists", file.display())));
    }
    // The text must load back as the same profile.
    let text = profile.to_toml();
    match Profile::from_toml(&text) {
        Ok(back) if back == *profile => {}
        Ok(_) => return Err(refuse("does not round-trip through TOML".to_owned())),
        Err(e) => return Err(refuse(format!("does not round-trip through TOML: {e}"))),
    }
    std::fs::create_dir_all(profiles_dir).map_err(|e| AssetError::io(profiles_dir, e))?;
    write_atomic(&file, text.as_bytes())?;
    Ok(file)
}

/// Whether `dir` (which need not exist yet) lies inside `install` (which
/// must exist): the nearest existing ancestor of `dir` is compared after
/// resolving both.
fn is_inside(dir: &Path, install: &Path) -> bool {
    let Ok(install) = std::fs::canonicalize(install) else {
        return false;
    };
    let mut probe = Some(dir);
    while let Some(p) = probe {
        if let Ok(real) = std::fs::canonicalize(p) {
            return real.starts_with(&install);
        }
        probe = p.parent().filter(|q| !q.as_os_str().is_empty());
    }
    false
}

/// Writes `<path>.tmp`, then renames it over `path`.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| AssetError::io(&tmp, e))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AssetError::io(path, e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").expect("write");
    }

    fn mkdir(dir: &Path) {
        std::fs::create_dir_all(dir).expect("mkdir");
    }

    /// Every file under `dir` with its bytes, to prove nothing changed.
    fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).expect("read_dir").flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((p.clone(), std::fs::read(&p).expect("read")));
                }
            }
        }
        out.sort();
        out
    }

    /// A search that only sees folders under `root`.
    fn search(root: &Path, registry: &[&Path]) -> InstallSearch {
        InstallSearch {
            default_vanilla: root.join("default-d2"),
            default_mxl: root.join("default-mxl"),
            registry: registry
                .iter()
                .map(|p| Candidate {
                    path: p.to_path_buf(),
                    source: "registry (HKLM)".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn kinds_are_detected_from_file_names() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        assert_eq!(detect_kind(d), InstallKind::Unknown, "empty");
        assert_eq!(detect_kind(&d.join("missing")), InstallKind::Unknown);
        touch(d, "D2DATA.MPQ");
        assert_eq!(detect_kind(d), InstallKind::Unknown, "d2exp is missing");
        touch(d, "d2exp.mpq");
        assert_eq!(detect_kind(d), InstallKind::Vanilla);
        touch(d, "MedianXL-dGlsZXM.mpq");
        assert_eq!(detect_kind(d), InstallKind::MedianXl, "Median XL wins");
        // A folder named like an archive is not one.
        let other = tmp.path().join("other");
        mkdir(&other.join("medianxl-x.mpq"));
        assert_eq!(detect_kind(&other), InstallKind::Unknown);
    }

    #[test]
    fn registry_values_are_cleaned() {
        assert_eq!(
            install_path_from_value("C:\\Program Files (x86)\\Diablo II\\\0"),
            Some(PathBuf::from(r"C:\Program Files (x86)\Diablo II"))
        );
        assert_eq!(
            install_path_from_value("\"D:\\Games\\D2\" "),
            Some(PathBuf::from(r"D:\Games\D2"))
        );
        assert_eq!(install_path_from_value(""), None);
        assert_eq!(install_path_from_value("\0"), None);
        assert_eq!(install_path_from_value("relative\\dir"), None);
        #[cfg(windows)]
        {
            assert_eq!(install_path_from_value("E:\\"), Some(PathBuf::from("E:\\")));
            assert_eq!(install_path_from_value("E:"), Some(PathBuf::from("E:\\")));
        }
    }

    #[test]
    fn first_run_keeps_the_defaults_and_adds_the_registry() {
        let reg = vec![
            Candidate {
                path: PathBuf::from(r"D:\D2"),
                source: "registry (HKLM)".into(),
            },
            Candidate {
                path: PathBuf::from(r"c:\program files (x86)\diablo ii\"),
                source: "registry (HKCU)".into(),
            },
        ];
        let (vanilla, mxl) = first_run_candidates(
            Path::new(DEFAULT_VANILLA_INSTALL),
            Path::new(DEFAULT_MXL_INSTALL),
            &reg,
        );
        assert_eq!(
            vanilla,
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Diablo II"),
                PathBuf::from(r"D:\D2")
            ],
            "the duplicate of the default is dropped"
        );
        assert_eq!(mxl, vec![PathBuf::from(r"C:\games\median-xl")]);
        let s = InstallSearch {
            default_vanilla: DEFAULT_VANILLA_INSTALL.into(),
            default_mxl: DEFAULT_MXL_INSTALL.into(),
            registry: reg,
        };
        assert_eq!(s.candidates(), (vanilla, mxl));
    }

    #[test]
    fn seeding_uses_a_registry_install_when_the_default_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profiles = tmp.path().join("profiles");
        let from_registry = tmp.path().join("reg-d2");
        mkdir(&from_registry);
        // A registry path without vanilla's archives is not trusted.
        let written = seed_default_profiles(&profiles, || search(tmp.path(), &[&from_registry]))
            .expect("seed");
        assert!(written.is_empty());
        assert!(!profiles.exists(), "nothing to write: no folder either");
        touch(&from_registry, "d2data.mpq");
        touch(&from_registry, "d2exp.mpq");
        let before = tree(&from_registry);
        let written = seed_default_profiles(&profiles, || search(tmp.path(), &[&from_registry]))
            .expect("seed");
        assert_eq!(written, vec![profiles.join("vanilla.toml")]);
        let loaded = Profile::load_dir(&profiles).expect("load");
        assert_eq!(loaded, vec![Profile::vanilla(&from_registry)]);
        assert_eq!(tree(&from_registry), before, "the install is untouched");
        assert!(!profiles.join("vanilla.toml.tmp").exists());
    }

    #[test]
    fn default_locations_are_used_whenever_they_are_folders() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = search(tmp.path(), &[]);
        // The default vanilla folder is trusted without archives (as the
        // GUI always did); the Median XL default likewise.
        mkdir(&s.default_vanilla);
        mkdir(&s.default_mxl);
        let profiles = tmp.path().join("capybara").join("profiles");
        let written = seed_default_profiles(&profiles, || s.clone()).expect("seed");
        assert_eq!(
            written,
            vec![profiles.join("vanilla.toml"), profiles.join("mxl.toml")]
        );
        let loaded = Profile::load_dir(&profiles).expect("load");
        assert_eq!(
            loaded,
            vec![
                Profile::median_xl(&s.default_mxl),
                Profile::vanilla(&s.default_vanilla)
            ]
        );
        // Byte-identical to what the GUI writes.
        assert_eq!(
            std::fs::read_to_string(profiles.join("vanilla.toml")).expect("read"),
            Profile::vanilla(&s.default_vanilla).to_toml()
        );
    }

    #[test]
    fn existing_profiles_skip_the_search_and_are_never_overwritten() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profiles = tmp.path().join("profiles");
        mkdir(&profiles);
        std::fs::write(profiles.join("Mine.TOML"), "user file").expect("write");
        assert!(has_profile_files(&profiles));
        let called = std::cell::Cell::new(false);
        let written = seed_default_profiles(&profiles, || {
            called.set(true);
            search(tmp.path(), &[])
        })
        .expect("seed");
        assert!(written.is_empty());
        assert!(!called.get(), "the search must not run when profiles exist");

        // seed_profiles itself skips files that exist.
        let d2 = tmp.path().join("d2");
        mkdir(&d2);
        std::fs::write(profiles.join("vanilla.toml"), "keep me").expect("write");
        let written = seed_profiles(&profiles, &[d2], &[]).expect("seed");
        assert!(written.is_empty());
        assert_eq!(
            std::fs::read_to_string(profiles.join("vanilla.toml")).expect("read"),
            "keep me"
        );
    }

    #[test]
    fn has_profile_files_ignores_other_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        assert!(!has_profile_files(&d.join("missing")));
        assert!(!has_profile_files(d));
        touch(d, "notes.txt");
        mkdir(&d.join("folder.toml"));
        assert!(!has_profile_files(d), "a folder named *.toml is no profile");
        touch(d, "x.toml");
        assert!(has_profile_files(d));
    }

    #[test]
    fn seeding_never_writes_inside_an_install() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install = tmp.path().join("d2");
        mkdir(&install);
        touch(&install, "d2data.mpq");
        touch(&install, "d2exp.mpq");
        let before = tree(&install);
        // A capybara folder (not yet created) inside the install.
        let profiles = install.join("capybara").join("profiles");
        let written = seed_profiles(&profiles, std::slice::from_ref(&install), &[]).expect("seed");
        assert!(written.is_empty());
        assert_eq!(tree(&install), before);
        assert!(!install.join("capybara").exists());
    }

    #[test]
    fn first_run_prefers_vanilla_over_alphabetical_order() {
        let tmp = Path::new(r"C:\x");
        let mxl = Profile::median_xl(tmp);
        let vanilla = Profile::vanilla(tmp);
        let mut aaa = Profile::vanilla(tmp);
        aaa.id = "aaa".into();
        let all = vec![aaa.clone(), mxl.clone(), vanilla.clone()];
        assert_eq!(startup_profile(&all, None).as_deref(), Some("vanilla"));
        assert_eq!(
            startup_profile(&all, Some("mxl")).as_deref(),
            Some("mxl"),
            "the last session's profile wins"
        );
        assert_eq!(
            startup_profile(&all, Some("gone")).as_deref(),
            Some("vanilla"),
            "an unknown wish falls back"
        );
        assert_eq!(
            startup_profile(&[mxl.clone(), aaa.clone()], None).as_deref(),
            Some("aaa"),
            "without vanilla: alphabetical"
        );
        assert_eq!(startup_profile(&[], None), None);
    }

    #[test]
    fn ids_are_unique_and_validated() {
        let taken = vec!["vanilla".to_owned(), "vanilla-2".to_owned()];
        assert_eq!(unique_id(&taken, "vanilla"), "vanilla-3");
        assert_eq!(unique_id(&taken, "mxl"), "mxl");
        assert_eq!(unique_id(&["MXL".to_owned()], "mxl"), "mxl-2");
        assert!(valid_id("my_mod-2"));
        assert!(!valid_id("My Mod"));
        assert!(!valid_id(""));
    }

    #[test]
    fn profiles_using_a_folder_are_found() {
        let d2 = Path::new(r"C:\Program Files (x86)\Diablo II");
        let mxl = Path::new(r"C:\games\median-xl");
        let all = vec![Profile::vanilla(d2), Profile::median_xl(mxl)];
        assert_eq!(
            profiles_using(&all, Path::new(r"c:\program files (x86)\diablo ii\")),
            vec!["vanilla"]
        );
        assert_eq!(profiles_using(&all, mxl), vec!["mxl"]);
        assert!(profiles_using(&all, Path::new(r"D:\other")).is_empty());
    }

    #[test]
    fn adding_an_install_writes_only_the_profile() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let install = tmp.path().join("Median XL");
        mkdir(&install);
        for f in ["medianxl-dGlsZXM.mpq", "d2data.mpq", "d2exp.mpq"] {
            touch(&install, f);
        }
        let before = tree(&install);
        let profiles = tmp.path().join("capybara").join("profiles");
        let kind = detect_kind(&install);
        assert_eq!(kind, InstallKind::MedianXl);
        let id = unique_id(&["mxl".to_owned()], kind.base_id());
        let profile = profile_for(kind, &install, &id, "My Median", GameVersion::V113c);
        let file = write_profile(&profiles, &install, &profile).expect("write");
        assert_eq!(file, profiles.join("mxl-2.toml"));
        let back = Profile::load_dir(&profiles).expect("load");
        assert_eq!(back, vec![profile.clone()]);
        assert_eq!(back[0].name, "My Median");
        assert_eq!(tree(&install), before, "nothing written in the install");
        // Never overwrite, never write inside the install (existing or not).
        assert!(write_profile(&profiles, &install, &profile).is_err());
        let mut other = profile.clone();
        other.id = "mxl-3".into();
        assert!(write_profile(&install.join("profiles"), &install, &other).is_err());
        assert!(!install.join("profiles").exists());
        mkdir(&install.join("profiles"));
        assert!(write_profile(&install.join("profiles"), &install, &other).is_err());
        let mut bad = profile;
        bad.id = "Bad Id".into();
        assert!(matches!(
            write_profile(&profiles, &install, &bad),
            Err(AssetError::Profile { .. })
        ));
    }

    #[test]
    fn profile_for_keeps_the_default_name_when_blank() {
        let p = profile_for(
            InstallKind::Unknown,
            Path::new(r"D:\mod"),
            "mod",
            "  ",
            GameVersion::V113c,
        );
        assert_eq!(p.id, "mod");
        assert_eq!(p.name, Profile::vanilla(Path::new(r"D:\mod")).name);
        assert_eq!(p.game_version, GameVersion::V113c);
    }

    #[test]
    fn the_registry_is_read_without_failing() {
        // Machine-dependent only in what it finds: every entry must be a
        // clean absolute path. Reading never writes.
        for c in registry_install_paths() {
            assert!(c.path.is_absolute(), "{c:?}");
            assert!(c.source.starts_with("registry"), "{c:?}");
        }
    }
}
