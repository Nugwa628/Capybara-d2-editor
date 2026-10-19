//! Application state shared by the egui panels and the Bevy render systems.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::prelude::Resource;
use bevy_egui::egui;
use d2_assets::{GamePath, LayerId, MapCatalog, Overlay, Profile};
use d2_formats::ds1::Ds1;

use crate::browser::BrowserState;
use crate::config::Config;
use crate::doc::{MapDoc, RenderState, SourceCategory, SourceInfo, categorize_source};
use crate::edit::state::EditState;
use crate::index_task::{self, IndexState, IndexTask};
use crate::paths::Paths;
use crate::sprites::SpritesState;
use crate::tasks::{self, CatalogData, SessionData, TaskEvent};

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
    /// The request that produced it; background results carry the same number.
    pub generation: u64,
    pub profile: Profile,
    pub overlay: std::sync::Arc<Overlay>,
    pub maps: Vec<GamePath>,
    /// Every `.dc6`/`.dcc`/`.cof` among the known names, sorted.
    pub sprites: Vec<GamePath>,
    /// Every `pal.dat` among the known names.
    pub palettes: Vec<GamePath>,
    pub summary: String,
}

/// The map catalog (LvlType, Def, act and DT1 list per DS1) of the open profile.
pub enum CatalogState {
    /// No profile yet.
    None,
    Building,
    Ready(Box<MapCatalog>),
    Failed(String),
}

impl CatalogState {
    pub fn ready(&self) -> Option<&MapCatalog> {
        match self {
            CatalogState::Ready(c) => Some(c),
            _ => None,
        }
    }
}

/// Edit buffer of the LvlType/Def override pickers for one document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverrideEdit {
    /// The document the buffer was filled for.
    pub doc: Option<u64>,
    pub lvl_type: u32,
    pub def_text: String,
}

impl OverrideEdit {
    /// The Def typed by the user: empty means none, anything else must parse.
    pub fn def(&self) -> Result<Option<u32>, String> {
        let t = self.def_text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        t.parse::<u32>()
            .map(Some)
            .map_err(|_| format!("{t:?} is not a Def number"))
    }
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
    pub catalog: CatalogState,
    pub index: IndexTask,
    /// Debug hook: use this LvlType (and Def) for every map that is opened,
    /// instead of the catalog's best guess. Never persisted.
    pub forced_context: Option<(u32, Option<u32>)>,
    /// Debug hook: do not build the asset index.
    pub index_disabled: bool,
    /// The Properties panel's LvlType/Def pickers.
    pub override_edit: OverrideEdit,
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
    /// Tools, clipboard, dialogs: everything about editing that is not one
    /// document's own state.
    pub edit: EditState,
    /// Sprites mode: viewer, converter, editor, and the map's sprite preview.
    pub sprites: SpritesState,
    /// The asset browser, the Ctrl+K palette and the user's lists.
    pub browser: BrowserState,
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
            catalog: CatalogState::None,
            index: IndexTask::new(),
            forced_context: None,
            index_disabled: false,
            override_edit: OverrideEdit::default(),
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
            edit: EditState::default(),
            sprites: SpritesState::default(),
            browser: BrowserState::default(),
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
        // Whatever the old profile's index was doing is moot now.
        self.index.cancel();
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
        self.poll_sprites();
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
                TaskEvent::Catalog { generation, result } => {
                    if self.session.as_ref().map(|s| s.generation) != Some(generation) {
                        continue;
                    }
                    self.install_catalog(result);
                }
                TaskEvent::Index { generation, event } => {
                    let before = self.index.state.clone();
                    self.index.apply(generation, event);
                    if self.index.state != before {
                        self.log_index_transition(&before);
                    }
                }
                TaskEvent::SavesRefreshed { generation, result } => {
                    if self.session.as_ref().map(|s| s.generation) != Some(generation) {
                        continue;
                    }
                    match result {
                        Ok(text) => self.info(text),
                        Err(e) => self.warn(format!("Asset index: cannot refresh saves: {e}")),
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
        let sprites = crate::sprites::sprite_list(names.iter());
        let palettes = crate::sprites::palette_list(names.iter());
        let generation = self.generation;
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
        self.catalog = CatalogState::Building;
        let overrides = self
            .config
            .overrides_for(&profile.id)
            .map(|o| (GamePath::new(&o.ds1), o.lvl_type, o.def))
            .collect();
        tasks::spawn_catalog(
            generation,
            overlay.clone(),
            profile.game_version,
            maps.clone(),
            overrides,
            self.tx.clone(),
        );
        self.start_index(&profile, overlay.clone());
        self.session = Some(Session {
            generation,
            profile,
            overlay,
            maps,
            sprites,
            palettes,
            summary,
        });
        self.session_epoch += 1;
        self.refresh_changed();
        self.maps_list = MapsList {
            filter: std::mem::take(&mut self.maps_list.filter),
            ..MapsList::default()
        };
    }

    // ───────── map catalog ─────────

    fn install_catalog(&mut self, result: Result<CatalogData, String>) {
        match result {
            Ok(CatalogData {
                catalog,
                warnings,
                elapsed,
            }) => {
                for w in warnings.iter().take(20) {
                    self.warn(format!("Map catalog: {w}"));
                }
                self.info(format!(
                    "Map catalog ready in {:.1}s ({} LvlTypes, {} overrides, {} warnings)",
                    elapsed.as_secs_f32(),
                    catalog.lvl_type_ids().len(),
                    catalog.overrides().len(),
                    warnings.len()
                ));
                self.catalog = CatalogState::Ready(catalog);
                for i in 0..self.docs.len() {
                    self.refresh_doc_info(i);
                }
            }
            Err(e) => {
                self.error(format!("Could not build the map catalog: {e}"));
                self.catalog = CatalogState::Failed(e);
                for doc in &mut self.docs {
                    if doc.render.state == RenderState::WaitingForContext {
                        doc.render.state =
                            RenderState::Failed("the map catalog could not be built".to_owned());
                    }
                }
            }
        }
    }

    /// Looks up how the game loads document `i` (or applies the debug
    /// override) once the catalog is ready.
    fn refresh_doc_info(&mut self, i: usize) {
        let Some(doc) = self.docs.get(i) else { return };
        let in_open_profile = self
            .session
            .as_ref()
            .is_some_and(|s| s.profile.id == doc.profile_id);
        if !in_open_profile {
            return;
        }
        let Some(catalog) = self.catalog.ready() else {
            return;
        };
        let path = doc.path.clone();
        let info = match self.forced_context {
            Some((lvl_type, def)) => catalog.resolve_explicit(&path, lvl_type, def).ok(),
            None => catalog.best(&path),
        };
        if info.is_none() {
            self.warn(format!("No map context for {path}"));
        }
        if let Some(doc) = self.docs.get_mut(i) {
            doc.render.state = if info.is_some() {
                RenderState::Loading
            } else {
                RenderState::Failed("no LvlType found for this map".to_owned())
            };
            doc.info = info;
        }
    }

    /// Use `lvl_type` / `def` for every open copy of document `i`'s map and
    /// remember it for the profile.
    pub fn set_map_override(&mut self, i: usize, lvl_type: u32, def: Option<u32>) {
        let Some(doc) = self.docs.get(i) else { return };
        let (path, profile) = (doc.path.clone(), doc.profile_id.clone());
        if !self
            .session
            .as_ref()
            .is_some_and(|s| s.profile.id == profile)
        {
            self.error("That map belongs to a profile that is not open");
            return;
        }
        let CatalogState::Ready(catalog) = &mut self.catalog else {
            self.error("The map catalog is not ready yet");
            return;
        };
        if let Err(e) = catalog.resolve_explicit(&path, lvl_type, def) {
            self.error(format!("Cannot use LvlType {lvl_type} for {path}: {e}"));
            return;
        }
        catalog.set_override(path.clone(), lvl_type, def);
        self.config
            .set_override(&profile, path.as_str(), lvl_type, def);
        self.config_dirty = true;
        self.refresh_infos_for(&path, &profile);
        self.info(format!(
            "{path}: LvlType {lvl_type}{} chosen by the user",
            def.map(|d| format!(", Def {d}")).unwrap_or_default()
        ));
    }

    /// Drop the user's choice for document `i`'s map.
    pub fn clear_map_override(&mut self, i: usize) {
        let Some(doc) = self.docs.get(i) else { return };
        let (path, profile) = (doc.path.clone(), doc.profile_id.clone());
        if let CatalogState::Ready(catalog) = &mut self.catalog {
            catalog.clear_override(&path);
        }
        self.config.clear_override(&profile, path.as_str());
        self.config_dirty = true;
        self.refresh_infos_for(&path, &profile);
    }

    fn refresh_infos_for(&mut self, path: &GamePath, profile: &str) {
        let indices: Vec<usize> = self
            .docs
            .iter()
            .enumerate()
            .filter(|(_, d)| &d.path == path && d.profile_id == profile)
            .map(|(i, _)| i)
            .collect();
        for i in indices {
            self.refresh_doc_info(i);
        }
    }

    // ───────── asset index ─────────

    fn start_index(&mut self, profile: &Profile, overlay: std::sync::Arc<Overlay>) {
        if self.index_disabled {
            return;
        }
        let (generation, cancel) = self.index.begin();
        let tx = self.tx.clone();
        index_task::spawn(
            self.paths.cache.clone(),
            profile.clone(),
            overlay,
            cancel,
            move |event| {
                let _ = tx.send(TaskEvent::Index { generation, event });
            },
        );
        self.info(format!("Asset index: checking {}", profile.id));
    }

    fn log_index_transition(&mut self, before: &IndexState) {
        match (&self.index.state, before) {
            (IndexState::Ready(s), _) => {
                let text = if s.rebuilt {
                    format!(
                        "Asset index built: {} assets in {:.1}s ({} warnings)",
                        s.assets,
                        s.elapsed.as_secs_f32(),
                        s.warnings
                    )
                } else {
                    format!(
                        "Asset index up to date: {} assets (checked in {:.2}s)",
                        s.assets,
                        s.elapsed.as_secs_f32()
                    )
                };
                self.info(text);
            }
            (IndexState::Cancelled, _) => self.info("Asset index build cancelled"),
            (IndexState::Failed(e), _) => {
                let text = format!("Asset index failed: {e}");
                self.error(text);
            }
            (IndexState::Building(_), IndexState::Checking) => {
                self.info("Asset index is missing or stale; rebuilding in the background");
            }
            _ => {}
        }
    }

    /// After a save: bring the index's view of the profile's saved maps up
    /// to date on a worker thread. Skipped while the index is building or not
    /// built (a build reads the saves anyway).
    pub fn start_saves_refresh(&mut self) {
        if self.index_disabled || !matches!(self.index.state, IndexState::Ready(_)) {
            return;
        }
        let Some(session) = &self.session else { return };
        let generation = session.generation;
        let tx = self.tx.clone();
        index_task::spawn_refresh(
            self.paths.cache.clone(),
            session.profile.clone(),
            session.overlay.clone(),
            move |result| {
                let _ = tx.send(TaskEvent::SavesRefreshed { generation, result });
            },
        );
    }

    /// The Cancel button of the status bar.
    pub fn cancel_index(&mut self) {
        self.index.cancel();
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
        self.refresh_doc_info(self.docs.len() - 1);
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
