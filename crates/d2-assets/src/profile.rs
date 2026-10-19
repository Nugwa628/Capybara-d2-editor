//! Game profiles: which layers make up a game setup, highest priority first.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AssetError, Result};
use crate::overlay::Overlay;
use crate::source::{AssetSource, CascSource, FolderSource, MpqSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameVersion {
    #[serde(rename = "1.13c")]
    V113c,
    #[serde(rename = "1.14d")]
    V114d,
    #[serde(rename = "d2r")]
    D2r,
}

impl GameVersion {
    /// The `.bin` layout family for this version; `None` for D2R, which has
    /// no `.bin` tables.
    pub fn excel(self) -> Option<d2_formats::excel::ExcelVersion> {
        match self {
            GameVersion::V113c => Some(d2_formats::excel::ExcelVersion::V113c),
            GameVersion::V114d => Some(d2_formats::excel::ExcelVersion::V114d),
            GameVersion::D2r => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LayerSpec {
    /// `<saves_root>\<profile id>\`. A missing folder is an empty layer.
    Saves,
    Folder {
        path: PathBuf,
    },
    Mpq {
        path: PathBuf,
        /// Skip the layer when the archive does not exist.
        #[serde(default)]
        optional: bool,
    },
    /// Every file in `dir` matching `pattern` (`*` wildcard, case-insensitive),
    /// sorted by name. Zero matches is an error.
    MpqGlob {
        dir: PathBuf,
        pattern: String,
    },
    Casc {
        path: PathBuf,
    },
}

/// One `capybara\profiles\<id>.toml`. Layers are listed highest priority first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// `[a-z0-9_-]+`; also the saves/history folder name.
    pub id: String,
    pub name: String,
    pub game_version: GameVersion,
    #[serde(rename = "layer")]
    pub layers: Vec<LayerSpec>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn profile_err(profile: &str, detail: impl Into<String>) -> AssetError {
    AssetError::Profile {
        profile: profile.to_owned(),
        detail: detail.into(),
    }
}

/// Case-insensitive `*` / `?` wildcard match.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
            }
            Some(&c) if c == '?' || Some(&c) == t.get(ti) => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    pi = sp + 1;
                    ti = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    p.get(pi..)
        .is_some_and(|rest| rest.iter().all(|&c| c == '*'))
}

impl Profile {
    pub fn from_toml(text: &str) -> Result<Profile> {
        let profile: Profile = toml::from_str(text).map_err(|e| profile_err("?", e.message()))?;
        if !valid_id(&profile.id) {
            return Err(profile_err(
                &profile.id,
                "id must be non-empty and contain only [a-z0-9_-]",
            ));
        }
        Ok(profile)
    }

    pub fn to_toml(&self) -> String {
        // Serialising plain strings, enums and paths cannot fail except for
        // non-UTF-8 paths; fall back to an explanatory comment rather than panic.
        toml::to_string(self).unwrap_or_else(|e| format!("# cannot serialise profile: {e}\n"))
    }

    /// saves → Patch_D2 → d2exp → d2data → d2char.
    pub fn vanilla(install_dir: &Path) -> Profile {
        let mpq = |file: &str| LayerSpec::Mpq {
            path: install_dir.join(file),
            optional: false,
        };
        Profile {
            id: "vanilla".into(),
            name: "Diablo II 1.14d".into(),
            game_version: GameVersion::V114d,
            layers: vec![
                LayerSpec::Saves,
                mpq("Patch_D2.mpq"),
                mpq("d2exp.mpq"),
                mpq("d2data.mpq"),
                mpq("d2char.mpq"),
            ],
        }
    }

    /// saves → medianxl-*.mpq → patch_d2 → d2exp → d2data → d2char, all from
    /// `install_dir` (Median XL's own base archives, never vanilla's).
    pub fn median_xl(install_dir: &Path) -> Profile {
        let mpq = |file: &str| LayerSpec::Mpq {
            path: install_dir.join(file),
            optional: false,
        };
        Profile {
            id: "mxl".into(),
            name: "Median XL".into(),
            game_version: GameVersion::V113c,
            layers: vec![
                LayerSpec::Saves,
                LayerSpec::MpqGlob {
                    dir: install_dir.to_path_buf(),
                    pattern: "medianxl-*.mpq".into(),
                },
                mpq("patch_d2.mpq"),
                mpq("d2exp.mpq"),
                mpq("d2data.mpq"),
                mpq("d2char.mpq"),
            ],
        }
    }

    /// `<saves_root>\<id>`.
    pub fn saves_dir(&self, saves_root: &Path) -> PathBuf {
        saves_root.join(&self.id)
    }

    /// Open every layer. Nothing is written anywhere.
    pub fn open(&self, saves_root: &Path) -> Result<Overlay> {
        if !valid_id(&self.id) {
            return Err(profile_err(&self.id, "invalid profile id"));
        }
        let mut layers: Vec<Box<dyn AssetSource>> = Vec::new();
        for spec in &self.layers {
            match spec {
                LayerSpec::Saves => layers.push(Box::new(FolderSource::new(
                    &self.saves_dir(saves_root),
                    format!("saves\\{}", self.id),
                ))),
                LayerSpec::Folder { path } => layers.push(Box::new(FolderSource::new(
                    path,
                    path.display().to_string(),
                ))),
                LayerSpec::Mpq { path, optional } => {
                    if *optional && !path.exists() {
                        continue;
                    }
                    layers.push(Box::new(MpqSource::open(path)?));
                }
                LayerSpec::MpqGlob { dir, pattern } => {
                    for path in self.glob(dir, pattern)? {
                        layers.push(Box::new(MpqSource::open(&path)?));
                    }
                }
                LayerSpec::Casc { path } => layers.push(Box::new(CascSource::open(path)?)),
            }
        }
        Ok(Overlay::new(layers))
    }

    fn glob(&self, dir: &Path, pattern: &str) -> Result<Vec<PathBuf>> {
        let entries = std::fs::read_dir(dir).map_err(|e| AssetError::io(dir, e))?;
        let mut found: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| AssetError::io(dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if glob_matches(pattern, &name) && entry.path().is_file() {
                found.push((name.to_lowercase(), entry.path()));
            }
        }
        if found.is_empty() {
            return Err(profile_err(
                &self.id,
                format!("no file in {} matches {pattern}", dir.display()),
            ));
        }
        found.sort();
        Ok(found.into_iter().map(|(_, p)| p).collect())
    }

    /// Every `*.toml` in `dir`, sorted by id. A bad file is an error naming
    /// it; a missing directory yields no profiles.
    pub fn load_dir(dir: &Path) -> Result<Vec<Profile>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(AssetError::io(dir, e)),
        };
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| AssetError::io(dir, e))?.path();
            let is_toml = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
            if is_toml && path.is_file() {
                files.push(path);
            }
        }
        files.sort();
        let mut profiles: Vec<(Profile, PathBuf)> = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).map_err(|e| AssetError::io(&file, e))?;
            let profile = Profile::from_toml(&text).map_err(|e| {
                profile_err(
                    &file.display().to_string(),
                    match e {
                        AssetError::Profile { detail, .. } => detail,
                        other => other.to_string(),
                    },
                )
            })?;
            if let Some((_, other)) = profiles.iter().find(|(p, _)| p.id == profile.id) {
                return Err(profile_err(
                    &file.display().to_string(),
                    format!(
                        "duplicate profile id {:?} (also in {})",
                        profile.id,
                        other.display()
                    ),
                ));
            }
            profiles.push((profile, file));
        }
        profiles.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        Ok(profiles.into_iter().map(|(p, _)| p).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_version_mapping() {
        use d2_formats::excel::ExcelVersion;
        assert_eq!(GameVersion::V113c.excel(), Some(ExcelVersion::V113c));
        assert_eq!(GameVersion::V114d.excel(), Some(ExcelVersion::V114d));
        assert_eq!(GameVersion::D2r.excel(), None);
    }

    #[test]
    fn globbing() {
        assert!(glob_matches("medianxl-*.mpq", "medianxl-Z3Vp-10.mpq"));
        assert!(glob_matches("MEDIANXL-*.MPQ", "medianxl-abc.mpq"));
        assert!(!glob_matches("medianxl-*.mpq", "d2data.mpq"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("a*b*c", "aXXbYYc"));
        assert!(!glob_matches("a*b*c", "aXXbYY"));
        assert!(glob_matches("d2?ata.mpq", "d2data.mpq"));
    }

    #[test]
    fn toml_round_trip() {
        for profile in [
            Profile::vanilla(Path::new("C:\\Games\\D2")),
            Profile::median_xl(Path::new("C:\\games\\median-xl")),
        ] {
            let text = profile.to_toml();
            assert_eq!(Profile::from_toml(&text).expect("parse"), profile, "{text}");
        }
        let text = Profile {
            id: "x".into(),
            name: "X".into(),
            game_version: GameVersion::D2r,
            layers: vec![
                LayerSpec::Folder {
                    path: "mods/x".into(),
                },
                LayerSpec::Mpq {
                    path: "a.mpq".into(),
                    optional: true,
                },
                LayerSpec::Casc { path: "d2r".into() },
            ],
        }
        .to_toml();
        assert!(text.contains("kind = \"casc\""), "{text}");
        assert!(text.contains("d2r"), "{text}");
    }

    #[test]
    fn hand_written_toml() {
        let p = Profile::from_toml(
            r#"
id = "mod-1"
name = "My mod"
game_version = "1.13c"

[[layer]]
kind = "saves"

[[layer]]
kind = "mpq"
path = "a.mpq"

[[layer]]
kind = "mpq-glob"
dir = "d"
pattern = "x-*.mpq"
"#,
        )
        .expect("parse");
        assert_eq!(p.game_version, GameVersion::V113c);
        assert_eq!(
            p.layers[1],
            LayerSpec::Mpq {
                path: "a.mpq".into(),
                optional: false
            }
        );
        assert_eq!(p.layers.len(), 3);
    }

    #[test]
    fn bad_toml_is_an_error() {
        assert!(Profile::from_toml("id = ").is_err());
        let bad_id = "id = \"Bad Id\"\nname = \"x\"\ngame_version = \"1.14d\"\nlayer = []\n";
        assert!(Profile::from_toml(bad_id).is_err());
        let bad_ver = "id = \"a\"\nname = \"x\"\ngame_version = \"2.0\"\nlayer = []\n";
        assert!(Profile::from_toml(bad_ver).is_err());
    }

    #[test]
    fn saves_dir_and_ids() {
        let p = Profile::vanilla(Path::new("x"));
        assert_eq!(p.saves_dir(Path::new("s")), Path::new("s").join("vanilla"));
        assert!(valid_id("a-b_c9"));
        assert!(!valid_id(""));
        assert!(!valid_id("A"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(".."));
    }
}
