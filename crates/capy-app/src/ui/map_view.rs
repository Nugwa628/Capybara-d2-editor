//! The Map panel: document tabs on top, a transparent rectangle below that the
//! Bevy camera renders into. All map input (zoom, pan, hover) is read here, so
//! it respects whatever egui has on top.

use bevy_egui::egui::{self, Color32, Key, PointerButton, Pos2, Sense, Stroke, Ui, Vec2};

use super::map_tools::{self, ViewFrame};
use super::toolbar;
use crate::iso::{self, View, ZOOM_STEPS};
use crate::state::AppState;

/// Arrow-key pan speed in screen pixels per second.
const KEY_PAN_SPEED: f32 = 900.0;
/// Scroll points per wheel notch (egui's default line scroll speed).
const NOTCH: f32 = 40.0;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    tab_strip(ui, state);
    toolbar::show(ui, state);

    let size = ui.available_size().max(Vec2::splat(1.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    state.map_rect = Some(rect);
    let ppp = ui.ctx().pixels_per_point();

    if state.active_doc().is_none() {
        state.hover_cell = None;
        ui.painter_at(rect).text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Pick a map in the Maps panel",
            egui::FontId::proportional(18.0),
            Color32::from_gray(150),
        );
        return;
    }

    let viewport_px = [
        f64::from(rect.width() * ppp),
        f64::from(rect.height() * ppp),
    ];
    let to_offset = |pos: Pos2| -> [f64; 2] {
        let d = pos - rect.center();
        [f64::from(d.x * ppp), f64::from(d.y * ppp)]
    };

    // Fit on first show, or when asked.
    let want_fit = std::mem::take(&mut state.request_fit);
    if let Some(doc) = state.active_doc_mut()
        && (!doc.fitted || want_fit)
    {
        doc.view = View::fit(doc.ds1.width, doc.ds1.height, viewport_px);
        doc.fitted = true;
    }

    // Input.
    let hover_pos = response.hover_pos();
    let typing = ui.ctx().text_edit_focused();
    let (scroll, space, dt) = ui.input(|i| {
        (
            i.smooth_scroll_delta.y,
            i.key_down(Key::Space),
            i.stable_dt.min(0.1),
        )
    });
    let mut notches = 0;
    if hover_pos.is_some() {
        state.scroll_accum += scroll;
        while state.scroll_accum.abs() >= NOTCH {
            notches += state.scroll_accum.signum() as i32;
            state.scroll_accum -= state.scroll_accum.signum() * NOTCH;
        }
    } else {
        state.scroll_accum = 0.0;
    }

    let panning = response.dragged_by(PointerButton::Middle)
        || (space && response.dragged_by(PointerButton::Primary));
    let drag = response.drag_delta();
    let mut key_dir = Vec2::ZERO;
    if !typing {
        ui.input(|i| {
            if i.key_down(Key::ArrowLeft) {
                key_dir.x += 1.0;
            }
            if i.key_down(Key::ArrowRight) {
                key_dir.x -= 1.0;
            }
            if i.key_down(Key::ArrowUp) {
                key_dir.y += 1.0;
            }
            if i.key_down(Key::ArrowDown) {
                key_dir.y -= 1.0;
            }
        });
    }

    if let Some(doc) = state.active_doc_mut() {
        if notches != 0 {
            let cursor = hover_pos.map_or([0.0, 0.0], to_offset);
            for _ in 0..notches.abs() {
                // Wheel up zooms in.
                doc.view.zoom_at(cursor, notches > 0);
            }
        }
        if panning {
            doc.view
                .pan_pixels([f64::from(drag.x * ppp), f64::from(drag.y * ppp)]);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if space && hover_pos.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if key_dir != Vec2::ZERO {
            let step = KEY_PAN_SPEED * dt;
            doc.view
                .pan_pixels([f64::from(key_dir.x * step), f64::from(key_dir.y * step)]);
        }
    }

    // Hovered cell (after the view update, so it matches what is drawn).
    let hover_cell = match (state.active_doc(), hover_pos) {
        (Some(doc), Some(pos)) if !panning => {
            let iso_pt = doc.view.offset_to_iso(to_offset(pos));
            let cell = iso::screen_to_cell(iso_pt);
            let in_map = cell.0 >= 0
                && cell.1 >= 0
                && (cell.0 as u32) < doc.ds1.width
                && (cell.1 as u32) < doc.ds1.height;
            in_map.then_some(cell)
        }
        _ => None,
    };
    state.hover_cell = hover_cell;

    // Editing: the active tool's gestures, then the overlays under the
    // hover outline.
    let frame = state.active_doc().map(|doc| ViewFrame {
        rect,
        ppp,
        view: doc.view,
    });
    if let Some(frame) = frame {
        map_tools::handle_input(ui, state, &frame, &response, panning);
        map_tools::paint_overlays(ui, state, &frame);
        if response.hovered() && !space && !panning {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }

    // Overlay: hovered cell outline and zoom label.
    if let Some(doc) = state.active_doc() {
        let painter = ui.painter_at(rect);
        if let Some((x, y)) = hover_cell {
            let to_screen = |iso_pt: [f64; 2]| -> Pos2 {
                let o = doc.view.iso_to_offset(iso_pt);
                rect.center() + Vec2::new(o[0] as f32 / ppp, o[1] as f32 / ppp)
            };
            let [tx, ty] = iso::cell_top(x, y);
            let pts = vec![
                to_screen([tx, ty]),
                to_screen([tx + iso::TILE_W / 2.0, ty + iso::TILE_H / 2.0]),
                to_screen([tx, ty + iso::TILE_H]),
                to_screen([tx - iso::TILE_W / 2.0, ty + iso::TILE_H / 2.0]),
            ];
            painter.add(egui::Shape::closed_line(
                pts,
                Stroke::new(1.5, Color32::from_rgb(255, 230, 120)),
            ));
        }
        let z = ZOOM_STEPS[doc.view.zoom.min(ZOOM_STEPS.len() - 1)];
        painter.text(
            rect.left_bottom() + Vec2::new(8.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            format!("1:{z}"),
            egui::FontId::monospace(13.0),
            Color32::from_gray(210),
        );
    }
}

/// Open maps as tabs: click to switch, x to close.
fn tab_strip(ui: &mut Ui, state: &mut AppState) {
    if state.docs.is_empty() {
        return;
    }
    let mut activate = None;
    let mut close = None;
    egui::ScrollArea::horizontal()
        .id_salt("map_tabs")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, doc) in state.docs.iter().enumerate() {
                    let selected = state.active == Some(i);
                    let resp = ui
                        .selectable_label(selected, doc.title())
                        .on_hover_text(format!("{}\n{}", doc.path, doc.source.layer_name));
                    if resp.clicked() {
                        activate = Some(i);
                    }
                    if resp.middle_clicked() {
                        close = Some(i);
                    }
                    if ui.small_button("\u{2715}").on_hover_text("Close").clicked() {
                        close = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        });
    if let Some(i) = activate {
        state.active = Some(i);
    }
    if let Some(i) = close {
        state.request_close(i);
    }
}
