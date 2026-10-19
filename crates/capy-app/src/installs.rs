//! Game installs: what kind of install a folder is, where the Windows
//! registry says Diablo II lives (read-only), first-run profile seeding and
//! the start-up profile choice, and writing a new profile for "Add game
//! install...". Install folders are only ever read; profiles are written to
//! `capybara\profiles\`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use d2_assets::{GameVersion, Profile};

use crate::config::write_atomic;

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
pub const REGISTRY_VALUE: &str = "InstallPath";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryRoot {
    LocalMachine,
    CurrentUser,
}

impl RegistryRoot {
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
/// XL wins because it also ships its own `d2data.mpq` and `d2exp.mpq`.
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

/// The `InstallPath` values of the Diablo II registry keys, read-only.
/// Missing keys and values are simply skipped.
#[cfg(windows)]
pub fn registry_install_paths() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for (root, key) in REGISTRY_KEYS {
        let hive = match root {
            RegistryRoot::LocalMachine => windows_registry::LOCAL_MACHINE,
            RegistryRoot::CurrentUser => windows_registry::CURRENT_USER,
        };
        // `open` asks for read access only.
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

#[cfg(not(windows))]
pub fn registry_install_paths() -> Vec<Candidate> {
    Vec::new()
}

/// Case-insensitive path equality, ignoring a trailing separator.
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

/// First run: write `vanilla.toml` / `mxl.toml` for the first candidate of
/// each that exists. The default location is used whenever it is a folder (as
/// before); a registry path only when it really holds vanilla's archives.
/// Existing files are never overwritten. Returns the files written.
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
        pick_vanilla.map(|p| ("vanilla", Profile::vanilla(&p))),
        pick_mxl.map(|p| ("mxl", Profile::median_xl(&p))),
    ];
    for (id, profile) in candidates.into_iter().flatten() {
        let file = profiles_dir.join(format!("{id}.toml"));
        if file.exists() {
            continue;
        }
        write_atomic(&file, profile.to_toml().as_bytes())?;
        written.push(file);
    }
    Ok(written)
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

/// `base`, else `base-2`, `base-3`, ... whichever is not taken.
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
    use d2_assets::LayerSpec;
    let inside = |p: &Path| same_path(p, dir) || p.parent().is_some_and(|q| same_path(q, dir));
    profiles
        .iter()
        .filter(|p| {
            p.layers.iter().any(|l| match l {
                LayerSpec::Mpq { path, .. }
                | LayerSpec::Folder { path }
                | LayerSpec::Casc { path } => inside(path),
                LayerSpec::MpqGlob { dir: d, .. } => same_path(d, dir),
                _ => false,
            })
        })
        .map(|p| p.id.clone())
        .collect()
}

/// A profile id must be `[a-z0-9_-]+` (it names folders).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// The profile for an install of `kind` in `dir`.
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

/// Writes `profiles_dir\<id>.toml`. Refuses an existing file and anything
/// that would land inside the install folder itself.
pub fn write_profile(
    profiles_dir: &Path,
    install_dir: &Path,
    profile: &Profile,
) -> Result<PathBuf> {
    if !valid_id(&profile.id) {
        bail!(
            "profile id {:?} may only hold a-z, 0-9, _ and -",
            profile.id
        );
    }
    let file = profiles_dir.join(format!("{}.toml", profile.id));
    let inside_install = std::fs::canonicalize(profiles_dir)
        .ok()
        .zip(std::fs::canonicalize(install_dir).ok())
        .is_some_and(|(p, i)| p.starts_with(i));
    if inside_install {
        bail!(
            "{} is inside the game install; the editor never writes there",
            profiles_dir.display()
        );
    }
    if file.exists() {
        bail!("{} already exists", file.display());
    }
    // The text must load back as the same profile.
    let text = profile.to_toml();
    Profile::from_toml(&text)
        .with_context(|| format!("profile {} does not round-trip", profile.id))?;
    std::fs::create_dir_all(profiles_dir)
        .with_context(|| format!("creating {}", profiles_dir.display()))?;
    write_atomic(&file, text.as_bytes())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").expect("write");
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
        std::fs::create_dir_all(other.join("medianxl-x.mpq")).expect("mkdir");
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
        assert_eq!(install_path_from_value("E:\\"), Some(PathBuf::from("E:\\")));
    }

    #[test]
    fn the_registry_is_read_without_failing() {
        // Machine-dependent: whatever is found must be a clean absolute path.
        for c in registry_install_paths() {
            assert!(c.path.is_absolute(), "{c:?}");
            assert!(c.source.starts_with("registry"), "{c:?}");
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
            Path::new(r"C:\Program Files (x86)\Diablo II"),
            Path::new(r"C:\games\median-xl"),
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
    }

    #[test]
    fn seeding_uses_a_registry_install_when_the_default_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profiles = tmp.path().join("profiles");
        std::fs::create_dir_all(&profiles).expect("mkdir");
        let from_registry = tmp.path().join("reg-d2");
        std::fs::create_dir_all(&from_registry).expect("mkdir");
        // A registry path without vanilla's archives is not trusted.
        let written = seed_profiles(
            &profiles,
            &[tmp.path().join("no-default"), from_registry.clone()],
            &[tmp.path().join("no-mxl")],
        )
        .expect("seed");
        assert!(written.is_empty());
        touch(&from_registry, "d2data.mpq");
        touch(&from_registry, "d2exp.mpq");
        let before = tree(&from_registry);
        let written = seed_profiles(
            &profiles,
            &[tmp.path().join("no-default"), from_registry.clone()],
            &[tmp.path().join("no-mxl")],
        )
        .expect("seed");
        assert_eq!(written, vec![profiles.join("vanilla.toml")]);
        let loaded = Profile::load_dir(&profiles).expect("load");
        assert_eq!(loaded[0], Profile::vanilla(&from_registry));
        assert_eq!(tree(&from_registry), before, "the install is untouched");
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
        std::fs::create_dir_all(&install).expect("mkdir");
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
        // Never overwrite, never write inside the install.
        assert!(write_profile(&profiles, &install, &profile).is_err());
        let inside = install.join("profiles");
        std::fs::create_dir_all(&inside).expect("mkdir");
        let mut other = profile.clone();
        other.id = "mxl-3".into();
        assert!(write_profile(&inside, &install, &other).is_err());
        let mut bad = profile;
        bad.id = "Bad Id".into();
        assert!(write_profile(&profiles, &install, &bad).is_err());
    }
}
