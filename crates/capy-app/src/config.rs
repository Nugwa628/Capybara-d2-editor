//! `config.toml`: settings, plus first-run profile seeding.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use d2_assets::Profile;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

pub const DEFAULT_GAMMA: f32 = 1.30;
pub const MAX_RECENT_MAPS: usize = 20;
/// Default install locations probed on first run.
pub const VANILLA_INSTALL: &str = r"C:\Program Files (x86)\Diablo II";
pub const MXL_INSTALL: &str = r"C:\games\median-xl";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: f32,
    pub height: f32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            width: 1600.0,
            height: 900.0,
        }
    }
}

/// Which layer kinds start visible when a map opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerVisibility {
    pub floor: bool,
    pub wall: bool,
    pub shadow: bool,
    pub tag: bool,
    pub objects: bool,
}

impl Default for LayerVisibility {
    fn default() -> Self {
        LayerVisibility {
            floor: true,
            wall: true,
            shadow: true,
            tag: false,
            objects: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentMap {
    pub profile: String,
    /// Canonical game path.
    pub path: String,
}

/// Field order matters for TOML: plain values first, then tables, then arrays
/// of tables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_profile: Option<String>,
    pub gamma: f32,
    pub autosave_minutes: u32,
    pub window: WindowConfig,
    pub layer_visibility: LayerVisibility,
    pub recent_maps: Vec<RecentMap>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            last_profile: None,
            gamma: DEFAULT_GAMMA,
            autosave_minutes: 5,
            window: WindowConfig::default(),
            layer_visibility: LayerVisibility::default(),
            recent_maps: Vec::new(),
        }
    }
}

impl Config {
    /// A missing file is the default config; a malformed one is an error.
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut cfg: Config =
                    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
                cfg.sanitize();
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Write via a temp file and rename so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("serialising config")?;
        write_atomic(path, text.as_bytes())
    }

    /// Clamp values a hand edit could have made unusable.
    pub fn sanitize(&mut self) {
        if !self.gamma.is_finite() || self.gamma <= 0.0 {
            self.gamma = DEFAULT_GAMMA;
        }
        if !self.window.width.is_finite() || self.window.width < 320.0 {
            self.window.width = WindowConfig::default().width;
        }
        if !self.window.height.is_finite() || self.window.height < 240.0 {
            self.window.height = WindowConfig::default().height;
        }
        self.recent_maps.truncate(MAX_RECENT_MAPS);
    }

    /// Move (profile, path) to the front of the recent list.
    pub fn push_recent(&mut self, profile: &str, path: &str) {
        self.recent_maps
            .retain(|r| !(r.profile == profile && r.path == path));
        self.recent_maps.insert(
            0,
            RecentMap {
                profile: profile.to_owned(),
                path: path.to_owned(),
            },
        );
        self.recent_maps.truncate(MAX_RECENT_MAPS);
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// First run: write `vanilla.toml` / `mxl.toml` for installs that exist at the
/// default paths. Existing files are never overwritten. Returns the files
/// written.
pub fn seed_profiles(paths: &Paths, vanilla: &Path, mxl: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let candidates = [
        (vanilla, "vanilla", Profile::vanilla(vanilla)),
        (mxl, "mxl", Profile::median_xl(mxl)),
    ];
    for (install, id, profile) in candidates {
        if !install.is_dir() {
            continue;
        }
        let file = paths.profiles.join(format!("{id}.toml"));
        if file.exists() {
            continue;
        }
        write_atomic(&file, profile.to_toml().as_bytes())?;
        written.push(file);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert!((c.gamma - 1.30).abs() < f32::EPSILON);
        assert_eq!(c.layer_visibility, LayerVisibility::default());
    }

    #[test]
    fn round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("config.toml");
        let mut c = Config {
            last_profile: Some("mxl".into()),
            gamma: 1.5,
            autosave_minutes: 9,
            ..Config::default()
        };
        c.window = WindowConfig {
            width: 1280.0,
            height: 720.0,
        };
        c.layer_visibility.tag = true;
        c.push_recent("mxl", r"data\global\tiles\a.ds1");
        c.push_recent("vanilla", r"data\global\tiles\b.ds1");
        c.save(&file).expect("save");
        let back = Config::load(&file).expect("load");
        assert_eq!(back, c);
    }

    #[test]
    fn missing_file_is_default_and_bad_file_is_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = Config::load(&tmp.path().join("nope.toml")).expect("default");
        assert_eq!(missing, Config::default());
        let bad = tmp.path().join("bad.toml");
        std::fs::write(&bad, "gamma = [").expect("write");
        assert!(Config::load(&bad).is_err());
    }

    #[test]
    fn partial_file_fills_defaults_and_sanitizes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("c.toml");
        std::fs::write(&file, "gamma = -4.0\n[window]\nwidth = 5.0\n").expect("write");
        let c = Config::load(&file).expect("load");
        assert!((c.gamma - DEFAULT_GAMMA).abs() < f32::EPSILON);
        assert!(c.window.width >= 320.0);
        assert_eq!(c.autosave_minutes, 5);
    }

    #[test]
    fn recent_list_dedups_and_caps() {
        let mut c = Config::default();
        for i in 0..30 {
            c.push_recent("v", &format!("p{i}"));
        }
        assert_eq!(c.recent_maps.len(), MAX_RECENT_MAPS);
        c.push_recent("v", "p25");
        assert_eq!(c.recent_maps[0].path, "p25");
        assert_eq!(c.recent_maps.iter().filter(|r| r.path == "p25").count(), 1);
    }

    #[test]
    fn seed_profiles_only_for_existing_installs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::at(tmp.path().join("capybara"));
        paths.ensure().expect("ensure");
        let vanilla = tmp.path().join("d2");
        std::fs::create_dir(&vanilla).expect("mkdir");
        let missing = tmp.path().join("no-mxl");
        let written = seed_profiles(&paths, &vanilla, &missing).expect("seed");
        assert_eq!(written.len(), 1);
        assert!(paths.profiles.join("vanilla.toml").is_file());
        assert!(!paths.profiles.join("mxl.toml").exists());
        // Loadable and not overwritten on a second run.
        let profiles = Profile::load_dir(&paths.profiles).expect("load_dir");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "vanilla");
        assert!(
            seed_profiles(&paths, &vanilla, &missing)
                .expect("again")
                .is_empty()
        );
    }
}
