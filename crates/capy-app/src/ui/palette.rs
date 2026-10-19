//! Ctrl+K: the global asset search, a centred command palette over every
//! asset of the profile.
//!
//! | Keys | Action |
//! |---|---|
//! | Up / Down, PgUp / PgDn | move the selection (1 / 10 rows) |
//! | Enter | use the asset: a tile becomes the brush, a sprite opens in Sprites mode, a map opens |
//! | Shift+Enter | show it in the Tiles browser |
//! | Esc (or a click outside) | close |
//!
//! With an empty query it lists the recently used tiles.

use bevy_egui::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Rect, RichText, Sense,
    Stroke, StrokeKind, Vec2,
};
use capy_index::AssetRow;

use crate::index_task::IndexState;
use crate::state::AppState;

const ROW: f32 = 44.0;
const PAGE: usize = 10;

#[derive(Default)]
struct Keys {
    delta: i64,
    enter: bool,
    shift_enter: bool,
    escape: bool,
}

fn read_keys(ctx: &egui::Context) -> Keys {
    ctx.input_mut(|i| {
        let mut k = Keys::default();
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            k.delta += 1;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            k.delta -= 1;
        }
        if i.consume_key(Modifiers::NONE, Key::PageDown) {
            k.delta += PAGE as i64;
        }
        if i.consume_key(Modifiers::NONE, Key::PageUp) {
            k.delta -= PAGE as i64;
        }
        k.shift_enter = i.consume_key(Modifiers::SHIFT, Key::Enter);
        k.enter = i.consume_key(Modifiers::NONE, Key::Enter);
        k.escape = i.consume_key(Modifiers::NONE, Key::Escape);
        k
    })
}

/// The rows on show: search hits, or the recents for an empty query.
fn rows(state: &mut AppState) -> Vec<(AssetRow, Option<String>)> {
    if state.browser.palette.query.trim().is_empty() {
        let keys = state.browser.lists.recents.clone();
        keys.iter()
            .filter_map(|k| match state.browser.asset(k) {
                Some(Some(a)) => Some((a.clone(), None)),
                _ => None,
            })
            .collect()
    } else {
        state
            .browser
            .palette
            .results
            .hits
            .iter()
            .map(|h| (h.asset.clone(), h.alias.clone()))
            .collect()
    }
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if !state.browser.palette.open {
        return;
    }
    let keys = read_keys(ctx);
    let items = rows(state);
    let n = items.len();
    let moved = keys.delta != 0;
    {
        let p = &mut state.browser.palette;
        let cur = i64::try_from(p.selected).unwrap_or(0);
        let max = i64::try_from(n.saturating_sub(1)).unwrap_or(0);
        p.selected = usize::try_from((cur + keys.delta).clamp(0, max)).unwrap_or(0);
    }
    let selected = state.browser.palette.selected;
    let mut use_row: Option<usize> = None;
    let mut show_row: Option<usize> = None;
    if keys.enter && n > 0 {
        use_row = Some(selected);
    }
    if keys.shift_enter && n > 0 {
        show_row = Some(selected);
    }

    let screen = ctx.content_rect();
    let width = (screen.width() - 80.0).clamp(320.0, 720.0);
    let modal = egui::Modal::new(Id::new("ctrl_k_palette")).show(ctx, |ui| {
        ui.set_width(width);
        let focus = std::mem::take(&mut state.browser.palette.focus);
        let edit = ui.add(
            egui::TextEdit::singleline(&mut state.browser.palette.query)
                .id(Id::new("ctrl_k_query"))
                .font(egui::TextStyle::Heading)
                .hint_text("Search every asset...")
                .desired_width(f32::INFINITY),
        );
        if focus || !edit.has_focus() {
            edit.request_focus();
        }
        if edit.changed() {
            state.browser.palette.selected = 0;
        }
        ui.label(
            RichText::new(
                "type:floor|wall|roof|shadow|ds1|dt1|dc6|dcc|cof  act:1  color:green  tag:x  \
                 idx:0:5/2  file:  similar:<key>  used-in:  mirrored:yes  src:mod  -tag:x",
            )
            .small()
            .weak(),
        );
        if let Some(err) = &state.browser.palette.results.parse_error {
            let q = state.browser.palette.query.clone();
            super::tiles::parse_error(ui, &q, err.position, &err.detail.clone());
        }
        let status = if !matches!(state.index.state, IndexState::Ready(_)) {
            "The asset index is not ready yet.".to_owned()
        } else if state.browser.palette.query.trim().is_empty() {
            format!("Recently used ({n})")
        } else if let Some(e) = &state.browser.palette.results.failed {
            format!("Search failed: {e}")
        } else {
            let busy = if state.browser.palette.live.busy() {
                " ..."
            } else {
                ""
            };
            format!(
                "{n} result(s) in {} ms{busy}",
                state.browser.palette.results.millis
            )
        };
        ui.weak(status);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("ctrl_k_rows")
            .max_height((screen.height() * 0.6).max(200.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, (asset, alias)) in items.iter().enumerate() {
                    let resp = row(ui, state, asset, alias.as_deref(), i == selected);
                    if i == selected && moved {
                        resp.scroll_to_me(None);
                    }
                    if resp.clicked() {
                        state.browser.palette.selected = i;
                    }
                    if resp.double_clicked() {
                        use_row = Some(i);
                    }
                }
            });
        ui.separator();
        ui.label(
            RichText::new(
                "Up/Down, PgUp/PgDn: move   Enter: use as brush (sprites and maps open)   \
                 Shift+Enter: show in the Tiles browser   Esc: close",
            )
            .small()
            .weak(),
        );
    });
    let close = keys.escape || (modal.should_close() && use_row.is_none());
    if let Some(i) = use_row
        && let Some((asset, _)) = items.get(i)
    {
        state.browser.palette.open = false;
        if let Err(e) = state.use_asset_key(&asset.key) {
            state.error(e);
        }
    } else if let Some(i) = show_row
        && let Some((asset, _)) = items.get(i)
    {
        state.browser.palette.open = false;
        state.show_in_browser(asset);
    } else if close {
        state.browser.palette.open = false;
    }
}

fn row(
    ui: &mut egui::Ui,
    state: &mut AppState,
    asset: &AssetRow,
    alias: Option<&str>,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(90, 140, 220, 70));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, Color32::from_gray(45));
    }
    let thumb = Rect::from_min_size(rect.min + Vec2::splat(2.0), Vec2::splat(ROW - 4.0));
    ui.painter().rect_filled(thumb, 3.0, Color32::from_gray(30));
    match state.browser.thumb(asset.id) {
        Some(t) => {
            let ts = t.size_vec2();
            let scale = (thumb.width() / ts.x).min(thumb.height() / ts.y);
            ui.painter().image(
                t.id(),
                Rect::from_center_size(thumb.center(), ts * scale),
                Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter().text(
                thumb.center(),
                Align2::CENTER_CENTER,
                asset.kind.as_str().to_ascii_uppercase(),
                FontId::monospace(10.0),
                Color32::from_gray(150),
            );
        }
    }
    let x = thumb.right() + 8.0;
    let title = alias.map_or_else(|| asset.gen_name.clone(), str::to_owned);
    ui.painter().text(
        egui::pos2(x, rect.top() + 12.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.0),
        if alias.is_some() {
            Color32::from_rgb(150, 210, 255)
        } else {
            Color32::from_gray(225)
        },
    );
    ui.painter().text(
        egui::pos2(x, rect.top() + 31.0),
        Align2::LEFT_CENTER,
        &asset.key,
        FontId::monospace(10.5),
        Color32::from_gray(140),
    );
    let badge = asset.kind.as_str().to_ascii_uppercase();
    let badge_pos = egui::pos2(rect.right() - 6.0, rect.top() + 12.0);
    ui.painter().text(
        badge_pos,
        Align2::RIGHT_CENTER,
        badge,
        FontId::monospace(10.0),
        Color32::from_gray(150),
    );
    if state.outside_tile_set(asset) {
        ui.painter().text(
            egui::pos2(rect.right() - 6.0, rect.top() + 31.0),
            Align2::RIGHT_CENTER,
            "not in this map's tile set",
            FontId::proportional(10.5),
            Color32::from_rgb(235, 150, 40),
        );
    }
    if selected {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, Color32::from_rgb(120, 170, 240)),
            StrokeKind::Inside,
        );
    }
    resp
}
