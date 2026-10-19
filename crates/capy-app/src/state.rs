//! Application state shared by the egui panels and the Bevy render systems.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::Resource;
use bevy_egui::egui;
use d2_assets::{GamePath, LayerId, Overlay, Profile};
use d2_formats::ds1::Ds1;

use crate::config::Config;
use crate::doc::{MapDoc, SourceCategory, SourceInfo, categorize_source};
use crate::paths::Paths;
use crate::tasks::{self, SessionData, TaskEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
}

pub struct Toast {
    pub level: LogLevel,
    pub text: String,
    /// Set the first time the toast is drawn (egui time, seconds).
    pub shown_at: Option<f64>,
}

/// What a finished background open produced, for the open profile.
pub struct Session {
    pub profile: Profile,
    pub overlay: std::sync::Arc<Overlay>,
    pub maps: Vec<GamePath>,
    pub summary: String,
}

/// A background open in flight.
pub struct Loading {
    pub generation: u64,
    pub profile_id: String,
    pub stage: String,
}

/// State of the Maps panel list.
#[derive(Default)]
pub struct MapsList {
    pub filter: String,
    /// Indices into `Session::maps` that pass the filter.
    pub filtered: Vec<usize>,
    /// What `filtered` was computed from, to detect staleness.
    pub computed_for: Option<(String, u64)>,
    /// Layer label per map path, for the visible rows only.
    pub source_cache: HashMap<usize, (SourceCategory, String)>,
}

#[derive(Resource)]
pub struct AppState {
    pub paths: Paths,
    pub config: Config,
    pub config_dirty: bool,
    pub profiles: Vec<Profile>,
    pub session: Option<Session>,
    /// Bumped whenever `session` changes, to invalidate list caches.
    pub session_epoch: u64,
    pub loading: Option<Loading>,
    generation: u64,
    tx: Sender<TaskEvent>,
    rx: Mutex<Receiver<TaskEvent>>,
    pub docs: Vec<MapDoc>,
    pub active: Option<usize>,
    next_doc_id: u64,
    pub log: Vec<LogEntry>,
    pub toasts: Vec<Toast>,
    pub maps_list: MapsList,
    /// The Map panel's rectangle in egui points, set while drawing the UI.
    pub map_rect: Option<egui::Rect>,
    pub pixels_per_point: f32,
    pub hover_cell: Option<(i32, i32)>,
    pub scroll_accum: f32,
    /// Set by the UI; the app shell clears it after handling.
    pub request_fit: bool,
    pub quit_requested: bool,
}

const MAX_LOG: usize = 2000;

impl AppState {
    pub fn new(paths: Paths, config: Config, profiles: Vec<Profile>) -> AppState {
        let (tx, rx) = channel();
        AppState {
            paths,
            config,
            config_dirty: false,
            profiles,
            session: None,
            session_epoch: 0,
            loading: None,
            generation: 0,
            tx,
            rx: Mutex::new(rx),
            docs: Vec::new(),
            active: None,
            next_doc_id: 1,
            log: Vec::new(),
            toasts: Vec::new(),
            maps_list: MapsList::default(),
            map_rect: None,
            pixels_per_point: 1.0,
            hover_cell: None,
            scroll_accum: 0.0,
            request_fit: false,
            quit_requested: false,
        }
    }

    // ───────── log and toasts ─────────

    pub fn log(&mut self, level: LogLevel, text: impl Into<String>) {
        let text = text.into();
        match level {
            LogLevel::Error => bevy::log::error!("{text}"),
            LogLevel::Warn => bevy::log::warn!("{text}"),
            LogLevel::Info => bevy::log::info!("{text}"),
        }
        self.log.push(LogEntry { level, text });
        if self.log.len() > MAX_LOG {
            let drop = self.log.len() - MAX_LOG;
            self.log.drain(..drop);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.log(LogLevel::Info, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.log(LogLevel::Warn, text);
    }

    /// Log an error and raise a toast.
    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.log(LogLevel::Error, text.clone());
        self.toasts.push(Toast {
            level: LogLevel::Error,
            text,
            shown_at: None,
        });
    }

    // ───────── profiles ─────────

    pub fn active_profile_id(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|s| s.profile.id.as_str())
            .or_else(|| self.loading.as_ref().map(|l| l.profile_id.as_str()))
    }

    /// Open (or switch to) a profile; the overlay and name harvest run on a
    /// worker thread. Results that belong to an older request are ignored.
    pub fn open_profile(&mut self, profile: Profile) {
        self.generation += 1;
        self.loading = Some(Loading {
            generation: self.generation,
            profile_id: profile.id.clone(),
            stage: "Starting".to_owned(),
        });
        self.info(format!("Opening profile {} ({})", profile.name, profile.id));
        self.config.last_profile = Some(profile.id.clone());
        self.config_dirty = true;
        tasks::spawn_open(
            self.generation,
            profile,
            self.paths.clone(),
            self.tx.clone(),
        );
    }

    pub fn open_profile_by_id(&mut self, id: &str) {
        if let Some(p) = self.profiles.iter().find(|p| p.id == id).cloned() {
            self.open_profile(p);
        } else {
            self.error(format!("Unknown profile {id:?}"));
        }
    }

    /// Drain finished background work. Call once per frame.
    pub fn poll_tasks(&mut self) {
        let events: Vec<TaskEvent> = match self.rx.lock() {
            Ok(rx) => rx.try_iter().collect(),
            Err(_) => return,
        };
        for event in events {
            match event {
                TaskEvent::Stage { generation, text } => {
                    if let Some(l) = self.loading.as_mut().filter(|l| l.generation == generation) {
                        l.stage = text;
                    }
                }
                TaskEvent::Finished { generation, result } => {
                    if self.loading.as_ref().map(|l| l.generation) != Some(generation) {
                        continue; // superseded by a newer request
                    }
                    self.loading = None;
                    match result {
                        Ok(data) => self.install_session(data),
                        Err(e) => self.error(format!("Could not open profile: {e}")),
                    }
                }
            }
        }
    }

    fn install_session(&mut self, data: SessionData) {
        let SessionData {
            profile,
            overlay,
            names,
            from_cache,
            summary,
            elapsed,
        } = data;
        let maps = tasks::ds1_list(&names);
        self.info(format!(
            "Profile {} ready in {:.1}s: {summary}; {} maps{}",
            profile.id,
            elapsed.as_secs_f32(),
            maps.len(),
            if from_cache {
                ""
            } else {
                " (names cache refreshed)"
            }
        ));
        self.session = Some(Session {
            profile,
            overlay,
            maps,
            summary,
        });
        self.session_epoch += 1;
        self.maps_list = MapsList {
            filter: std::mem::take(&mut self.maps_list.filter),
            ..MapsList::default()
        };
    }

    // ───────── maps ─────────

    pub fn active_doc(&self) -> Option<&MapDoc> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut MapDoc> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    /// The Map panel rectangle in physical pixels: `[x, y, w, h]`.
    pub fn map_rect_physical(&self, scale_factor: f32) -> Option<[f32; 4]> {
        let r = self.map_rect?;
        let s = scale_factor;
        Some([r.min.x * s, r.min.y * s, r.width() * s, r.height() * s])
    }

    /// Where `path` resolves in the open overlay.
    pub fn source_of(&self, path: &GamePath) -> Option<SourceInfo> {
        let session = self.session.as_ref()?;
        let id = session.overlay.resolve(path)?;
        Some(source_info(&session.overlay, id))
    }

    /// Read `path` through the overlay, parse it and show it as a tab. Errors
    /// go to the log and a toast.
    pub fn open_map(&mut self, path: &GamePath) {
        let Some(session) = self.session.as_ref() else {
            self.error("No profile is open yet");
            return;
        };
        let profile_id = session.profile.id.clone();
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path == *path && d.profile_id == profile_id)
        {
            self.active = Some(i);
            return;
        }
        let overlay = session.overlay.clone();
        let (layer, bytes) = match overlay.read(path) {
            Ok(Some(found)) => found,
            Ok(None) => {
                self.error(format!("{path} was not found in profile {profile_id}"));
                return;
            }
            Err(e) => {
                self.error(format!("Cannot read {path}: {e}"));
                return;
            }
        };
        let source = source_info(&overlay, layer);
        let (ds1, report) = match Ds1::parse_with_report(&bytes) {
            Ok(parsed) => parsed,
            Err(e) => {
                self.error(format!("Cannot parse {path}: {e}"));
                return;
            }
        };
        for w in &report.warnings {
            self.warn(format!("{path}: {w}"));
        }
        if report.missing_bytes > 0 || report.trailing_bytes > 0 {
            self.info(format!(
                "{path}: {} missing and {} trailing bytes (kept as-is)",
                report.missing_bytes, report.trailing_bytes
            ));
        }
        self.info(format!(
            "Opened {path} from {} ({}): v{} {}x{}",
            source.layer_name,
            source.category.label(),
            ds1.version,
            ds1.width,
            ds1.height
        ));
        let id = self.next_doc_id;
        self.next_doc_id += 1;
        self.config.push_recent(&profile_id, path.as_str());
        self.config_dirty = true;
        self.docs.push(MapDoc::new(
            id,
            profile_id,
            path.clone(),
            source,
            ds1,
            &self.config.layer_visibility,
        ));
        self.active = Some(self.docs.len() - 1);
    }

    pub fn close_doc(&mut self, index: usize) {
        if index >= self.docs.len() {
            return;
        }
        self.docs.remove(index);
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(a) if a > index => Some(a - 1),
            Some(a) if a == index => Some(index.min(self.docs.len() - 1)),
            other => other,
        };
        self.hover_cell = None;
    }

    // ───────── Maps list ─────────

    /// Recompute the filtered index list when the filter or session changed.
    pub fn refresh_maps_filter(&mut self) {
        let key = (self.maps_list.filter.clone(), self.session_epoch);
        if self.maps_list.computed_for.as_ref() == Some(&key) {
            return;
        }
        self.maps_list.filtered = match &self.session {
            Some(s) => filter_paths(&s.maps, &self.maps_list.filter),
            None => Vec::new(),
        };
        self.maps_list.computed_for = Some(key);
    }
}

pub fn source_info(overlay: &Overlay, id: LayerId) -> SourceInfo {
    match overlay.layers().get(id.0) {
        Some(layer) => SourceInfo {
            category: categorize_source(
                layer.name(),
                layer.kind() == d2_assets::SourceKind::Folder,
            ),
            layer_name: layer.name().to_owned(),
        },
        None => SourceInfo {
            category: SourceCategory::Base,
            layer_name: "?".to_owned(),
        },
    }
}

/// Case-insensitive substring filter over game paths (pure, for tests).
pub fn filter_paths(paths: &[GamePath], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    paths
        .iter()
        .enumerate()
        .filter(|(_, p)| needle.is_empty() || p.as_str().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths::at(tmp.path().join("capybara"));
        AppState::new(paths, Config::default(), Vec::new())
    }

    #[test]
    fn filter_is_case_insensitive_substring() {
        let paths: Vec<GamePath> = ["data\\a\\Town.ds1", "data\\b\\wild.ds1"]
            .iter()
            .map(|s| GamePath::new(s))
            .collect();
        assert_eq!(filter_paths(&paths, "TOWN"), vec![0]);
        assert_eq!(filter_paths(&paths, ""), vec![0, 1]);
        assert_eq!(filter_paths(&paths, "\\b\\"), vec![1]);
        assert!(filter_paths(&paths, "zzz").is_empty());
    }

    #[test]
    fn closing_tabs_keeps_a_valid_active_index() {
        let mut s = state();
        for i in 0..3 {
            s.docs.push(MapDoc::new(
                i,
                "p".into(),
                GamePath::new(&format!("data\\{i}.ds1")),
                SourceInfo {
                    category: SourceCategory::Base,
                    layer_name: "x".into(),
                },
                Ds1::new(2, 2, 0),
                &Config::default().layer_visibility,
            ));
        }
        s.active = Some(2);
        s.close_doc(0);
        assert_eq!(s.active, Some(1));
        s.close_doc(1);
        assert_eq!(s.active, Some(0));
        s.close_doc(0);
        assert_eq!(s.active, None);
        s.close_doc(5); // out of range is a no-op
    }

    #[test]
    fn open_map_without_profile_is_an_error_not_a_crash() {
        let mut s = state();
        s.open_map(&GamePath::new("data\\x.ds1"));
        assert!(s.docs.is_empty());
        assert_eq!(s.toasts.len(), 1);
    }
}
