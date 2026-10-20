//! The Viewer panel: controls, the COF layer list and the canvas.

use std::hash::{Hash, Hasher};

use bevy_egui::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::{paint_badge_row, paint_checker, palette_controls, rgba_texture};
use crate::sprites::Focus;
use crate::sprites::viewer::{Background, MAX_ZOOM, Tex};
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let now = ui.input(|i| i.time);
    if state.sprites.viewer.advance(now) || state.sprites.viewer.playing {
        ui.ctx().request_repaint();
    }
    toolbar(ui, state);
    playback(ui, state);
    ui.separator();
    if let Some(e) = state.sprites.viewer.error.clone()
        && state.sprites.viewer.loaded.is_none()
    {
        ui.colored_label(Color32::from_rgb(235, 90, 90), e);
    }
    let has_cof = state.sprites.viewer.cof().is_some();
    if has_cof {
        egui::Panel::left("viewer_cof_layers")
            .resizable(true)
            .default_size(250.0)
            .show(ui, |ui| cof_panel(ui, state));
    }
    egui::CentralPanel::no_frame().show(ui, |ui| canvas(ui, state));
}

fn toolbar(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        let v = &state.sprites.viewer;
        ui.strong(v.title());
        if let Some((_, format)) = v.sprite() {
            ui.weak(format.extension().to_ascii_uppercase());
        }
        if v.dt1().is_some() {
            ui.weak("DT1");
        }
        let can_edit = v.sprite().is_some();
        let can_gif = can_edit || v.cof().is_some();
        let converted = matches!(
            v.source,
            Some(crate::sprites::SpriteSource::Converted { .. })
        );
        let can_convert = v.sprite().is_some() || v.dt1().is_some();
        if ui
            .add_enabled(can_edit || v.dt1().is_some(), egui::Button::new("Open in editor"))
            .on_disabled_hover_text("Open a DC6, DCC or DT1 (a COF layer has its own button)")
            .clicked()
            && let Err(e) = state.edit_viewer_sprite(None)
        {
            state.error(e);
        }
        if ui
            .add_enabled(can_edit || converted, egui::Button::new("Save to game path..."))
            .clicked()
        {
            state.begin_sprite_save(crate::sprites::Target::Viewer);
        }
        if ui
            .add_enabled(can_gif, egui::Button::new("Export GIF..."))
            .on_hover_text("Animated GIF: frame range, directions, speed, scale, background")
            .on_disabled_hover_text("Open a DC6, DCC or COF")
            .clicked()
            && let Err(e) = state.open_gif_dialog(false)
        {
            state.error(e);
        }
        if ui
            .add_enabled(can_convert, egui::Button::new("Convert to palette..."))
            .on_hover_text("Carry the colours into another palette (act, UI, ...); nothing changes until Convert")
            .on_disabled_hover_text("Open a DC6, DCC or DT1")
            .clicked()
        {
            let _ = state.convert_open_viewer();
        }
    });
    paint_badge_row(ui, state, Focus::Viewer);
    ui.horizontal_wrapped(|ui| {
        palette_controls(ui, state, Focus::Viewer);
        ui.separator();
        let v = &mut state.sprites.viewer;
        ui.label("Zoom");
        ui.add(egui::Slider::new(&mut v.zoom, 1..=MAX_ZOOM).suffix("x"));
        ui.separator();
        let mut checker = v.background == Background::Checker;
        if ui.checkbox(&mut checker, "Checker").changed() {
            v.background = if checker {
                Background::Checker
            } else {
                Background::Solid([0, 0, 0])
            };
        }
        if let Background::Solid(rgb) = &mut v.background {
            ui.color_edit_button_srgb(rgb);
        }
        ui.checkbox(&mut v.show_box, "Box");
        ui.checkbox(&mut v.show_anchor, "Anchor");
    });
}

fn playback(ui: &mut Ui, state: &mut AppState) {
    let v = &mut state.sprites.viewer;
    let (dirs, frames) = (v.directions(), v.frames());
    let is_dt1 = v.dt1().is_some();
    ui.horizontal_wrapped(|ui| {
        if is_dt1 {
            ui.label("Tile");
            let mut tile = v.frame;
            if ui
                .add(egui::Slider::new(&mut tile, 0..=frames.saturating_sub(1)))
                .changed()
            {
                v.set_frame(tile);
            }
            ui.weak(format!("of {frames}"));
            let mut jump = None;
            if let Some(d) = v.dt1() {
                let (o, m, s) = d
                    .tiles
                    .get(v.frame)
                    .map_or((0, 0, 0), |t| (t.orientation, t.main, t.sub));
                let (mut o2, mut m2, mut s2) = (o, m, s);
                ui.add(
                    egui::DragValue::new(&mut o2)
                        .prefix("orient. ")
                        .range(0..=19),
                );
                ui.add(egui::DragValue::new(&mut m2).prefix("main ").range(0..=255));
                ui.add(egui::DragValue::new(&mut s2).prefix("sub ").range(0..=255));
                if (o2, m2, s2) != (o, m, s) {
                    jump = d.find(o2, m2, s2, None);
                }
                ui.weak(d.label(v.frame));
            }
            if let Some(i) = jump {
                v.set_frame(i);
            }
            return;
        }
        ui.label("Direction");
        let mut dir = v.dir;
        egui::ComboBox::from_id_salt("viewer_dir")
            .selected_text(if dirs == 0 {
                "-".to_owned()
            } else {
                format!("{dir} / {dirs}")
            })
            .show_ui(ui, |ui| {
                for d in 0..dirs {
                    ui.selectable_value(&mut dir, d, d.to_string());
                }
            });
        if dir != v.dir {
            v.set_dir(dir);
        }
        let label = if v.playing { "Pause" } else { "Play" };
        if ui
            .add_enabled(frames > 1, egui::Button::new(label))
            .on_hover_text("Space")
            .clicked()
        {
            v.playing = !v.playing;
        }
        ui.label("Frame");
        let mut frame = v.frame;
        let max = frames.saturating_sub(1);
        if ui
            .add_enabled(frames > 1, egui::Slider::new(&mut frame, 0..=max))
            .changed()
        {
            v.set_frame(frame);
            v.playing = false;
        }
        ui.weak(format!("of {frames}"));
        ui.add(
            egui::DragValue::new(&mut v.fps)
                .range(0.5..=100.0)
                .speed(0.1)
                .suffix(" fps"),
        )
        .on_hover_text(format!("From {}", v.fps_note));
    });
}

fn cof_panel(ui: &mut Ui, state: &mut AppState) {
    let Some(c) = state.sprites.viewer.cof() else {
        return;
    };
    let (token, mode, class) = (
        c.name.token.clone(),
        c.name.mode.clone(),
        c.name.class.clone(),
    );
    let variants = c.variants.clone();
    let kind = crate::sprites::cof::folder(c.kind);
    ui.strong(format!("COF {token}{mode}{class}"));
    if let Some(p) = &c.path {
        ui.weak(p.as_str());
    }
    ui.weak(format!(
        "{kind}\\{token}, {} directions x {} frames",
        c.directions(),
        c.frames()
    ));
    let mut variant = None;
    ui.horizontal(|ui| {
        ui.label("Mode / class");
        egui::ComboBox::from_id_salt("cof_variant")
            .selected_text(format!("{mode} {class}"))
            .show_ui(ui, |ui| {
                for (m, k) in &variants {
                    let on = *m == mode && *k == class;
                    if ui.selectable_label(on, format!("{m} {k}")).clicked() && !on {
                        variant = Some((m.clone(), k.clone()));
                    }
                }
            });
    });
    ui.separator();
    ui.label("Layers (draw order follows the COF per frame)");
    let mut toggles = Vec::new();
    let mut armors = Vec::new();
    let mut edit = None;
    egui::ScrollArea::vertical()
        .id_salt("cof_layers_scroll")
        .show(ui, |ui| {
            for (i, l) in c.layers.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut vis = l.visible;
                    if ui.checkbox(&mut vis, l.code).changed() {
                        toggles.push((i, vis));
                    }
                    ui.weak(&l.weapon_class);
                    if l.armors.len() > 1 {
                        egui::ComboBox::from_id_salt(("cof_armor", i))
                            .selected_text(&l.armor)
                            .width(60.0)
                            .show_ui(ui, |ui| {
                                for a in &l.armors {
                                    if ui.selectable_label(*a == l.armor, a).clicked()
                                        && *a != l.armor
                                    {
                                        armors.push((i, a.clone()));
                                    }
                                }
                            });
                    } else {
                        ui.weak(&l.armor);
                    }
                    if l.transparent {
                        ui.weak(format!("blend {}", l.draw_effect));
                    }
                    if l.sprite.is_some() && ui.small_button("Edit").clicked() {
                        edit = Some(i);
                    }
                });
                match (&l.problem, &l.path) {
                    (Some(p), _) => {
                        ui.colored_label(Color32::from_rgb(230, 150, 60), p);
                    }
                    (None, Some(path)) => {
                        ui.weak(path.file_name());
                    }
                    _ => {}
                }
            }
        });
    if let Some((m, k)) = variant {
        state.open_cof_variant(&m, &k);
    }
    for (i, vis) in toggles {
        state.set_cof_layer_visible(i, vis);
    }
    for (i, a) in armors {
        state.set_cof_armor(i, &a);
    }
    if let Some(i) = edit
        && let Err(e) = state.edit_viewer_sprite(Some(i))
    {
        state.error(e);
    }
}

/// The texture of the frame on show, built on demand and cached.
fn frame_texture(ui: &Ui, state: &mut AppState) -> Option<Tex> {
    let epoch = state.session_epoch;
    let pal_key = state.sprites.palette.key(epoch);
    let palette = state.sprite_display_palette();
    let v = &mut state.sprites.viewer;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (v.revision, pal_key).hash(&mut h);
    let key = h.finish();
    if v.tex_key != key {
        v.textures.clear();
        v.tex_key = key;
    }
    let (dir, frame) = (v.dir, v.frame);
    if let Some(t) = v.textures.get(&(dir, frame)) {
        return t.clone();
    }
    let made = match v.rgba(dir, frame, &palette) {
        Some(Ok(rgba)) => rgba_texture(
            ui.ctx(),
            &format!("viewer-{dir}-{frame}"),
            rgba.width,
            rgba.height,
            &rgba.pixels,
        )
        .map(|handle| Tex {
            handle,
            offset: (rgba.offset_x, rgba.offset_y),
            size: (rgba.width, rgba.height),
        }),
        Some(Err(e)) => {
            v.error = Some(e);
            None
        }
        None => None,
    };
    // Keep the cache small: a direction's frames at most.
    if v.textures.len() > 512 {
        v.textures.clear();
    }
    v.textures.insert((dir, frame), made.clone());
    made
}

fn canvas(ui: &mut Ui, state: &mut AppState) {
    let size = ui.available_size().max(Vec2::splat(1.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    match state.sprites.viewer.background {
        Background::Checker => paint_checker(&painter, rect, 8.0),
        Background::Solid([r, g, b]) => {
            painter.rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
        }
    }
    if state.sprites.viewer.loaded.is_none() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Pick a sprite in the Sprites list, or Open file...",
            egui::FontId::proportional(18.0),
            Color32::from_gray(200),
        );
        return;
    }
    // Pan and zoom.
    if response.dragged_by(PointerButton::Primary) || response.dragged_by(PointerButton::Middle) {
        state.sprites.viewer.pan += response.drag_delta();
    }
    if response.hovered() {
        // One zoom step per wheel notch (or Ctrl+wheel / pinch).
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let id = egui::Id::new("viewer_scroll_accum");
        let mut acc: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
        acc += scroll + (zoom - 1.0) * 400.0;
        let v = &mut state.sprites.viewer;
        while acc.abs() >= 40.0 {
            v.zoom = if acc > 0.0 {
                (v.zoom + 1).min(MAX_ZOOM)
            } else {
                v.zoom.saturating_sub(1).max(1)
            };
            acc -= 40.0 * acc.signum();
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, acc));
    }
    if response.double_clicked() {
        state.sprites.viewer.pan = Vec2::ZERO;
    }
    let tex = frame_texture(ui, state);
    // Fit a freshly opened sprite: the largest whole zoom that leaves room.
    if state.sprites.viewer.fit
        && let Some((x0, y0, x1, y1)) = state.sprites.viewer.bounds()
    {
        let (bw, bh) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
        let z = (rect.width() / bw).min(rect.height() / bh) * 0.7;
        state.sprites.viewer.zoom = (z.floor() as u32).clamp(1, 8);
        state.sprites.viewer.fit = false;
    }
    let v = &state.sprites.viewer;
    let z = v.zoom as f32;
    let (bx0, by0, bx1, by1) = v.bounds().unwrap_or((0, 0, 0, 0));
    let centre = Vec2::new((bx0 + bx1) as f32 / 2.0, (by0 + by1) as f32 / 2.0);
    let anchor = (rect.center() + v.pan - centre * z).round();
    let to_screen = |x: i32, y: i32| Pos2::new(anchor.x + x as f32 * z, anchor.y + y as f32 * z);
    if let Some(t) = &tex {
        let r = Rect::from_min_max(
            to_screen(t.offset.0, t.offset.1),
            to_screen(t.offset.0 + t.size.0 as i32, t.offset.1 + t.size.1 as i32),
        );
        painter.image(
            t.handle.id(),
            r,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if v.show_box {
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_rgb(240, 200, 60)),
                egui::StrokeKind::Outside,
            );
        }
    }
    if v.show_box && bx1 > bx0 {
        painter.rect_stroke(
            Rect::from_min_max(to_screen(bx0, by0), to_screen(bx1, by1)),
            0.0,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 180, 255, 160)),
            egui::StrokeKind::Outside,
        );
    }
    if v.show_anchor {
        let red = Stroke::new(1.5, Color32::from_rgb(255, 70, 70));
        painter.line_segment(
            [anchor - Vec2::new(8.0, 0.0), anchor + Vec2::new(8.0, 0.0)],
            red,
        );
        painter.line_segment(
            [anchor - Vec2::new(0.0, 8.0), anchor + Vec2::new(0.0, 8.0)],
            red,
        );
    }
    let info = match &tex {
        Some(t) => format!(
            "dir {} frame {}  {}x{} at ({}, {})  box {}x{}",
            v.dir,
            v.frame,
            t.size.0,
            t.size.1,
            t.offset.0,
            t.offset.1,
            bx1 - bx0,
            by1 - by0
        ),
        None => format!("dir {} frame {}  (empty frame)", v.dir, v.frame),
    };
    painter.text(
        rect.left_bottom() + Vec2::new(6.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        info,
        egui::FontId::monospace(12.0),
        Color32::from_gray(230),
    );
}
