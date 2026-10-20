//! Tiles panel: the asset browser (P3), laid out like ds1-studio's tile
//! palette: category chips (Floors, Walls, Shadows, Objects, Roofs,
//! Special) and a DT1 drop-down over the filter box, which keeps Capybara's
//! query language (the chips and the drop-down only edit its `type:` and
//! `file:` tokens). Ctrl+click adds tiles to the random paint mix.
//!
//! A query box over `Index::search` (live, debounced), scope toggles, pinned
//! saved searches, the Recent and Favorites sections, and a virtualised
//! grid of index thumbnails (a list at the smallest sizes). Ctrl+wheel
//! resizes the thumbnails. Click selects, double-click or Enter uses the
//! asset (a tile becomes the brush), right-click opens the tile menu, the
//! star adds to favorites. Tiles from outside the active map's tile set
//! carry an orange "!" badge.

use bevy_egui::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2,
};
use capy_index::{AssetKind, AssetRow};

use crate::browser::order::Drop;
use crate::browser::scope::Scope;
use crate::browser::{LIST_BELOW, PromptKind, THUMB_MAX, THUMB_MIN, TextPrompt, display_name};
use crate::index_task::IndexState;
use crate::state::AppState;

const STAR_ON: Color32 = Color32::from_rgb(255, 205, 60);
const STAR_OFF: Color32 = Color32::from_gray(120);
const WARN: Color32 = Color32::from_rgb(235, 150, 40);
const SELECTED: Color32 = Color32::from_rgb(255, 220, 90);
const ERROR: Color32 = Color32::from_rgb(235, 90, 90);
/// Edge of the Recent and Favorites strips' thumbnails.
const STRIP: f32 = 44.0;

/// A tile dragged from anywhere (adds to a favorites group).
#[derive(Clone, Debug)]
struct TileDrag(String);
/// A favorite dragged inside its group.
#[derive(Clone, Debug)]
struct FavDrag {
    group: i64,
    key: String,
}
/// A favorites group header dragged.
#[derive(Clone, Copy, Debug)]
struct GroupDrag(i64);

/// What one tile widget reported.
#[derive(Default)]
struct TileEvents {
    clicked: bool,
    double: bool,
    menu: Option<Pos2>,
    star: bool,
    /// Ctrl was held on the click (random mix).
    ctrl: bool,
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    // Wide content (a long hint, say) grows the ui past the panel; the grid
    // is laid out for what is on screen.
    let panel = ui.max_rect().intersect(ui.clip_rect());
    resize_with_ctrl_wheel(ui, state, panel);
    studio_bar(ui, state);
    query_bar(ui, state);
    status_line(ui, state);
    if state.session.is_none() {
        return;
    }
    if state.browser.scope == Scope::MapTileSet && state.map_dt1s().is_empty() {
        state.browser.scope = Scope::Profile;
    }
    egui::CollapsingHeader::new("Search options")
        .id_salt("tiles_search_options")
        .default_open(false)
        .show(ui, |ui| {
            scope_bar(ui, state);
            saved_searches(ui, state);
            recent_section(ui, state);
            favorites_section(ui, state);
        });
    ui.add_space(2.0);
    results_grid(ui, state, panel);
    enter_uses_selection(ui, state);
}

fn resize_with_ctrl_wheel(ui: &Ui, state: &mut AppState, panel: Rect) {
    let hovered = ui.rect_contains_pointer(panel);
    if !hovered {
        return;
    }
    let zoom = ui.input(|i| i.zoom_delta());
    if (zoom - 1.0).abs() > f32::EPSILON {
        let size = (state.config.browser_thumb * zoom).clamp(THUMB_MIN, THUMB_MAX);
        if (size - state.config.browser_thumb).abs() > 0.01 {
            state.config.browser_thumb = size;
            state.config_dirty = true;
        }
    }
}

fn query_bar(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut state.browser.query)
                .id(Id::new("browser_query"))
                .hint_text(
                    "Search: grass  type:wall  act:1  color:green  tag:x  idx:5/2  file:  similar:",
                )
                .desired_width(ui.available_width() - 28.0),
        );
        if edit.has_focus() {
            state.browser.grid_focus = false;
        }
        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let now = ui.input(|i| i.time);
            state.browser.live.flush(now);
        }
        if ui
            .small_button("x")
            .on_hover_text("Clear the query")
            .clicked()
        {
            state.browser.query.clear();
        }
    });
    if let Some(err) = &state.browser.results.parse_error {
        parse_error(ui, &state.browser.query, err.position, &err.detail);
    }
}

/// The query with a caret under the bad position.
pub fn parse_error(ui: &mut Ui, query: &str, position: usize, detail: &str) {
    let chars = query
        .get(..position.min(query.len()))
        .map_or(position, |s| s.chars().count());
    ui.label(
        RichText::new(format!("{query}\n{}^ {detail}", " ".repeat(chars)))
            .monospace()
            .color(ERROR),
    );
}

fn scope_bar(ui: &mut Ui, state: &mut AppState) {
    let has_map = !state.map_dt1s().is_empty();
    ui.horizontal_wrapped(|ui| {
        for scope in Scope::ALL {
            let enabled = scope != Scope::MapTileSet || has_map;
            let resp = ui.add_enabled(
                enabled,
                egui::Button::selectable(state.browser.scope == scope, scope.label()),
            );
            let resp = if scope == Scope::MapTileSet && !has_map {
                resp.on_disabled_hover_text("Open a map whose LvlType is known")
            } else {
                resp
            };
            if resp.clicked() {
                state.browser.scope = scope;
            }
        }
        ui.weak(format!("{:.0}px", state.config.browser_thumb))
            .on_hover_text("Ctrl+mouse wheel over the panel resizes the thumbnails");
    });
    if state.browser.scope == Scope::MapTileSet && !has_map {
        state.browser.scope = Scope::Profile;
    }
}

fn saved_searches(ui: &mut Ui, state: &mut AppState) {
    let saved = state.browser.lists.saved.clone();
    let mut apply: Option<String> = None;
    let mut unpin = None;
    ui.horizontal_wrapped(|ui| {
        for s in saved.iter().filter(|s| s.pinned) {
            let resp = ui
                .add(egui::Button::new(RichText::new(&s.name).strong()).small())
                .on_hover_text(format!("Saved search: {}", s.query));
            if resp.clicked() {
                apply = Some(s.query.clone());
            }
            resp.context_menu(|ui| {
                if ui.button("Unpin").clicked() {
                    unpin = Some(s.id);
                    ui.close();
                }
            });
        }
        ui.menu_button("Saved searches", |ui| {
            if saved.is_empty() {
                ui.weak("None yet");
            }
            for s in &saved {
                ui.horizontal(|ui| {
                    if ui.button(&s.name).on_hover_text(&s.query).clicked() {
                        apply = Some(s.query.clone());
                        ui.close();
                    }
                    let pin = if s.pinned { "Unpin" } else { "Pin" };
                    if ui.small_button(pin).clicked() {
                        state.search_pin(s.id, !s.pinned);
                    }
                    if ui.small_button("Delete").clicked() {
                        state.search_delete(s.id);
                    }
                });
            }
            ui.separator();
            if ui.button("Save the current query...").clicked() {
                state.browser.prompt = Some(TextPrompt {
                    kind: PromptKind::SaveSearch(state.browser.query.clone()),
                    text: String::new(),
                    error: None,
                });
                ui.close();
            }
        });
    });
    if let Some(id) = unpin {
        state.search_pin(id, false);
    }
    if let Some(q) = apply {
        state.browser.query = q;
        let now = ui.input(|i| i.time);
        state.browser.live.flush(now);
    }
}

fn status_line(ui: &mut Ui, state: &mut AppState) {
    if state.session.is_none() {
        ui.weak("Pick a profile in the top bar.");
        return;
    }
    if let Some(p) = state.browser.user_problem() {
        ui.colored_label(ERROR, p.to_owned());
    }
    let text = match &state.index.state {
        IndexState::Ready(_) => {
            let r = &state.browser.results;
            match &r.failed {
                Some(e) => {
                    ui.colored_label(ERROR, format!("Search failed: {e}"));
                    return;
                }
                None => {
                    let busy = if state.browser.live.busy() {
                        " ..."
                    } else {
                        ""
                    };
                    format!("{} result(s) in {} ms{busy}", r.hits.len(), r.millis)
                }
            }
        }
        IndexState::Building(_) | IndexState::Checking => {
            "Indexing assets in the background; search starts when it is done.".to_owned()
        }
        IndexState::Idle => "The asset index is off.".to_owned(),
        IndexState::Cancelled => "The asset index build was cancelled.".to_owned(),
        IndexState::Failed(e) => format!("The asset index failed: {e}"),
    };
    ui.weak(text);
}

// ───────────────────────── tile widgets ─────────────────────────

/// Paints a thumbnail (or the asset kind) fitted into `rect`.
fn paint_thumb(ui: &Ui, state: &mut AppState, asset: &AssetRow, rect: Rect) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_gray(34));
    match state.browser.thumb(asset.id) {
        Some(t) => {
            let ts = t.size_vec2();
            let scale = (rect.width() / ts.x).min(rect.height() / ts.y);
            let target = Rect::from_center_size(rect.center(), ts * scale);
            painter.image(
                t.id(),
                target,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let label = match asset.kind {
                AssetKind::Tile => match (asset.main, asset.sub) {
                    (Some(m), Some(s)) => format!("{m}/{s}"),
                    _ => "tile".to_owned(),
                },
                kind => kind.as_str().to_ascii_uppercase(),
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::monospace((rect.height() / 4.0).clamp(8.0, 13.0)),
                Color32::from_gray(160),
            );
        }
    }
}

fn paint_badges(ui: &Ui, rect: Rect, starred: bool, outside: bool, star_rect: Rect) {
    let painter = ui.painter();
    let star = if starred { "★" } else { "☆" };
    painter.text(
        star_rect.center(),
        Align2::CENTER_CENTER,
        star,
        FontId::proportional(star_rect.height()),
        if starred { STAR_ON } else { STAR_OFF },
    );
    if outside {
        let c = rect.left_top() + Vec2::splat(8.0);
        painter.circle_filled(c, 7.0, WARN);
        painter.text(
            c,
            Align2::CENTER_CENTER,
            "!",
            FontId::proportional(11.0),
            Color32::BLACK,
        );
    }
}

/// The hover card: names, key, size, rarity, source, maps that use it.
fn details(ui: &mut Ui, state: &mut AppState, asset: &AssetRow, alias: Option<&str>) {
    if let Some(a) = alias {
        ui.label(RichText::new(a).strong());
    }
    ui.label(&asset.gen_name);
    ui.label(RichText::new(&asset.key).monospace().small());
    let mut facts = Vec::new();
    if let (Some(w), Some(h)) = (asset.width, asset.height) {
        facts.push(format!("{w} x {h} px"));
    }
    if let Some(r) = asset.rarity {
        facts.push(format!("rarity {r}"));
    }
    if let (Some(d), Some(f)) = (asset.directions, asset.frames) {
        facts.push(format!("{d} dir x {f} frames"));
    }
    facts.push(format!("from {} ({})", asset.src, asset.src_tag));
    ui.label(facts.join(", "));
    if asset.kind == AssetKind::Tile {
        let used = match state.browser.used_in(asset.id) {
            Some(Ok(maps)) => {
                let cells: u32 = maps.iter().map(|(_, c)| c).sum();
                format!("used in {} map(s), {cells} cell(s)", maps.len())
            }
            Some(Err(e)) => format!("used in: {e}"),
            None => "used in: ...".to_owned(),
        };
        ui.label(used);
        if state.outside_tile_set(asset) {
            ui.colored_label(
                WARN,
                "Not in this map's tile set: its DT1 isn't loaded by the map's level type, \
                 so it would be invisible in game. Right-click \u{2192} Add to this map.",
            );
        }
    }
}

/// One grid cell: thumbnail, star, badge, label.
fn grid_tile(
    ui: &mut Ui,
    state: &mut AppState,
    asset: &AssetRow,
    alias: Option<&str>,
    size: f32,
) -> TileEvents {
    let label_h = 16.0;
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(size + 6.0, size + label_h + 6.0),
        Sense::click_and_drag(),
    );
    let thumb = Rect::from_min_size(rect.min + Vec2::splat(3.0), Vec2::splat(size));
    paint_thumb(ui, state, asset, thumb);
    let starred = state.browser.lists.starred.contains(&asset.key);
    let star_rect = Rect::from_min_size(
        Pos2::new(thumb.right() - 16.0, thumb.top()),
        Vec2::splat(16.0),
    );
    let outside = state.outside_tile_set(asset);
    paint_badges(ui, thumb, starred, outside, star_rect);
    let label_rect = Rect::from_min_size(
        Pos2::new(rect.left() + 2.0, thumb.bottom() + 2.0),
        Vec2::new(rect.width() - 4.0, label_h),
    );
    let mut job = egui::text::LayoutJob::default();
    label_job(&mut job, asset, alias, 11.0, false, &state.shell.shades);
    job.wrap = egui::text::TextWrapping::truncate_at_width(label_rect.width());
    let galley = ui.painter().layout_job(job);
    ui.painter_at(label_rect)
        .galley(label_rect.min, galley, Color32::WHITE);
    let selected = state.browser.selected.as_deref() == Some(asset.key.as_str());
    let stroke = if selected {
        Stroke::new(2.0, SELECTED)
    } else if resp.hovered() {
        Stroke::new(1.0, state.shell.shades.border2)
    } else {
        Stroke::NONE
    };
    ui.painter()
        .rect_stroke(rect, CornerRadius::same(3), stroke, StrokeKind::Inside);
    events(ui, state, resp, asset, alias, Some(star_rect))
}

/// One list row (smallest sizes).
fn list_row(
    ui: &mut Ui,
    state: &mut AppState,
    asset: &AssetRow,
    alias: Option<&str>,
    height: f32,
) -> TileEvents {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click_and_drag(),
    );
    let selected = state.browser.selected.as_deref() == Some(asset.key.as_str());
    if selected {
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgba_unmultiplied(255, 220, 90, 40));
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, state.shell.shades.hover);
    }
    let star_edge = (height - 4.0).min(16.0);
    let star_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 2.0 + star_edge / 2.0, rect.center().y),
        Vec2::splat(star_edge),
    );
    let icon = Rect::from_min_size(
        Pos2::new(star_rect.right() + 4.0, rect.top() + 1.0),
        Vec2::new((height - 2.0) * 2.0, height - 2.0),
    );
    paint_thumb(ui, state, asset, icon);
    let starred = state.browser.lists.starred.contains(&asset.key);
    let outside = state.outside_tile_set(asset);
    // The warning badge sits at the right end, the text stops before it.
    let badge = Rect::from_min_size(
        Pos2::new(rect.right() - 17.0, rect.center().y - 8.0),
        Vec2::splat(16.0),
    );
    paint_badges(ui, badge, starred, outside, star_rect);
    let text_left = icon.right() + 6.0;
    let text_right = if outside {
        badge.left() - 4.0
    } else {
        rect.right() - 2.0
    };
    let mut job = egui::text::LayoutJob::default();
    label_job(&mut job, asset, alias, 12.0, true, &state.shell.shades);
    job.wrap = egui::text::TextWrapping::truncate_at_width((text_right - text_left).max(10.0));
    let galley = ui.painter().layout_job(job);
    let pos = Pos2::new(text_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, Color32::WHITE);
    events(ui, state, resp, asset, alias, Some(star_rect))
}

/// A generated tile name with the parts that tell tiles of one DT1 apart
/// first: `Act1 Town · Floor · grass · 05/02` gives
/// (`05/02 · Floor · grass`, `Act1 Town`). Other names stay whole.
pub fn short_name(name: &str) -> (String, Option<&str>) {
    let parts: Vec<&str> = name.split(" · ").collect();
    match parts.as_slice() {
        [place, class, descriptor, index] => {
            (format!("{index} · {class} · {descriptor}"), Some(*place))
        }
        _ => (name.to_owned(), None),
    }
}

/// The label of a tile: its alias, or its short name (with the place,
/// dimmed, when `place` is asked for).
fn label_job(
    job: &mut egui::text::LayoutJob,
    asset: &AssetRow,
    alias: Option<&str>,
    size: f32,
    place: bool,
    shades: &super::theme::Shades,
) {
    let format = |color: Color32| egui::text::TextFormat {
        font_id: FontId::proportional(size),
        color,
        ..Default::default()
    };
    if let Some(a) = alias {
        job.append(a, 0.0, format(shades.accent_text));
        if place {
            job.append(&asset.gen_name, 8.0, format(shades.muted));
        }
        return;
    }
    let name = display_name(asset, None);
    let (short, where_) = short_name(&name);
    job.append(&short, 0.0, format(shades.text));
    if place && let Some(w) = where_ {
        job.append(w, 8.0, format(shades.muted));
    }
}

fn events(
    ui: &Ui,
    state: &mut AppState,
    resp: egui::Response,
    asset: &AssetRow,
    alias: Option<&str>,
    star: Option<Rect>,
) -> TileEvents {
    resp.dnd_set_drag_payload(TileDrag(asset.key.clone()));
    let mut ev = TileEvents::default();
    let on_star = star.is_some_and(|r| {
        resp.interact_pointer_pos()
            .is_some_and(|p| r.expand(2.0).contains(p))
    });
    if resp.clicked() {
        if on_star {
            ev.star = true;
        } else {
            ev.clicked = true;
            ev.ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        }
    }
    if resp.double_clicked() && !on_star {
        ev.double = true;
    }
    if resp.secondary_clicked() {
        ev.menu = resp
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_latest_pos());
    }
    resp.on_hover_ui(|ui| details(ui, state, asset, alias));
    ev
}

/// Applies a tile widget's events.
fn handle(state: &mut AppState, key: &str, ev: TileEvents) {
    if ev.star {
        state.toggle_star(key);
    }
    if ev.clicked && ev.ctrl {
        add_to_mix(state, key);
        return;
    }
    if ev.clicked || ev.double {
        state.browser.selected = Some(key.to_owned());
        state.browser.grid_focus = true;
    }
    if ev.double
        && let Err(e) = state.use_asset_key(key)
    {
        state.error(e);
    }
    if let Some(pos) = ev.menu {
        state.browser.selected = Some(key.to_owned());
        state.open_tile_menu(key, pos);
    }
}

/// Ctrl+click: the tile joins (or leaves) the random paint mix.
fn add_to_mix(state: &mut AppState, key: &str) {
    let Some((_, id)) = crate::browser::brush::parse_tile_key(key) else {
        return;
    };
    let (active, version) = state
        .active_doc()
        .map_or((capy_edit::LayerRef::Floor(0), 18), |d| {
            (d.active_layer, d.ds1.version)
        });
    match crate::browser::brush::brush_for(id, active, version) {
        Ok((layer, value)) => {
            state.browser.selected = Some(key.to_owned());
            super::studio::paint::toggle_mix(state, layer, value);
        }
        Err(e) => state.error(e),
    }
}

// ───────────────────────── category chips and DT1 ─────────────────────────

/// ds1-studio's tile categories and the query token each stands for.
pub const CATEGORIES: [(&str, &str); 6] = [
    ("Floors", "type:floor"),
    ("Walls", "type:wall"),
    ("Shadows", "type:shadow"),
    ("Objects", "type:tree"),
    ("Roofs", "type:roof"),
    ("Special", "type:special"),
];

/// `query` with every token starting with one of `prefixes` dropped and
/// `add` (if any) appended.
pub fn with_token(query: &str, prefixes: &[&str], add: Option<&str>) -> String {
    let mut parts: Vec<&str> = query
        .split_whitespace()
        .filter(|t| {
            let lower = t.to_ascii_lowercase();
            !prefixes.iter().any(|p| lower.starts_with(p))
        })
        .collect();
    if let Some(a) = add {
        parts.push(a);
    }
    parts.join(" ")
}

fn has_token(query: &str, token: &str) -> bool {
    query
        .split_whitespace()
        .any(|t| t.eq_ignore_ascii_case(token))
}

const CLASS_TOKENS: [&str; 1] = ["type:"];

fn set_query(ui: &Ui, state: &mut AppState, query: String) {
    state.browser.query = query;
    let now = ui.input(|i| i.time);
    state.browser.live.flush(now);
}

/// Category chips and the DT1 drop-down.
fn studio_bar(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (label, token) in CATEGORIES {
            let on = has_token(&state.browser.query, token);
            if super::shell::chip(ui, &s, label, on, true)
                .on_hover_text(format!("{label} ({token})"))
                .clicked()
            {
                let q = with_token(&state.browser.query, &CLASS_TOKENS, (!on).then_some(token));
                set_query(ui, state, q);
            }
        }
    });
    let dt1s: Vec<String> = state.map_dt1s();
    if dt1s.is_empty() {
        return;
    }
    let current = state
        .browser
        .query
        .split_whitespace()
        .find_map(|t| t.strip_prefix("file:"))
        .map(str::to_owned);
    let name = |p: &str| p.rsplit('\\').next().unwrap_or(p).to_owned();
    ui.horizontal(|ui| {
        ui.label(RichText::new("DT1").size(11.5).color(s.muted));
        egui::ComboBox::from_id_salt("tiles_dt1")
            .width(ui.available_width() - 8.0)
            .selected_text(
                current
                    .clone()
                    .unwrap_or_else(|| format!("Every DT1 of the map ({})", dt1s.len())),
            )
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current.is_none(), "Every DT1 of the map")
                    .clicked()
                {
                    let q = with_token(&state.browser.query, &["file:"], None);
                    state.browser.scope = crate::browser::scope::Scope::MapTileSet;
                    set_query(ui, state, q);
                }
                for p in &dt1s {
                    let n = name(p);
                    let on = current.as_deref() == Some(n.as_str());
                    if ui.selectable_label(on, &n).on_hover_text(p).clicked() {
                        let token = format!("file:{n}");
                        let q = with_token(&state.browser.query, &["file:"], Some(&token));
                        state.browser.scope = crate::browser::scope::Scope::MapTileSet;
                        set_query(ui, state, q);
                    }
                }
            });
    });
}

// ───────────────────────── sections ─────────────────────────

/// A small strip tile for a key (looked up in the index). `None` while the
/// row is being looked up or when the key is not in the index.
fn strip_tile(ui: &mut Ui, state: &mut AppState, key: &str) -> Option<(egui::Response, AssetRow)> {
    let asset = match state.browser.asset(key) {
        Some(Some(a)) => a.clone(),
        Some(None) => {
            ui.add_sized(
                Vec2::splat(STRIP),
                egui::Label::new(RichText::new("?").weak()),
            )
            .on_hover_text(format!("{key}\nis not in this profile's index"));
            return None;
        }
        None => {
            ui.add_sized(Vec2::splat(STRIP), egui::Spinner::new());
            return None;
        }
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(STRIP), Sense::click_and_drag());
    paint_thumb(ui, state, &asset, rect.shrink(1.0));
    let outside = state.outside_tile_set(&asset);
    if outside {
        let c = rect.left_top() + Vec2::splat(6.0);
        ui.painter().circle_filled(c, 5.0, WARN);
    }
    let selected = state.browser.selected.as_deref() == Some(key);
    if selected || resp.hovered() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(if selected { 2.0 } else { 1.0 }, SELECTED),
            StrokeKind::Inside,
        );
    }
    Some((resp, asset))
}

fn recent_section(ui: &mut Ui, state: &mut AppState) {
    let recents = state.browser.lists.recents.clone();
    egui::CollapsingHeader::new(format!("Recent ({})", recents.len()))
        .id_salt("browser_recent")
        .default_open(true)
        .show(ui, |ui| {
            if recents.is_empty() {
                ui.weak("Tiles you paint with collect here.");
                return;
            }
            egui::ScrollArea::horizontal()
                .id_salt("recent_strip")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for key in &recents {
                            if let Some((resp, asset)) = strip_tile(ui, state, key) {
                                let alias = None;
                                let ev = events(ui, state, resp, &asset, alias, None);
                                handle(state, key, ev);
                            }
                        }
                    });
                });
        });
}

fn favorites_section(ui: &mut Ui, state: &mut AppState) {
    let groups = state.browser.lists.groups.clone();
    let profile = state
        .session
        .as_ref()
        .map(|s| s.profile.id.clone())
        .unwrap_or_default();
    let count = match groups.len() {
        1 => "1 group".to_owned(),
        n => format!("{n} groups"),
    };
    let header = egui::CollapsingHeader::new(format!("Favorites ({count})"))
        .id_salt("browser_favorites")
        .default_open(true);
    header.show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.small_button("+ Group").clicked() {
                state.browser.prompt = Some(TextPrompt {
                    kind: PromptKind::NewGroup(None),
                    text: String::new(),
                    error: None,
                });
            }
            let hint = "Drag tiles onto a group to add them, drag to reorder; \
                        the star adds to the first group.";
            ui.add(egui::Label::new(RichText::new(hint).weak()).truncate())
                .on_hover_text(hint);
        });
        if groups.is_empty() {
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("favorites_scroll")
            .max_height(220.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (group, items) in &groups {
                    favorite_group(ui, state, &profile, group, items);
                }
            });
    });
}

fn favorite_group(
    ui: &mut Ui,
    state: &mut AppState,
    profile: &str,
    group: &capy_index::FavoriteGroup,
    items: &[(String, String)],
) {
    let keys: Vec<&String> = items
        .iter()
        .filter(|(p, _)| p == profile)
        .map(|(_, k)| k)
        .collect();
    // Header: drag handle, name, count, actions; drop target for groups.
    let header = ui.horizontal(|ui| {
        let handle = ui
            .add(egui::Label::new(RichText::new("::").monospace().weak()).sense(Sense::drag()))
            .on_hover_text("Drag to reorder the groups");
        handle.dnd_set_drag_payload(GroupDrag(group.id));
        ui.label(RichText::new(&group.name).strong());
        ui.weak(format!("({})", keys.len()));
        if ui.small_button("Rename").clicked() {
            state.browser.prompt = Some(TextPrompt {
                kind: PromptKind::RenameGroup(group.id),
                text: group.name.clone(),
                error: None,
            });
        }
        if ui
            .small_button("Delete")
            .on_hover_text("Deletes the group (the tiles stay in the index)")
            .clicked()
        {
            state.group_delete(group.id);
        }
    });
    let header_resp = header.response;
    if let Some(dragged) = header_resp.dnd_hover_payload::<GroupDrag>()
        && dragged.0 != group.id
    {
        let y = header_resp.rect.top();
        ui.painter()
            .hline(header_resp.rect.x_range(), y, Stroke::new(2.0, SELECTED));
    }
    if let Some(dragged) = header_resp.dnd_release_payload::<GroupDrag>() {
        state.group_move(dragged.0, &Drop::Before(group.id));
    }

    // The strip: items, then an end zone.
    let strip = ui.horizontal_wrapped(|ui| {
        for key in &keys {
            let Some((resp, asset)) = strip_tile(ui, state, key) else {
                continue;
            };
            resp.dnd_set_drag_payload(FavDrag {
                group: group.id,
                key: (*key).clone(),
            });
            if let Some(d) = resp.dnd_hover_payload::<FavDrag>()
                && d.group == group.id
                && d.key != **key
            {
                let x = resp.rect.left() - 2.0;
                ui.painter()
                    .vline(x, resp.rect.y_range(), Stroke::new(2.0, SELECTED));
            }
            if let Some(d) = resp.dnd_release_payload::<FavDrag>()
                && d.group == group.id
            {
                state.favorite_move(group.id, &d.key, &Drop::Before((*key).clone()));
            }
            if let Some(t) = resp.dnd_release_payload::<TileDrag>() {
                state.favorite_add(group.id, &t.0);
            }
            let ev = events(ui, state, resp, &asset, None, None);
            handle(state, key, ev);
        }
        let (end, end_resp) = ui.allocate_exact_size(Vec2::new(STRIP, STRIP), Sense::hover());
        let hovering = end_resp.dnd_hover_payload::<FavDrag>().is_some()
            || end_resp.dnd_hover_payload::<TileDrag>().is_some();
        ui.painter().rect_stroke(
            end,
            CornerRadius::same(3),
            Stroke::new(
                1.0,
                if hovering {
                    SELECTED
                } else {
                    Color32::from_gray(70)
                },
            ),
            StrokeKind::Inside,
        );
        ui.painter().text(
            end.center(),
            Align2::CENTER_CENTER,
            "+",
            FontId::proportional(16.0),
            Color32::from_gray(120),
        );
        if let Some(d) = end_resp.dnd_release_payload::<FavDrag>()
            && d.group == group.id
        {
            state.favorite_move(group.id, &d.key, &Drop::End);
        }
        if let Some(t) = end_resp.dnd_release_payload::<TileDrag>() {
            state.favorite_add(group.id, &t.0);
        }
    });
    // Dropping a tile anywhere on the strip adds it too.
    if let Some(t) = strip.response.dnd_release_payload::<TileDrag>() {
        state.favorite_add(group.id, &t.0);
    }
    ui.add_space(4.0);
}

// ───────────────────────── results ─────────────────────────

fn results_grid(ui: &mut Ui, state: &mut AppState, panel: Rect) {
    let n = state.browser.results.hits.len();
    if n == 0 {
        if !state.browser.live.busy() && state.browser.results.failed.is_none() {
            ui.weak("No results.");
        }
        return;
    }
    let size = state.config.browser_thumb.clamp(THUMB_MIN, THUMB_MAX);
    let selected_index = state.browser.selected.as_ref().and_then(|k| {
        state
            .browser
            .results
            .hits
            .iter()
            .position(|h| &h.asset.key == k)
    });
    let scroll_to = std::mem::take(&mut state.browser.scroll_to_selected);
    let spacing = ui.spacing().item_spacing;
    if size < LIST_BELOW {
        let row_h = size.max(18.0) * 0.6 + 6.0;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("browser_list")
            .auto_shrink([false, false]);
        if let (true, Some(i)) = (scroll_to, selected_index) {
            area = area.vertical_scroll_offset(i as f32 * (row_h + spacing.y));
        }
        area.show_rows(ui, row_h, n, |ui, range| {
            let hits = hits_in(state, range);
            for h in hits {
                let ev = list_row(ui, state, &h.asset, h.alias.as_deref(), row_h);
                handle(state, &h.asset.key, ev);
            }
        });
        return;
    }
    let cell = Vec2::new(size + 6.0, size + 22.0);
    let width = ui
        .available_width()
        .min(panel.right() - ui.cursor().left())
        .max(cell.x);
    let cols = (((width + spacing.x) / (cell.x + spacing.x)).floor() as usize).max(1);
    let rows = n.div_ceil(cols);
    let mut area = egui::ScrollArea::vertical()
        .id_salt("browser_grid")
        .auto_shrink([false, false]);
    if let (true, Some(i)) = (scroll_to, selected_index) {
        area = area.vertical_scroll_offset((i / cols) as f32 * (cell.y + spacing.y));
    }
    area.show_rows(ui, cell.y, rows, |ui, range| {
        for row in range {
            let start = row * cols;
            let end = (start + cols).min(n);
            let hits = hits_in(state, start..end);
            ui.horizontal(|ui| {
                for h in hits {
                    let ev = grid_tile(ui, state, &h.asset, h.alias.as_deref(), size);
                    handle(state, &h.asset.key, ev);
                }
            });
        }
    });
}

/// The results in `range` (none when they changed meanwhile).
fn hits_in(state: &AppState, range: std::ops::Range<usize>) -> Vec<capy_index::Hit> {
    state
        .browser
        .results
        .hits
        .get(range)
        .map(<[capy_index::Hit]>::to_vec)
        .unwrap_or_default()
}

/// Enter with the grid focused uses the selection.
fn enter_uses_selection(ui: &Ui, state: &mut AppState) {
    if !state.browser.grid_focus || ui.ctx().text_edit_focused() || state.browser.palette.open {
        return;
    }
    if ui.input(|i| i.key_pressed(Key::Enter))
        && let Some(key) = state.browser.selected.clone()
        && let Err(e) = state.use_asset_key(&key)
    {
        state.error(e);
    }
}

// ───────────────────────── windows ─────────────────────────

/// The tile menu, the text prompt and the "maps that use it" window.
pub fn windows(ctx: &egui::Context, state: &mut AppState) {
    tile_menu(ctx, state);
    prompt(ctx, state);
    used_in_window(ctx, state);
}

fn tile_menu(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut menu) = state.browser.menu.take() else {
        return;
    };
    let id = Id::new("browser_tile_menu");
    let mut action: Option<MenuAction> = None;
    let groups: Vec<(i64, String)> = state
        .browser
        .lists
        .groups
        .iter()
        .map(|(g, _)| (g.id, g.name.clone()))
        .collect();
    let key = menu.key.clone();
    let shown = egui::Popup::new(
        id,
        ctx.clone(),
        menu.pos,
        egui::LayerId::new(egui::Order::Foreground, id.with("layer")),
    )
    .kind(egui::PopupKind::Menu)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .open_bool(&mut menu.open)
    .show(|ui| {
        ui.set_min_width(220.0);
        ui.label(
            RichText::new(key.rsplit('\\').next().unwrap_or(&key))
                .monospace()
                .weak(),
        );
        ui.separator();
        if ui.button("Use as brush / open").clicked() {
            action = Some(MenuAction::Use);
        }
        if ui.button("Set alias...").clicked() {
            action = Some(MenuAction::Prompt(PromptKind::Alias(key.clone())));
        }
        if ui.button("Add tag...").clicked() {
            action = Some(MenuAction::Prompt(PromptKind::AddTag(key.clone())));
        }
        if !menu.tags.is_empty() {
            ui.menu_button("Remove tag", |ui| {
                for t in &menu.tags {
                    if ui.button(t).clicked() {
                        action = Some(MenuAction::RemoveTag(t.clone()));
                    }
                }
            });
        }
        let note_label = if menu.note.is_some() {
            "Edit note..."
        } else {
            "Add note..."
        };
        let note = ui.button(note_label);
        let note = match &menu.note {
            Some(n) => note.on_hover_text(n),
            None => note,
        };
        if note.clicked() {
            action = Some(MenuAction::Prompt(PromptKind::Note(key.clone())));
        }
        ui.separator();
        ui.menu_button("Add to favorites", |ui| {
            for (gid, name) in &groups {
                let inside = menu.in_groups.contains(gid);
                if ui.add_enabled(!inside, egui::Button::new(name)).clicked() {
                    action = Some(MenuAction::AddFav(*gid));
                }
            }
            if ui.button("New group...").clicked() {
                action = Some(MenuAction::Prompt(PromptKind::NewGroup(Some(key.clone()))));
            }
        });
        for (gid, name) in groups.iter().filter(|(g, _)| menu.in_groups.contains(g)) {
            if ui.button(format!("Remove from {name}")).clicked() {
                action = Some(MenuAction::RemoveFav(*gid));
            }
        }
        ui.separator();
        let is_tile = crate::browser::brush::parse_tile_key(&key).is_some();
        if ui
            .add_enabled(is_tile, egui::Button::new("Find similar"))
            .clicked()
        {
            action = Some(MenuAction::Similar);
        }
        if ui
            .add_enabled(is_tile, egui::Button::new("Show maps that use it"))
            .clicked()
        {
            action = Some(MenuAction::UsedIn);
        }
        if ui.button("Copy key").clicked() {
            action = Some(MenuAction::Copy);
        }
        ui.separator();
        let dt1 = key
            .split('#')
            .next()
            .is_some_and(|p| p.to_ascii_lowercase().ends_with(".dt1"));
        if ui
            .add_enabled(dt1, egui::Button::new("Convert tile set to palette..."))
            .on_hover_text("Open this DT1 in Convert to palette (nothing changes until Convert)")
            .clicked()
        {
            action = Some(MenuAction::ConvertPalette);
        }
    });
    if shown.is_none() {
        menu.open = false;
    }
    if action.is_some() {
        menu.open = false;
    }
    if menu.open {
        state.browser.menu = Some(menu);
    }
    let Some(action) = action else { return };
    match action {
        MenuAction::Use => {
            if let Err(e) = state.use_asset_key(&key) {
                state.error(e);
            }
        }
        MenuAction::Prompt(kind) => {
            let text = match &kind {
                PromptKind::Alias(k) => state
                    .browser
                    .assets
                    .get(k)
                    .and_then(|_| current_alias(state, k))
                    .unwrap_or_default(),
                PromptKind::Note(_) => menu_note(state, &key),
                _ => String::new(),
            };
            state.browser.prompt = Some(TextPrompt {
                kind,
                text,
                error: None,
            });
        }
        MenuAction::RemoveTag(t) => state.remove_tag(&key, &t),
        MenuAction::AddFav(g) => state.favorite_add(g, &key),
        MenuAction::RemoveFav(g) => state.favorite_remove(g, &key),
        MenuAction::Similar => state.find_similar(&key),
        MenuAction::UsedIn => state.browser.used_in_window = Some(key.clone()),
        MenuAction::ConvertPalette => state.convert_open_tile_key(&key),
        MenuAction::Copy => {
            ctx.copy_text(key.clone());
            state.notify(format!("Copied {key}"));
        }
    }
}

/// The alias shown with `key` in the current results, if any.
fn current_alias(state: &AppState, key: &str) -> Option<String> {
    state
        .browser
        .results
        .hits
        .iter()
        .find(|h| h.asset.key == key)
        .and_then(|h| h.alias.clone())
}

fn menu_note(state: &AppState, key: &str) -> String {
    let _ = key;
    state
        .browser
        .menu
        .as_ref()
        .and_then(|m| m.note.clone())
        .unwrap_or_default()
}

enum MenuAction {
    Use,
    Prompt(PromptKind),
    RemoveTag(String),
    AddFav(i64),
    RemoveFav(i64),
    Similar,
    UsedIn,
    Copy,
    ConvertPalette,
}

fn prompt(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut p) = state.browser.prompt.take() else {
        return;
    };
    let title = match &p.kind {
        PromptKind::Alias(_) => "Alias (shown and searched instead of the generated name)",
        PromptKind::AddTag(_) => "Add a tag",
        PromptKind::Note(_) => "Note",
        PromptKind::NewGroup(_) => "New favorites group",
        PromptKind::RenameGroup(_) => "Rename the group",
        PromptKind::SaveSearch(_) => "Save the query as a pinned search",
    };
    let mut ok = false;
    let mut cancel = false;
    egui::Modal::new(Id::new("browser_prompt")).show(ctx, |ui| {
        ui.set_min_width(420.0);
        ui.heading(title);
        match &p.kind {
            PromptKind::Alias(k) | PromptKind::AddTag(k) | PromptKind::Note(k) => {
                ui.label(RichText::new(k).monospace().small());
            }
            PromptKind::SaveSearch(q) => {
                ui.label(RichText::new(q).monospace());
            }
            _ => {}
        }
        let edit = ui.add(egui::TextEdit::singleline(&mut p.text).desired_width(f32::INFINITY));
        edit.request_focus();
        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if let Some(e) = &p.error {
            ui.colored_label(ERROR, e);
        }
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                ok = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
        });
    });
    if cancel {
        return;
    }
    if ok {
        match state.answer_prompt(&p) {
            Ok(()) => return,
            Err(e) => p.error = Some(e),
        }
    }
    state.browser.prompt = Some(p);
}

fn used_in_window(ctx: &egui::Context, state: &mut AppState) {
    let Some(key) = state.browser.used_in_window.clone() else {
        return;
    };
    let asset = match state.browser.asset(&key) {
        Some(Some(a)) => Some(a.clone()),
        _ => None,
    };
    let mut open = true;
    let mut to_open: Option<String> = None;
    egui::Window::new("Maps that use it")
        .id(Id::new("browser_used_in"))
        .open(&mut open)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label(RichText::new(&key).monospace().small());
            let Some(asset) = asset else {
                ui.spinner();
                return;
            };
            match state.browser.used_in(asset.id) {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => {
                    ui.colored_label(ERROR, e.clone());
                }
                Some(Ok(maps)) if maps.is_empty() => {
                    ui.weak("No map of this profile uses it.");
                }
                Some(Ok(maps)) => {
                    let maps = maps.clone();
                    ui.weak(format!("{} map(s); click to open", maps.len()));
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (path, cells) in maps {
                                if ui
                                    .selectable_label(false, format!("{path}  ({cells} cells)"))
                                    .clicked()
                                {
                                    to_open = Some(path);
                                }
                            }
                        });
                }
            }
        });
    if !open {
        state.browser.used_in_window = None;
    }
    if let Some(path) = to_open {
        state.open_map(&d2_assets::GamePath::new(&path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_tokens_replace_each_other() {
        assert_eq!(
            with_token("grass type:wall act:1", &CLASS_TOKENS, Some("type:floor")),
            "grass act:1 type:floor"
        );
        assert_eq!(with_token("type:floor", &CLASS_TOKENS, None), "");
        assert_eq!(
            with_token("file:a.dt1 x", &["file:"], Some("file:b.dt1")),
            "x file:b.dt1"
        );
        assert!(has_token("a TYPE:floor", "type:floor"));
    }

    #[test]
    fn short_names_put_the_index_first() {
        assert_eq!(
            short_name("Act1 Town Floor · Floor · grass · 05/02"),
            ("05/02 · Floor · grass".to_owned(), Some("Act1 Town Floor"))
        );
        assert_eq!(
            short_name("Act1 Town Towne1 · DS1"),
            ("Act1 Town Towne1 · DS1".to_owned(), None)
        );
    }
}
