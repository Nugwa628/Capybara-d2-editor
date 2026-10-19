//! The asset browser (P3): the Tiles panel's search grid, the Ctrl+K
//! palette, and the user's recents, favorites, saved searches, aliases,
//! tags and notes (`capybara\user.db`).
//!
//! Searches, thumbnails and lookups run on the [`worker`] thread; this module
//! holds the state the panels draw and the actions they (and the scripts)
//! call. The egui drawing lives in `ui::browser` and `ui::palette`.

pub mod brush;
pub mod live;
pub mod lru;
pub mod order;
pub mod scope;
pub mod worker;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use bevy_egui::egui;
use capy_edit::{CellValue, LayerRef};
use capy_index::{AssetKind, AssetRow, FavoriteGroup, Hit, QueryError, SavedSearch, UserDb, parse_query};
use d2_assets::GamePath;

use self::live::LiveQuery;
use self::lru::Lru;
use self::order::Drop;
use self::scope::Scope;
use self::worker::{Channel, Request, Response, Worker};
use crate::config::AppMode;
use crate::index_task::IndexState;
use crate::state::{AppState, LogLevel, Toast};

/// Thumbnail edge range (Ctrl+wheel), in points.
pub const THUMB_MIN: f32 = 32.0;
pub const THUMB_MAX: f32 = 192.0;
/// Below this edge the grid becomes a list.
pub const LIST_BELOW: f32 = 48.0;
/// Most results the browser asks for.
pub const RESULT_LIMIT: usize = 3000;
/// Results of the Ctrl+K palette.
pub const PALETTE_LIMIT: usize = 60;
/// Recents listed in the browser and the palette.
pub const RECENT_LIMIT: usize = 40;
/// Uploaded thumbnail textures kept.
const THUMB_CACHE: usize = 768;
/// Name of the group a star adds to when there is none.
pub const DEFAULT_GROUP: &str = "Favorites";

pub enum ThumbSlot {
    Pending,
    Ready(egui::TextureHandle),
    Missing,
}

/// Where a live search stands (scripts wait on it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchStatus {
    /// Waiting for the debounce or the worker.
    Pending,
    /// The answer for the current query is on screen.
    Done,
    /// The query does not parse, or the search failed.
    Failed(String),
}

/// What one search shows.
#[derive(Default)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    /// The worker's error, if the search failed.
    pub failed: Option<String>,
    /// The query text does not parse (shown inline, nothing is searched).
    pub parse_error: Option<QueryError>,
    pub millis: u64,
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub live: LiveQuery,
    pub results: SearchResults,
    /// Row under the keyboard cursor.
    pub selected: usize,
    /// Give the text field the focus on the next frame.
    pub focus: bool,
}

/// The user's lists, reloaded from user.db after every change.
#[derive(Default)]
pub struct UserLists {
    /// Each group with its `(profile, key)` items in order (every profile).
    pub groups: Vec<(FavoriteGroup, Vec<(String, String)>)>,
    /// Keys of the open profile, newest first.
    pub recents: Vec<String>,
    pub saved: Vec<SavedSearch>,
    /// Keys of the open profile that are in any group.
    pub starred: HashSet<String>,
    pub dirty: bool,
}

/// A one-line text question (alias, tag, note, group name, search name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Alias(String),
    AddTag(String),
    Note(String),
    /// Create a group, then add the key to it when there is one.
    NewGroup(Option<String>),
    RenameGroup(i64),
    SaveSearch(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPrompt {
    pub kind: PromptKind,
    pub text: String,
    pub error: Option<String>,
}

/// The right-click menu of a tile, with what it shows filled in on open.
#[derive(Clone, Debug)]
pub struct TileMenu {
    pub key: String,
    pub pos: egui::Pos2,
    pub open: bool,
    pub tags: Vec<String>,
    pub note: Option<String>,
    /// Groups holding the key.
    pub in_groups: Vec<i64>,
}

pub struct BrowserState {
    pub query: String,
    pub scope: Scope,
    pub live: LiveQuery,
    pub results: SearchResults,
    /// Key of the selected tile.
    pub selected: Option<String>,
    /// The grid was clicked last (Enter uses the selection as the brush).
    pub grid_focus: bool,
    /// Scroll the grid to the selection on the next frame.
    pub scroll_to_selected: bool,
    pub thumbs: Lru<i64, ThumbSlot>,
    /// Asset rows by key (lookups and search hits). `None`: not in the index.
    pub assets: HashMap<String, Option<AssetRow>>,
    lookups: Vec<String>,
    lookups_pending: HashSet<String>,
    /// "Used in" lists by asset id; `None` while asked.
    pub used_in: HashMap<i64, Option<Result<Vec<(String, u32)>, String>>>,
    /// The "Maps that use it" window: the key.
    pub used_in_window: Option<String>,
    user: Mutex<Option<UserDb>>,
    user_failed: Option<String>,
    pub lists: UserLists,
    pub menu: Option<TileMenu>,
    pub prompt: Option<TextPrompt>,
    pub palette: PaletteState,
    /// The key the current brush was made from, and the value it made.
    pub brush_key: Option<(String, CellValue)>,
    /// Bring the Tiles panel to the front.
    pub focus_tiles: bool,
    worker: Option<Worker>,
    worker_failed: bool,
    /// The profile the caches belong to.
    profile: Option<String>,
    index_ready: bool,
    now: f64,
}

impl Default for BrowserState {
    fn default() -> Self {
        BrowserState {
            query: String::new(),
            scope: Scope::default(),
            live: LiveQuery::default(),
            results: SearchResults::default(),
            selected: None,
            grid_focus: false,
            scroll_to_selected: false,
            thumbs: Lru::new(THUMB_CACHE),
            assets: HashMap::new(),
            lookups: Vec::new(),
            lookups_pending: HashSet::new(),
            used_in: HashMap::new(),
            used_in_window: None,
            user: Mutex::new(None),
            user_failed: None,
            lists: UserLists {
                dirty: true,
                ..UserLists::default()
            },
            menu: None,
            prompt: None,
            palette: PaletteState::default(),
            brush_key: None,
            focus_tiles: false,
            worker: None,
            worker_failed: false,
            profile: None,
            index_ready: false,
            now: 0.0,
        }
    }
}

impl BrowserState {
    /// The thumbnail texture of `asset_id`, asking the worker the first time.
    pub fn thumb(&mut self, asset_id: i64) -> Option<egui::TextureHandle> {
        if let Some(slot) = self.thumbs.get(&asset_id) {
            return match slot {
                ThumbSlot::Ready(t) => Some(t.clone()),
                ThumbSlot::Pending | ThumbSlot::Missing => None,
            };
        }
        let sent = self
            .worker
            .as_ref()
            .is_some_and(|w| w.send(Request::Thumb { asset_id }));
        if sent {
            self.thumbs.insert(asset_id, ThumbSlot::Pending);
        }
        None
    }

    /// The index row of `key` (lower-case), asking the worker when unknown.
    /// `Some(None)` means "not in the index".
    pub fn asset(&mut self, key: &str) -> Option<Option<&AssetRow>> {
        if !self.assets.contains_key(key) {
            if self.lookups_pending.insert(key.to_owned()) {
                self.lookups.push(key.to_owned());
            }
            return None;
        }
        self.assets.get(key).map(Option::as_ref)
    }

    /// The maps using `asset_id`, asking the worker the first time.
    pub fn used_in(&mut self, asset_id: i64) -> Option<&Result<Vec<(String, u32)>, String>> {
        if !self.used_in.contains_key(&asset_id) {
            let sent = self
                .worker
                .as_ref()
                .is_some_and(|w| w.send(Request::UsedIn { asset_id }));
            if sent {
                self.used_in.insert(asset_id, None);
            }
            return None;
        }
        self.used_in.get(&asset_id).and_then(Option::as_ref)
    }

    /// Runs `f` with the user database, opening it on first use.
    fn with_user<T>(
        &mut self,
        path: &std::path::Path,
        f: impl FnOnce(&UserDb) -> capy_index::Result<T>,
    ) -> Result<T, String> {
        let slot = self.user.get_mut().map_err(|_| "user.db is unusable".to_owned())?;
        if slot.is_none() {
            match UserDb::open(path) {
                Ok(db) => *slot = Some(db),
                Err(e) => {
                    let text = format!("cannot open {}: {e}", path.display());
                    self.user_failed = Some(text.clone());
                    return Err(text);
                }
            }
        }
        let db = slot.as_ref().ok_or("user.db is not open")?;
        f(db).map_err(|e| e.to_string())
    }

    pub fn user_problem(&self) -> Option<&str> {
        self.user_failed.as_deref()
    }

    /// The search channels' state for the open profile is moot.
    fn clear_profile_caches(&mut self) {
        self.thumbs.clear();
        self.assets.clear();
        self.lookups.clear();
        self.lookups_pending.clear();
        self.used_in.clear();
        self.results = SearchResults::default();
        self.palette.results = SearchResults::default();
        self.live.reset();
        self.palette.live.reset();
        self.selected = None;
        self.menu = None;
        self.used_in_window = None;
        self.lists.dirty = true;
    }

    /// Puts `query` in the query box and searches it without the debounce.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_owned();
        self.live.flush(self.now);
    }

    fn invalidate_searches(&mut self) {
        let now = self.now;
        self.live.invalidate(now);
        self.palette.live.invalidate(now);
    }
}

/// The text a search key is made of: everything that changes the answer.
fn search_key(profile: &str, scope: &str, dt1s: &[String], query: &str) -> String {
    format!("{profile}\u{1}{scope}\u{1}{}\u{1}{}", dt1s.join("|"), query.trim())
}

/// `true` for keys of sprite files.
pub fn is_sprite_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".dc6", ".dcc", ".cof"].iter().any(|e| lower.ends_with(e))
}

/// The label a tile shows: the alias when there is one, else the end of
/// the generated name.
pub fn display_name(asset: &AssetRow, alias: Option<&str>) -> String {
    match alias {
        Some(a) => a.to_owned(),
        None => asset.gen_name.clone(),
    }
}

impl AppState {
    fn browser_profile(&self) -> Option<String> {
        self.session.as_ref().map(|s| s.profile.id.clone())
    }

    /// DT1 paths of the active map (lower-case, load order); empty without one.
    pub fn map_dt1s(&self) -> Vec<String> {
        self.active_doc()
            .and_then(|d| d.info.as_ref())
            .map(|info| {
                info.dt1_files
                    .iter()
                    .map(|p| p.as_str().to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True when a map is open and `path` is not one of its DT1s.
    pub fn outside_tile_set(&self, asset: &AssetRow) -> bool {
        if asset.kind != AssetKind::Tile {
            return false;
        }
        let dt1s = self.map_dt1s();
        !dt1s.is_empty() && !scope::in_tile_set(&asset.path, &dt1s)
    }

    fn warn_toast(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.log(LogLevel::Warn, text.clone());
        self.toasts.push(Toast {
            level: LogLevel::Warn,
            text,
            shown_at: None,
        });
    }

    /// Once a frame (from the UI pass): start the worker, take its answers,
    /// send due searches and lookups, and reload the user's lists.
    pub fn browser_tick(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.browser.now = now;
        let profile = self.browser_profile();
        if profile != self.browser.profile {
            self.browser.clear_profile_caches();
            self.browser.profile = profile.clone();
        }
        let ready = matches!(self.index.state, IndexState::Ready(_));
        if ready != self.browser.index_ready {
            self.browser.index_ready = ready;
            // A (re)built index: old ids and answers are stale.
            self.browser.clear_profile_caches();
        }
        if ready && self.browser.worker.is_none() && !self.browser.worker_failed {
            match Worker::spawn(self.paths.cache.clone(), self.paths.user_db()) {
                Ok(w) => self.browser.worker = Some(w),
                Err(e) => {
                    self.browser.worker_failed = true;
                    self.error(format!("Asset search: {e}"));
                }
            }
        }
        self.drain_worker(ctx);
        if self.browser.lists.dirty {
            self.refresh_lists();
        }
        let Some(profile) = profile.filter(|_| ready) else {
            return;
        };
        self.send_browser_search(&profile, now);
        self.send_palette_search(&profile, now);
        if !self.browser.lookups.is_empty()
            && let Some(w) = &self.browser.worker
        {
            let keys = std::mem::take(&mut self.browser.lookups);
            w.send(Request::Lookup {
                profile: profile.clone(),
                keys,
            });
        }
    }

    /// The DT1s the browser's scope narrows to (empty unless "this map").
    fn scope_dt1s(&self) -> Vec<String> {
        if self.browser.scope == Scope::MapTileSet {
            self.map_dt1s()
        } else {
            Vec::new()
        }
    }

    /// Where the Tiles browser's search for its current query stands.
    pub fn browser_search_status(&self) -> SearchStatus {
        let Some(profile) = self.browser_profile() else {
            return SearchStatus::Failed("no profile is open".to_owned());
        };
        if let Err(e) = parse_query(&self.browser.query) {
            return SearchStatus::Failed(format!("query {:?}: {e}", self.browser.query));
        }
        let key = search_key(
            &profile,
            self.browser.scope.slug(),
            &self.scope_dt1s(),
            &self.browser.query,
        );
        if !self.browser.live.settled_on(&key) {
            return SearchStatus::Pending;
        }
        match &self.browser.results.failed {
            Some(e) => SearchStatus::Failed(format!("search failed: {e}")),
            None => SearchStatus::Done,
        }
    }

    /// The open profile's recently used keys, newest first, from user.db.
    pub fn recent_keys(&mut self) -> Result<Vec<String>, String> {
        let profile = self.browser_profile().ok_or("no profile is open")?;
        self.user(|db| db.recents(&profile, RECENT_LIMIT))
    }

    fn send_browser_search(&mut self, profile: &str, now: f64) {
        let dt1s = self.scope_dt1s();
        let query = match parse_query(&self.browser.query) {
            Ok(q) => {
                self.browser.results.parse_error = None;
                q
            }
            Err(e) => {
                self.browser.results.parse_error = Some(e);
                return;
            }
        };
        let key = search_key(profile, self.browser.scope.slug(), &dt1s, &self.browser.query);
        self.browser.live.edit(&key, now);
        if let Some((id, _)) = self.browser.live.poll(now) {
            let queries = scope::scoped_queries(&query, self.browser.scope, &dt1s);
            let sent = self.browser.worker.as_ref().is_some_and(|w| {
                w.send(Request::Search {
                    channel: Channel::Browser,
                    id,
                    profile: profile.to_owned(),
                    queries,
                    limit: RESULT_LIMIT,
                })
            });
            if !sent {
                self.browser.live.reset();
            }
        }
    }

    fn send_palette_search(&mut self, profile: &str, now: f64) {
        if !self.browser.palette.open {
            return;
        }
        let text = self.browser.palette.query.clone();
        if text.trim().is_empty() {
            // Recents are shown instead.
            self.browser.palette.results.parse_error = None;
            return;
        }
        let query = match parse_query(&text) {
            Ok(q) => {
                self.browser.palette.results.parse_error = None;
                q
            }
            Err(e) => {
                self.browser.palette.results.parse_error = Some(e);
                return;
            }
        };
        let key = search_key(profile, "all", &[], &text);
        self.browser.palette.live.edit(&key, now);
        if let Some((id, _)) = self.browser.palette.live.poll(now) {
            let sent = self.browser.worker.as_ref().is_some_and(|w| {
                w.send(Request::Search {
                    channel: Channel::Palette,
                    id,
                    profile: profile.to_owned(),
                    queries: vec![query],
                    limit: PALETTE_LIMIT,
                })
            });
            if !sent {
                self.browser.palette.live.reset();
            }
        }
    }

    fn drain_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = &self.browser.worker else {
            return;
        };
        let answers = worker.drain();
        let profile = self.browser.profile.clone();
        for answer in answers {
            match answer {
                Response::Search {
                    channel,
                    id,
                    result,
                    millis,
                } => {
                    let b = &mut self.browser;
                    let (live, results) = match channel {
                        Channel::Browser => (&mut b.live, &mut b.results),
                        Channel::Palette => (&mut b.palette.live, &mut b.palette.results),
                    };
                    if !live.complete(id) {
                        continue;
                    }
                    results.millis = millis;
                    match result {
                        Ok(hits) => {
                            for h in &hits {
                                b.assets.insert(h.asset.key.clone(), Some(h.asset.clone()));
                            }
                            results.hits = hits;
                            results.failed = None;
                        }
                        Err(e) => {
                            results.hits.clear();
                            results.failed = Some(e);
                        }
                    }
                    if channel == Channel::Palette {
                        let n = self.browser.palette.results.hits.len();
                        let sel = &mut self.browser.palette.selected;
                        *sel = (*sel).min(n.saturating_sub(1));
                    }
                }
                Response::Thumb { asset_id, image } => {
                    if !self.browser.thumbs.contains(&asset_id) {
                        continue; // evicted meanwhile
                    }
                    let slot = match image {
                        Some(t) => {
                            let color =
                                egui::ColorImage::from_rgba_unmultiplied([t.width, t.height], &t.rgba);
                            ThumbSlot::Ready(ctx.load_texture(
                                format!("asset-thumb-{asset_id}"),
                                color,
                                egui::TextureOptions::NEAREST,
                            ))
                        }
                        None => ThumbSlot::Missing,
                    };
                    self.browser.thumbs.insert(asset_id, slot);
                }
                Response::Lookup { profile: p, rows } => {
                    if Some(&p) != profile.as_ref() {
                        continue;
                    }
                    for (key, row) in rows {
                        self.browser.lookups_pending.remove(&key);
                        self.browser.assets.insert(key, row);
                    }
                }
                Response::UsedIn { asset_id, maps } => {
                    self.browser.used_in.insert(asset_id, Some(maps));
                }
            }
        }
    }

    // ───────── user lists ─────────

    fn user<T>(&mut self, f: impl FnOnce(&UserDb) -> capy_index::Result<T>) -> Result<T, String> {
        let path = self.paths.user_db();
        self.browser.with_user(&path, f)
    }

    /// Reload groups, recents and saved searches from user.db.
    pub fn refresh_lists(&mut self) {
        self.browser.lists.dirty = false;
        let profile = self.browser_profile().unwrap_or_default();
        let loaded = self.user(|db| {
            let mut groups = Vec::new();
            for g in db.favorite_groups()? {
                let items = db.favorites(g.id)?;
                groups.push((g, items));
            }
            let recents = if profile.is_empty() {
                Vec::new()
            } else {
                db.recents(&profile, RECENT_LIMIT)?
            };
            Ok((groups, recents, db.saved_searches()?))
        });
        match loaded {
            Ok((groups, recents, saved)) => {
                let starred = groups
                    .iter()
                    .flat_map(|(_, items)| items.iter())
                    .filter(|(p, _)| *p == profile)
                    .map(|(_, k)| k.clone())
                    .collect();
                self.browser.lists = UserLists {
                    groups,
                    recents,
                    saved,
                    starred,
                    dirty: false,
                };
            }
            Err(e) => self.warn(format!("user.db: {e}")),
        }
    }

    /// Runs a user.db change; reloads the lists and reports errors.
    fn user_change(&mut self, what: &str, f: impl FnOnce(&UserDb) -> capy_index::Result<()>) -> bool {
        let result = self.user(f);
        self.browser.lists.dirty = true;
        match result {
            Ok(()) => true,
            Err(e) => {
                self.error(format!("{what}: {e}"));
                false
            }
        }
    }

    fn need_profile(&mut self) -> Option<String> {
        let p = self.browser_profile();
        if p.is_none() {
            self.error("No profile is open");
        }
        p
    }

    /// Paint started with `value` on `layer` of document `index`: count it
    /// as a use of its asset (the browser's key, or the tile the map shows).
    pub fn note_paint(&mut self, index: usize, layer: LayerRef, value: CellValue) {
        let key = match &self.browser.brush_key {
            Some((key, v)) if *v == value => Some(key.clone()),
            _ => self.docs.get(index).and_then(|d| asset_key_of(d, layer, value)),
        };
        let (Some(key), Some(profile)) = (key, self.browser_profile()) else {
            return;
        };
        let result = self.user(|db| db.record_use(&profile, &key));
        self.browser.lists.dirty = true;
        if let Err(e) = result {
            self.warn(format!("Cannot record the use of {key}: {e}"));
        }
    }

    /// The id of the first group, creating "Favorites" when there is none.
    fn default_group(&mut self) -> Option<i64> {
        if let Some((g, _)) = self.browser.lists.groups.first() {
            return Some(g.id);
        }
        match self.user(|db| db.create_group(DEFAULT_GROUP)) {
            Ok(g) => {
                self.browser.lists.dirty = true;
                Some(g.id)
            }
            Err(e) => {
                self.error(format!("Cannot create a favorites group: {e}"));
                None
            }
        }
    }

    /// The star: add to the first group, or remove from every group.
    pub fn toggle_star(&mut self, key: &str) {
        let Some(profile) = self.need_profile() else {
            return;
        };
        if self.browser.lists.starred.contains(key) {
            let groups: Vec<i64> = self
                .browser
                .lists
                .groups
                .iter()
                .filter(|(_, items)| items.iter().any(|(p, k)| *p == profile && k == key))
                .map(|(g, _)| g.id)
                .collect();
            self.user_change("Remove favorite", |db| {
                for g in groups {
                    db.remove_favorite(g, &profile, key)?;
                }
                Ok(())
            });
        } else if let Some(group) = self.default_group() {
            self.user_change("Add favorite", |db| db.add_favorite(group, &profile, key));
        }
    }

    /// Adds `key` to the group named `group` (created when missing).
    pub fn favorite_add_named(&mut self, group: &str, key: &str) -> Result<(), String> {
        let profile = self.browser_profile().ok_or("no profile is open")?;
        let result = self.user(|db| {
            let id = match db.favorite_groups()?.into_iter().find(|g| g.name == group) {
                Some(g) => g.id,
                None => db.create_group(group)?.id,
            };
            db.add_favorite(id, &profile, key)
        });
        self.browser.lists.dirty = true;
        result
    }

    pub fn favorite_add(&mut self, group: i64, key: &str) {
        if let Some(profile) = self.need_profile() {
            self.user_change("Add favorite", |db| db.add_favorite(group, &profile, key));
        }
    }

    pub fn favorite_remove(&mut self, group: i64, key: &str) {
        if let Some(profile) = self.need_profile() {
            self.user_change("Remove favorite", |db| db.remove_favorite(group, &profile, key));
        }
    }

    /// Drag-and-drop inside a group's strip.
    pub fn favorite_move(&mut self, group: i64, key: &str, drop: &Drop<String>) {
        let Some(profile) = self.need_profile() else {
            return;
        };
        let Some(items) = self
            .browser
            .lists
            .groups
            .iter()
            .find(|(g, _)| g.id == group)
            .map(|(_, items)| items.clone())
        else {
            return;
        };
        if let Some(pos) = order::favorite_position(&items, &profile, key, drop) {
            self.user_change("Move favorite", |db| {
                db.move_favorite(group, &profile, key, pos)
            });
        }
    }

    /// Drag-and-drop of a group header.
    pub fn group_move(&mut self, group: i64, drop: &Drop<i64>) {
        let ids: Vec<i64> = self.browser.lists.groups.iter().map(|(g, _)| g.id).collect();
        if let Some(pos) = order::move_position(&ids, &group, drop) {
            self.user_change("Move group", |db| db.move_group(group, pos));
        }
    }

    pub fn group_create(&mut self, name: &str, add: Option<&str>) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("type a name".to_owned());
        }
        let profile = self.browser_profile();
        let result = self.user(|db| {
            let g = db.create_group(name)?;
            if let (Some(key), Some(p)) = (add, profile.as_deref()) {
                db.add_favorite(g.id, p, key)?;
            }
            Ok(())
        });
        self.browser.lists.dirty = true;
        result
    }

    pub fn group_rename(&mut self, id: i64, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("type a name".to_owned());
        }
        let r = self.user(|db| db.rename_group(id, name));
        self.browser.lists.dirty = true;
        r
    }

    pub fn group_delete(&mut self, id: i64) {
        self.user_change("Delete group", |db| db.delete_group(id));
    }

    pub fn set_alias(&mut self, key: &str, alias: &str) -> Result<(), String> {
        let profile = self.browser_profile().ok_or("no profile is open")?;
        let text = alias.trim();
        let r = self.user(|db| db.set_alias(&profile, key, (!text.is_empty()).then_some(text)));
        if r.is_ok() {
            // Results show (and match) aliases.
            self.browser.invalidate_searches();
        }
        r
    }

    pub fn add_tag(&mut self, key: &str, tag: &str) -> Result<(), String> {
        let profile = self.browser_profile().ok_or("no profile is open")?;
        let tag = tag.trim();
        if tag.is_empty() {
            return Err("type a tag".to_owned());
        }
        let r = self.user(|db| db.add_tag(&profile, key, tag));
        self.browser.invalidate_searches();
        r
    }

    pub fn remove_tag(&mut self, key: &str, tag: &str) {
        if let Some(profile) = self.need_profile() {
            self.user_change("Remove tag", |db| db.remove_tag(&profile, key, tag));
            self.browser.invalidate_searches();
        }
    }

    pub fn set_note(&mut self, key: &str, note: &str) -> Result<(), String> {
        let profile = self.browser_profile().ok_or("no profile is open")?;
        let text = note.trim();
        self.user(|db| db.set_note(&profile, key, (!text.is_empty()).then_some(text)))
    }

    pub fn save_search(&mut self, name: &str, query: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("type a name".to_owned());
        }
        if let Err(e) = parse_query(query) {
            return Err(e.to_string());
        }
        let r = self.user(|db| {
            let s = db.save_search(name, query)?;
            db.set_search_pinned(s.id, true)
        });
        self.browser.lists.dirty = true;
        r
    }

    pub fn search_pin(&mut self, id: i64, pinned: bool) {
        self.user_change("Pin search", |db| db.set_search_pinned(id, pinned));
    }

    pub fn search_delete(&mut self, id: i64) {
        self.user_change("Delete search", |db| db.delete_search(id));
    }

    /// Opens the tile menu for `key` at `pos`, reading its tags and note.
    pub fn open_tile_menu(&mut self, key: &str, pos: egui::Pos2) {
        let profile = self.browser_profile().unwrap_or_default();
        let (tags, note) = self
            .user(|db| Ok((db.tags(&profile, key)?, db.note(&profile, key)?)))
            .unwrap_or_default();
        let in_groups = self
            .browser
            .lists
            .groups
            .iter()
            .filter(|(_, items)| items.iter().any(|(p, k)| *p == profile && k == key))
            .map(|(g, _)| g.id)
            .collect();
        self.browser.menu = Some(TileMenu {
            key: key.to_owned(),
            pos,
            open: true,
            tags,
            note,
            in_groups,
        });
    }

    /// Answers the text prompt. `Err` keeps it open with the message.
    pub fn answer_prompt(&mut self, prompt: &TextPrompt) -> Result<(), String> {
        let text = prompt.text.clone();
        match &prompt.kind {
            PromptKind::Alias(key) => self.set_alias(key, &text),
            PromptKind::AddTag(key) => self.add_tag(key, &text),
            PromptKind::Note(key) => self.set_note(key, &text),
            PromptKind::NewGroup(add) => self.group_create(&text, add.as_deref()),
            PromptKind::RenameGroup(id) => self.group_rename(*id, &text),
            PromptKind::SaveSearch(query) => self.save_search(&text, query),
        }
    }

    // ───────── using an asset ─────────

    /// Double-click or Enter: a tile becomes the brush, a sprite opens in
    /// Sprites mode, a map opens, a DT1 lists its tiles.
    pub fn use_asset_key(&mut self, key: &str) -> Result<(), String> {
        if brush::parse_tile_key(key).is_some() {
            return self.set_brush_from_key(key);
        }
        let lower = key.to_ascii_lowercase();
        if is_sprite_path(&lower) {
            self.config.mode = AppMode::Sprites;
            self.config_dirty = true;
            return self.open_sprite(&GamePath::new(key));
        }
        if lower.ends_with(".ds1") {
            let path = GamePath::new(key);
            self.open_map(&path);
            return match self.active_doc() {
                Some(d) if d.path == path => Ok(()),
                _ => Err(format!("cannot open {key}")),
            };
        }
        if lower.ends_with(".dt1") {
            let name = lower.rsplit('\\').next().unwrap_or(&lower).to_owned();
            self.browser.query = format!("file:{name}");
            self.browser.scope = Scope::Profile;
            self.browser.live.flush(self.browser.now);
            self.browser.focus_tiles = true;
            return Ok(());
        }
        Err(format!("{key} cannot be used here"))
    }

    /// Makes the tile `key` the paint brush for the active layer and picks
    /// the Paint tool.
    pub fn set_brush_from_key(&mut self, key: &str) -> Result<(), String> {
        let (dt1, id) = brush::parse_tile_key(key).ok_or_else(|| format!("{key} is not a tile key"))?;
        let (active, version) = self
            .active_doc()
            .map_or((LayerRef::Floor(0), 18), |d| (d.active_layer, d.ds1.version));
        let (layer, value) = brush::brush_for(id, active, version)?;
        crate::ui::cancel_map_gesture(self);
        self.edit.pick(layer, value);
        self.edit.tool = crate::edit::tools::Tool::Paint;
        if self.config.mode != AppMode::Map {
            self.config.mode = AppMode::Map;
            self.config_dirty = true;
        }
        let key = key.to_ascii_lowercase();
        self.browser.brush_key = Some((key.clone(), value));
        self.browser.selected = Some(key.clone());
        let missing_layer = match self.active_doc_mut() {
            Some(doc) if layer.exists_in(&doc.ds1) => {
                doc.active_layer = layer;
                false
            }
            Some(_) => true,
            None => false,
        };
        if missing_layer {
            self.warn_toast(format!(
                "This map has no {} layer; add it in the Layers panel to paint there",
                crate::doc::layer_label(layer)
            ));
        }
        let dt1s = self.map_dt1s();
        if !dt1s.is_empty() && !scope::in_tile_set(dt1, &dt1s) {
            let name = dt1.rsplit('\\').next().unwrap_or(dt1);
            self.warn_toast(format!(
                "{name} is not in this map's tile set; tile {}/{} will be missing",
                id.main, id.sub
            ));
        } else {
            self.info(format!(
                "Brush: {key} on {}",
                crate::doc::layer_label(layer)
            ));
        }
        Ok(())
    }

    /// Shift+Enter in the palette: list the asset in the Tiles browser.
    pub fn show_in_browser(&mut self, asset: &AssetRow) {
        let file = asset.path.rsplit('\\').next().unwrap_or(&asset.path);
        self.browser.query = match (asset.kind, asset.orientation, asset.main, asset.sub) {
            (AssetKind::Tile, Some(o), Some(m), Some(s)) => format!("file:{file} idx:{o}:{m}/{s}"),
            _ => format!("file:{file}"),
        };
        self.browser.scope = if is_sprite_path(&asset.path) {
            Scope::Sprites
        } else {
            Scope::Profile
        };
        self.browser.selected = Some(asset.key.clone());
        self.browser.scroll_to_selected = true;
        self.browser.live.flush(self.browser.now);
        self.browser.focus_tiles = true;
        if self.config.mode != AppMode::Map {
            self.config.mode = AppMode::Map;
            self.config_dirty = true;
        }
    }

    /// "Find similar" on a tile.
    pub fn find_similar(&mut self, key: &str) {
        self.browser.query = format!("similar:{key}");
        self.browser.scope = Scope::Profile;
        self.browser.focus_tiles = true;
        self.browser.live.flush(self.browser.now);
    }

    /// Ctrl+K: open (or close) the palette.
    pub fn toggle_palette(&mut self) {
        self.browser.menu = None;
        let p = &mut self.browser.palette;
        p.open = !p.open;
        if p.open {
            p.focus = true;
            p.selected = 0;
        }
    }

    /// Opens the palette with `query` typed in (scripts).
    pub fn open_palette_with(&mut self, query: &str) {
        let now = self.browser.now;
        self.browser.menu = None;
        let p = &mut self.browser.palette;
        p.open = true;
        p.focus = true;
        p.selected = 0;
        p.query = query.to_owned();
        p.live.flush(now);
    }
}

/// The asset key of the tile `value` shows on `layer` of `doc` (through its
/// tile set), for recents of values that did not come from the browser.
fn asset_key_of(doc: &crate::doc::MapDoc, layer: LayerRef, value: CellValue) -> Option<String> {
    let tiles = doc.tiles.as_ref()?;
    let key = crate::ui::tile_key_for(layer, value, doc.ds1.version)?;
    let r = tiles.resolve(key)?;
    let tile = tiles.tile(&r)?;
    let own = tile.key()?;
    let n = tiles
        .candidates(own)
        .iter()
        .filter(|c| c.dt1 == r.dt1 && c.tile < r.tile)
        .count();
    let dt1 = tiles.dt1_paths().get(r.dt1)?;
    Some(brush::tile_key(
        dt1.as_str(),
        brush::TileId {
            orientation: own.orientation,
            main: own.main,
            sub: own.sub,
            n: u32::try_from(n).ok()?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keys_change_with_everything_that_changes_the_answer() {
        let a = search_key("mxl", "map", &["x.dt1".into()], "grass");
        assert_ne!(a, search_key("vanilla", "map", &["x.dt1".into()], "grass"));
        assert_ne!(a, search_key("mxl", "profile", &["x.dt1".into()], "grass"));
        assert_ne!(a, search_key("mxl", "map", &["y.dt1".into()], "grass"));
        assert_ne!(a, search_key("mxl", "map", &["x.dt1".into()], "grass2"));
        assert_eq!(a, search_key("mxl", "map", &["x.dt1".into()], " grass "));
    }

    #[test]
    fn sprite_paths_are_recognised() {
        assert!(is_sprite_path("data\\global\\monsters\\zm\\cof\\ZMNUHTH.COF"));
        assert!(is_sprite_path("a.dc6") && is_sprite_path("a.dcc"));
        assert!(!is_sprite_path("a.dt1") && !is_sprite_path("a.ds1"));
    }

    #[test]
    fn a_brush_from_the_browser_paints_and_is_recorded() {
        let tmp = tempfile::tempdir().expect("tmp");
        let paths = crate::paths::Paths::at(tmp.path().join("capybara"));
        let mut s = AppState::new(paths, crate::config::Config::default(), Vec::new());
        let key = "data\\global\\tiles\\a\\floor.dt1#0/3/1/0";
        s.set_brush_from_key(key).expect("brush");
        assert_eq!(s.edit.current_layer, Some(LayerRef::Floor(0)));
        assert_eq!(s.edit.current.cell, 0x0030_01C2);
        assert_eq!(s.edit.tool, crate::edit::tools::Tool::Paint);
        assert_eq!(s.browser.brush_key.as_ref().map(|(k, _)| k.as_str()), Some(key));
        assert!(s.set_brush_from_key("data\\x.ds1").is_err());
        // Without a profile nothing is recorded, and nothing breaks.
        let value = s.edit.current;
        s.note_paint(0, LayerRef::Floor(0), value);
    }
}
