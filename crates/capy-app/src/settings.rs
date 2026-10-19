//! State of the Settings window and the "Add game install..." dialog, and
//! what applying them does. Drawing lives in `crate::ui::settings`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use d2_assets::{GameVersion, Profile};

use crate::config::LayerVisibility;
use crate::doc::{LayerKind, layer_keys};
use crate::edit::tools::SelectScope;
use crate::installs::{self, Candidate, InstallKind};
use crate::state::AppState;

/// The "Add game install..." dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddInstall {
    /// The folder as typed or picked.
    pub dir: String,
    /// What `dir` was found to hold (re-checked when `dir` changes).
    pub detected: Option<(String, InstallKind)>,
    /// The kind the profile is written as; `Unknown` until the user says.
    pub kind: InstallKind,
    pub id: String,
    pub name: String,
    pub version: GameVersion,
    pub error: Option<String>,
}

impl AddInstall {
    fn new() -> AddInstall {
        AddInstall {
            dir: String::new(),
            detected: None,
            kind: InstallKind::Unknown,
            id: String::new(),
            name: String::new(),
            version: GameVersion::V114d,
            error: None,
        }
    }

    /// The detected kind of the current folder, if it was checked.
    pub fn detected_kind(&self) -> Option<InstallKind> {
        self.detected
            .as_ref()
            .filter(|(d, _)| d == &self.dir)
            .map(|(_, k)| *k)
    }

    /// Why "Add profile" cannot be pressed yet, if it cannot.
    pub fn blocker(&self) -> Option<&'static str> {
        if self.dir.trim().is_empty() {
            Some("Choose the install folder")
        } else if !Path::new(self.dir.trim()).is_dir() {
            Some("That folder does not exist")
        } else if self.kind == InstallKind::Unknown {
            Some("Say whether it is vanilla or Median XL")
        } else if !installs::valid_id(&self.id) {
            Some("The id may only hold a-z, 0-9, _ and -")
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    pub add: Option<AddInstall>,
    registry: Option<Vec<Candidate>>,
    picker: Option<Mutex<Receiver<Option<PathBuf>>>>,
}

impl SettingsUi {
    pub fn picking(&self) -> bool {
        self.picker.is_some()
    }

    pub fn modal_open(&self) -> bool {
        self.add.is_some()
    }
}

impl AppState {
    /// Diablo II install paths from the registry (read once).
    pub fn registry_candidates(&mut self) -> Vec<Candidate> {
        self.settings
            .registry
            .get_or_insert_with(installs::registry_install_paths)
            .clone()
    }

    /// Where "Add game install..." suggests looking: the registry, then the
    /// default locations, existing folders only, without duplicates.
    pub fn install_suggestions(&mut self) -> Vec<Candidate> {
        let mut out = self.registry_candidates();
        for (path, what) in [
            (crate::config::VANILLA_INSTALL, "default location"),
            (crate::config::MXL_INSTALL, "default Median XL location"),
        ] {
            let p = PathBuf::from(path);
            if p.is_dir() && !out.iter().any(|c| installs::same_path(&c.path, &p)) {
                out.push(Candidate {
                    path: p,
                    source: what.to_owned(),
                });
            }
        }
        out.retain(|c| c.path.is_dir());
        out
    }

    /// Open the "Add game install..." dialog, optionally for `dir`.
    pub fn open_add_install(&mut self, dir: Option<&str>) {
        self.settings.add = Some(AddInstall::new());
        if let Some(d) = dir {
            self.add_install_set_dir(d);
        }
    }

    /// Take `dir` into the dialog: detect the kind, propose id and name.
    pub fn add_install_set_dir(&mut self, dir: &str) {
        let taken: Vec<String> = self.profiles.iter().map(|p| p.id.clone()).collect();
        let Some(add) = self.settings.add.as_mut() else {
            return;
        };
        add.dir = dir.trim().to_owned();
        add.error = None;
        let path = PathBuf::from(&add.dir);
        let kind = installs::detect_kind(&path);
        add.detected = Some((add.dir.clone(), kind));
        add.kind = kind;
        propose(add, &taken, &path);
    }

    /// The user picked the kind of an install that was not recognised.
    pub fn add_install_set_kind(&mut self, kind: InstallKind) {
        let taken: Vec<String> = self.profiles.iter().map(|p| p.id.clone()).collect();
        if let Some(add) = self.settings.add.as_mut() {
            add.kind = kind;
            let path = PathBuf::from(&add.dir);
            propose(add, &taken, &path);
        }
    }

    /// "Choose folder...": a native folder picker on a worker thread.
    pub fn pick_install_folder(&mut self) {
        if self.settings.picker.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("install-picker".to_owned())
            .spawn(move || {
                let picked = rfd::FileDialog::new()
                    .set_title("Choose a Diablo II or Median XL install folder")
                    .pick_folder();
                let _ = tx.send(picked);
            });
        if spawned.is_ok() {
            self.settings.picker = Some(Mutex::new(rx));
        }
    }

    /// The folder picker's answer, once it closed.
    pub fn poll_settings(&mut self) {
        let answer = match &self.settings.picker {
            Some(rx) => match rx.lock().map(|r| r.try_recv()) {
                Ok(Ok(answer)) => answer,
                Ok(Err(std::sync::mpsc::TryRecvError::Empty)) => return,
                _ => None,
            },
            None => return,
        };
        self.settings.picker = None;
        if let Some(dir) = answer {
            self.add_install_set_dir(&dir.to_string_lossy());
        }
    }

    /// "Add profile": write `capybara\profiles\<id>.toml`, load it and open
    /// it. Returns the new id.
    pub fn confirm_add_install(&mut self) -> Result<String, String> {
        let add = self.settings.add.clone().ok_or("the dialog is closed")?;
        if let Some(why) = add.blocker() {
            return Err(why.to_owned());
        }
        if self.profiles.iter().any(|p| p.id == add.id) {
            return Err(format!("a profile with id {:?} exists already", add.id));
        }
        let dir = PathBuf::from(add.dir.trim());
        let profile = installs::profile_for(add.kind, &dir, &add.id, &add.name, add.version);
        let file = installs::write_profile(&self.paths.profiles, &dir, &profile)
            .map_err(|e| format!("{e:#}"))?;
        self.info(format!(
            "Added profile {} ({}) for {} in {}",
            profile.id,
            add.kind.label(),
            dir.display(),
            file.display()
        ));
        match Profile::load_dir(&self.paths.profiles) {
            Ok(all) => self.profiles = all,
            Err(e) => {
                self.warn(format!("Profiles: {e}"));
                self.profiles.retain(|p| p.id != profile.id);
                self.profiles.push(profile.clone());
                self.profiles.sort_by(|a, b| a.id.cmp(&b.id));
            }
        }
        self.settings.add = None;
        self.open_profile_by_id(&profile.id);
        Ok(profile.id)
    }

    /// Settings: the default layer visibility changed. Applies to maps
    /// opened from now on (and is saved).
    pub fn set_layer_defaults(&mut self, v: LayerVisibility) {
        self.config.layer_visibility = v;
        self.config_dirty = true;
    }

    /// Settings: the default selection scope; also becomes the current one.
    pub fn set_scope_default(&mut self, scope: SelectScope) {
        self.config.select_scope = scope;
        self.edit.scope = scope;
        self.config_dirty = true;
    }

    /// The `setting <key> <value>` script command and the dialog's backend.
    pub fn apply_setting(&mut self, key: &str, value: &str) -> Result<(), String> {
        let num = |what: &str| -> Result<f64, String> {
            value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("{what}: {value:?} is not a number"))
        };
        let flag = || -> Result<bool, String> {
            match value.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" | "1" => Ok(true),
                "off" | "false" | "no" | "0" => Ok(false),
                _ => Err(format!("{value:?} must be on or off")),
            }
        };
        match key.to_ascii_lowercase().as_str() {
            "gamma" => {
                let g = num("gamma")? as f32;
                if !crate::config::GAMMA_RANGE.contains(&g) {
                    return Err(format!("gamma {g} is outside 0.5..3.0"));
                }
                self.config.gamma = g;
            }
            "autosave" | "autosave_minutes" | "autosave-minutes" => {
                let m = num("autosave minutes")?;
                if !(0.0..=f64::from(crate::config::MAX_AUTOSAVE_MINUTES)).contains(&m) {
                    return Err(format!(
                        "autosave minutes must be 0..{}",
                        crate::config::MAX_AUTOSAVE_MINUTES
                    ));
                }
                self.config.autosave_minutes = m as u32;
                self.history.timer.reset();
            }
            "keep_last" | "keep-last" => {
                let n = num("keep_last")?;
                if n < 1.0 {
                    return Err("keep_last must be at least 1".to_owned());
                }
                self.config.keep_last = n as usize;
            }
            "max_age_days" | "max-age-days" => {
                let n = num("max_age_days")?;
                if n < 0.0 {
                    return Err("max_age_days must be 0 (no limit) or more".to_owned());
                }
                self.config.max_age_days = n as u32;
            }
            "select_scope" | "select-scope" | "scope" => {
                let scope = match value.to_ascii_lowercase().as_str() {
                    "active" => SelectScope::Active,
                    "visible" => SelectScope::Visible,
                    other => return Err(format!("scope {other:?} must be active or visible")),
                };
                self.set_scope_default(scope);
            }
            layer if layer.starts_with("layer.") || layer.starts_with("layer_") => {
                let on = flag()?;
                let mut v = self.config.layer_visibility;
                match &layer[6..] {
                    "floor" => v.floor = on,
                    "wall" => v.wall = on,
                    "shadow" => v.shadow = on,
                    "tag" => v.tag = on,
                    "objects" => v.objects = on,
                    other => {
                        return Err(format!(
                            "layer kind {other:?} must be floor, wall, shadow, tag or objects"
                        ));
                    }
                }
                self.set_layer_defaults(v);
            }
            other => {
                return Err(format!(
                    "unknown setting {other:?} (gamma, autosave_minutes, keep_last, \
                     max_age_days, select_scope, layer.<kind>)"
                ));
            }
        }
        self.config_dirty = true;
        Ok(())
    }

    /// Apply the default layer visibility to the active map now ("Apply to
    /// this map" in Settings).
    pub fn apply_layer_defaults_to_active(&mut self) {
        let v = self.config.layer_visibility;
        let Some(doc) = self.active_doc_mut() else {
            return;
        };
        for key in layer_keys(&doc.ds1) {
            let on = match key.kind() {
                LayerKind::Floor => v.floor,
                LayerKind::Wall => v.wall,
                LayerKind::Shadow => v.shadow,
                LayerKind::Tag => v.tag,
            };
            doc.set_visible(key, on);
        }
        doc.objects_visible = v.objects;
    }
}

/// Proposes an id, name and game version for the kind and folder in `add`.
fn propose(add: &mut AddInstall, taken: &[String], dir: &Path) {
    if add.kind == InstallKind::Unknown {
        return;
    }
    let base_id = add.kind.base_id();
    let template = match add.kind {
        InstallKind::MedianXl => Profile::median_xl(dir),
        _ => Profile::vanilla(dir),
    };
    add.version = template.game_version;
    add.id = installs::unique_id(taken, base_id);
    let folder = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    add.name = if add.id == base_id || folder.is_empty() {
        template.name
    } else {
        format!("{} ({folder})", template.name)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::paths::Paths;

    fn state(root: &Path) -> AppState {
        let paths = Paths::at(root.join("capybara"));
        paths.ensure().expect("ensure");
        AppState::new(paths, Config::default(), Vec::new())
    }

    #[test]
    fn the_add_install_dialog_detects_and_proposes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut s = state(tmp.path());
        s.profiles = vec![Profile::vanilla(tmp.path())];
        let d2 = tmp.path().join("Diablo II");
        std::fs::create_dir_all(&d2).expect("mkdir");
        for f in ["d2data.mpq", "d2exp.mpq"] {
            std::fs::write(d2.join(f), b"x").expect("write");
        }
        s.open_add_install(Some(&d2.to_string_lossy()));
        let add = s.settings.add.clone().expect("open");
        assert_eq!(add.detected_kind(), Some(InstallKind::Vanilla));
        assert_eq!(add.id, "vanilla-2", "vanilla is taken");
        assert_eq!(add.version, GameVersion::V114d);
        assert!(add.name.contains("Diablo II"), "{}", add.name);
        assert_eq!(add.blocker(), None);
        let id = s.confirm_add_install().expect("added");
        assert_eq!(id, "vanilla-2");
        assert!(s.paths.profiles.join("vanilla-2.toml").is_file());
        assert!(s.profiles.iter().any(|p| p.id == "vanilla-2"));
        assert!(s.settings.add.is_none());
        assert_eq!(
            s.loading.as_ref().map(|l| l.profile_id.as_str()),
            Some("vanilla-2"),
            "the new profile opens"
        );
        // Nothing appeared in the install folder.
        let mut names: Vec<String> = std::fs::read_dir(&d2)
            .expect("read")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["d2data.mpq", "d2exp.mpq"]);
    }

    #[test]
    fn an_unknown_folder_waits_for_the_users_choice() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut s = state(tmp.path());
        let dir = tmp.path().join("mystery");
        std::fs::create_dir_all(&dir).expect("mkdir");
        s.open_add_install(Some(&dir.to_string_lossy()));
        let add = s.settings.add.clone().expect("open");
        assert_eq!(add.detected_kind(), Some(InstallKind::Unknown));
        assert!(add.blocker().is_some());
        assert!(s.confirm_add_install().is_err());
        s.add_install_set_kind(InstallKind::MedianXl);
        let add = s.settings.add.clone().expect("open");
        assert_eq!(add.id, "mxl");
        assert_eq!(add.version, GameVersion::V113c);
        assert_eq!(add.blocker(), None);
    }

    #[test]
    fn settings_apply_and_validate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut s = state(tmp.path());
        s.apply_setting("gamma", "1.8").expect("gamma");
        assert!((s.config.gamma - 1.8).abs() < 1e-6);
        assert!(s.apply_setting("gamma", "9").is_err());
        s.apply_setting("autosave_minutes", "0").expect("off");
        assert_eq!(s.config.autosave_minutes, 0);
        s.apply_setting("keep_last", "3").expect("keep");
        assert_eq!(s.config.retention().keep_last, 3);
        assert!(s.apply_setting("keep_last", "0").is_err());
        s.apply_setting("max_age_days", "30").expect("age");
        assert_eq!(s.config.retention().max_age_days, Some(30));
        s.apply_setting("select_scope", "visible").expect("scope");
        assert_eq!(s.edit.scope, SelectScope::Visible);
        assert_eq!(s.config.select_scope, SelectScope::Visible);
        s.apply_setting("layer.tag", "on").expect("tag");
        assert!(s.config.layer_visibility.tag);
        assert!(s.apply_setting("layer.roof", "on").is_err());
        assert!(s.apply_setting("colour", "red").is_err());
        assert!(s.config_dirty);
    }
}
