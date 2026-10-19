//! `config.toml`: settings, plus first-run profile seeding.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::edit::tools::SelectScope;

pub const DEFAULT_GAMMA: f32 = 1.30;
/// The gamma range the Settings dialog offers.
pub const GAMMA_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;
/// Default retention: newest Save/Autosave/BeforeRestore snapshots kept per map.
pub const DEFAULT_KEEP_LAST: usize = 100;
/// Longest autosave interval the Settings dialog offers (minutes).
pub const MAX_AUTOSAVE_MINUTES: u32 = 240;
pub const MAX_RECENT_MAPS: usize = 20;
/// Default install locations probed on first run.
pub const VANILLA_INSTALL: &str = d2_assets::installs::DEFAULT_VANILLA_INSTALL;
pub const MXL_INSTALL: &str = d2_assets::installs::DEFAULT_MXL_INSTALL;

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

/// Top-level editor mode (the tab strip above the dock).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppMode {
    #[default]
    Map,
    Sprites,
}

/// A user-picked LvlType/Def for one map, persisted per profile. This is what
/// `MapCatalog::set_override` is fed with when a profile opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapOverride {
    pub profile: String,
    /// Canonical game path of the DS1.
    pub ds1: String,
    pub lvl_type: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub def: Option<u32>,
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
    /// Minutes between autosaves of dirty maps; 0 turns autosave off.
    pub autosave_minutes: u32,
    /// History retention: newest Save/Autosave/BeforeRestore snapshots kept
    /// per map (checkpoints are never pruned).
    pub keep_last: usize,
    /// History retention: also drop those older than this many days; 0 = no
    /// age limit.
    pub max_age_days: u32,
    /// The layers a selection tool looks at when the editor starts.
    pub select_scope: SelectScope,
    /// Thumbnail edge of the Tiles browser in points (Ctrl+wheel).
    pub browser_thumb: f32,
    /// View > "Animate objects": play object and NPC animations on the map.
    pub animate_objects: bool,
    /// View > "Object markers": diamond handles on every object, not only
    /// with the Objects tool.
    pub object_markers: bool,
    /// Settings > "Show loading screen": the door animation at start-up.
    pub show_loading_screen: bool,
    pub mode: AppMode,
    pub window: WindowConfig,
    pub layer_visibility: LayerVisibility,
    pub recent_maps: Vec<RecentMap>,
    pub overrides: Vec<MapOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            last_profile: None,
            gamma: DEFAULT_GAMMA,
            autosave_minutes: 5,
            keep_last: DEFAULT_KEEP_LAST,
            max_age_days: 0,
            select_scope: SelectScope::default(),
            browser_thumb: 64.0,
            animate_objects: true,
            object_markers: false,
            show_loading_screen: true,
            mode: AppMode::default(),
            window: WindowConfig::default(),
            layer_visibility: LayerVisibility::default(),
            recent_maps: Vec::new(),
            overrides: Vec::new(),
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
        self.autosave_minutes = self.autosave_minutes.min(MAX_AUTOSAVE_MINUTES);
        if self.keep_last == 0 {
            self.keep_last = 1; // keep_last = 0 would drop every save snapshot
        }
    }

    /// The retention policy for `SnapshotStore::prune`.
    pub fn retention(&self) -> capy_history::Retention {
        capy_history::Retention {
            keep_last: self.keep_last.max(1),
            max_age_days: (self.max_age_days > 0).then_some(self.max_age_days),
        }
    }

    /// Remember an LvlType/Def override for `ds1` in `profile`, replacing any
    /// earlier one for the same map.
    pub fn set_override(&mut self, profile: &str, ds1: &str, lvl_type: u32, def: Option<u32>) {
        self.clear_override(profile, ds1);
        self.overrides.push(MapOverride {
            profile: profile.to_owned(),
            ds1: ds1.to_owned(),
            lvl_type,
            def,
        });
    }

    pub fn clear_override(&mut self, profile: &str, ds1: &str) {
        self.overrides
            .retain(|o| !(o.profile == profile && o.ds1 == ds1));
    }

    /// The overrides of one profile.
    pub fn overrides_for<'a>(&'a self, profile: &'a str) -> impl Iterator<Item = &'a MapOverride> {
        self.overrides.iter().filter(move |o| o.profile == profile)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use d2_assets::Profile;

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
    fn overrides_persist_per_profile_and_replace() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("config.toml");
        let mut c = Config::default();
        c.set_override("mxl", r"data\global\tiles\a.ds1", 5, Some(12));
        c.set_override("vanilla", r"data\global\tiles\a.ds1", 1, None);
        c.set_override("mxl", r"data\global\tiles\a.ds1", 7, None); // replaces
        c.mode = AppMode::Sprites;
        c.push_recent("mxl", "x"); // arrays of tables must coexist
        c.save(&file).expect("save");
        let back = Config::load(&file).expect("load");
        assert_eq!(back, c);
        let mxl: Vec<_> = back.overrides_for("mxl").collect();
        assert_eq!(mxl.len(), 1);
        assert_eq!((mxl[0].lvl_type, mxl[0].def), (7, None));
        assert_eq!(back.overrides_for("vanilla").count(), 1);
        assert_eq!(back.mode, AppMode::Sprites);
        let mut back = back;
        back.clear_override("mxl", r"data\global\tiles\a.ds1");
        assert_eq!(back.overrides_for("mxl").count(), 0);
        assert_eq!(back.overrides_for("vanilla").count(), 1);
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
        let seed = |v: &Path, m: &Path| {
            crate::installs::seed_profiles(&paths.profiles, &[v.to_path_buf()], &[m.to_path_buf()])
        };
        let written = seed(&vanilla, &missing).expect("seed");
        assert_eq!(written.len(), 1);
        assert!(paths.profiles.join("vanilla.toml").is_file());
        assert!(!paths.profiles.join("mxl.toml").exists());
        // Loadable and not overwritten on a second run.
        let profiles = Profile::load_dir(&paths.profiles).expect("load_dir");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "vanilla");
        assert!(seed(&vanilla, &missing).expect("again").is_empty());
    }
}
