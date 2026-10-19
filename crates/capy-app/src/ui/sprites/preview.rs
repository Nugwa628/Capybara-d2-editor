//! The map editor's Sprite preview box: the one selected object, animated.

use bevy_egui::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use capy_render::objects::frame_index;

use super::{paint_checker, rgba_texture};
use crate::config::AppMode;
use capy_render::file_direction;

use crate::sprites::preview::{Built, PreviewKey, build_frames, cof_path};
use crate::sprites::viewer::Tex;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let Some(doc) = state.active_doc() else {
        ui.weak("Open a map and select one object with the Objects tool (O).");
        return;
    };
    let [index] = doc.object_sel.as_slice() else {
        ui.weak(match doc.object_sel.len() {
            0 => "Select one object with the Objects tool (O) to preview its sprite.".to_owned(),
            n => format!("{n} objects are selected; select just one to preview it."),
        });
        return;
    };
    let Some(object) = doc.ds1.objects.get(*index).cloned() else {
        return;
    };
    let (doc_id, act, version, index) = (doc.id, doc.ds1.effective_act(), doc.ds1.version, *index);
    let Some(session) = state.session.as_ref() else {
        ui.weak("No profile is open.");
        return;
    };
    let (overlay, game_version) = (session.overlay.clone(), session.profile.game_version);
    let epoch = state.session_epoch;
    let catalog = match state.sprites.preview.catalog(epoch, &overlay, game_version) {
        Ok(Some(c)) => c,
        Ok(None) => {
            ui.weak("Loading the object tables...");
            ui.ctx().request_repaint();
            return;
        }
        Err(e) => {
            ui.colored_label(
                Color32::from_rgb(235, 90, 90),
                format!("Object tables: {e}"),
            );
            return;
        }
    };
    let info = catalog.resolve(act, version, &object);
    // A new object starts at its own default facing.
    let same_object = state.sprites.preview.key.as_ref().is_some_and(|k| {
        (k.epoch, k.doc, k.index, k.kind, k.id) == (epoch, doc_id, index, object.kind, object.id)
    });
    if !same_object {
        state.sprites.preview.direction = info.direction.unwrap_or(0);
    }
    let gamma = state.config.gamma;
    let palette = state
        .sprites
        .palette
        .act(act, Some(&overlay), epoch)
        .with_gamma(gamma);
    let key = PreviewKey {
        epoch,
        doc: doc_id,
        index,
        kind: object.kind,
        id: object.id,
        act,
        direction: state.sprites.preview.direction,
        palette: u64::from(gamma.to_bits()),
    };
    if state.sprites.preview.key.as_ref() != Some(&key) {
        let built = match build_frames(&overlay, &catalog, &info, key.direction, &palette) {
            Ok((directions, frames, delta)) => Built {
                info: info.clone(),
                cof_path: cof_path(&info),
                directions,
                frames: frames
                    .into_iter()
                    .enumerate()
                    .map(|(f, raw)| {
                        let (w, h, rgba, offset) = raw?;
                        rgba_texture(ui.ctx(), &format!("preview-{f}"), w, h, &rgba).map(|handle| {
                            Tex {
                                handle,
                                offset,
                                size: (w, h),
                            }
                        })
                    })
                    .collect(),
                delta: delta.filter(|d| *d != 0).unwrap_or(256),
                error: None,
            },
            Err(e) => Built {
                info: info.clone(),
                cof_path: cof_path(&info),
                directions: 0,
                frames: Vec::new(),
                delta: 256,
                error: Some(e),
            },
        };
        state.sprites.preview.built = Some(built);
        state.sprites.preview.key = Some(key);
    }
    let Some(built) = state.sprites.preview.built.take() else {
        return;
    };
    let i = &built.info;
    ui.strong(&i.name);
    ui.weak(format!(
        "type {} id {}  token {}  mode {}  class {}",
        object.kind,
        object.id,
        i.token.as_deref().unwrap_or("-"),
        i.mode,
        i.weapon_class
    ));
    let mut open = false;
    let dirs = built.directions;
    let error = built.error.clone();
    let cof = built.cof_path.clone();
    let skins = i.skins.clone();
    ui.horizontal(|ui| {
        let p = &mut state.sprites.preview;
        ui.label("Direction");
        egui::ComboBox::from_id_salt("preview_dir")
            .selected_text(p.direction.to_string())
            .show_ui(ui, |ui| {
                for d in 0..dirs.max(1) {
                    ui.selectable_value(&mut p.direction, d, d.to_string());
                }
            });
        let label = if p.playing { "Pause" } else { "Play" };
        if ui.button(label).clicked() {
            p.playing = !p.playing;
        }
        if ui
            .add_enabled(cof.is_some(), egui::Button::new("Open in Sprites mode"))
            .clicked()
        {
            open = true;
        }
    });
    if let Some(e) = &error {
        ui.colored_label(Color32::from_rgb(230, 150, 60), e);
    }
    // The animation.
    let p = &state.sprites.preview;
    let count = built.frames.len() as u32;
    let tick = (ui.input(|i| i.time) * 25.0) as u32;
    let frame = if p.playing {
        usize::from(frame_index(Some(tick), built.delta, count))
    } else {
        0
    };
    if p.playing && count > 1 {
        ui.ctx().request_repaint();
    }
    let size = ui.available_size().max(Vec2::new(64.0, 64.0));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    paint_checker(&painter, rect, 8.0);
    // A fixed anchor: the union of every frame keeps the object still.
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for t in built.frames.iter().flatten() {
        x0 = x0.min(t.offset.0);
        y0 = y0.min(t.offset.1);
        x1 = x1.max(t.offset.0 + t.size.0 as i32);
        y1 = y1.max(t.offset.1 + t.size.1 as i32);
    }
    if x0 <= x1 {
        let (bw, bh) = ((x1 - x0) as f32, (y1 - y0) as f32);
        let z = ((rect.width() - 8.0) / bw)
            .min((rect.height() - 8.0) / bh)
            .clamp(0.25, 4.0);
        let z = if z >= 1.0 { z.floor() } else { z };
        let centre = Vec2::new((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
        let anchor = (rect.center() - centre * z).round();
        if let Some(Some(t)) = built.frames.get(frame) {
            let min = anchor + Vec2::new(t.offset.0 as f32, t.offset.1 as f32) * z;
            let r = Rect::from_min_size(min, Vec2::new(t.size.0 as f32, t.size.1 as f32) * z);
            painter.image(
                t.handle.id(),
                r,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let red = Stroke::new(1.0, Color32::from_rgb(255, 70, 70));
        painter.line_segment(
            [anchor - Vec2::new(5.0, 0.0), anchor + Vec2::new(5.0, 0.0)],
            red,
        );
        painter.line_segment(
            [anchor - Vec2::new(0.0, 5.0), anchor + Vec2::new(0.0, 5.0)],
            red,
        );
        painter.text(
            rect.left_bottom() + Vec2::new(4.0, -4.0),
            egui::Align2::LEFT_BOTTOM,
            format!("frame {frame} / {count}"),
            egui::FontId::monospace(11.0),
            Color32::from_gray(230),
        );
    }
    state.sprites.preview.built = Some(built);
    if open && let Some(path) = cof {
        let user_dir = state.sprites.preview.direction;
        if state.open_sprite_with(&path, &skins).is_ok() {
            let file_dir = file_direction(dirs, user_dir);
            state.sprites.viewer.set_dir(usize::from(file_dir));
            state.config.mode = AppMode::Sprites;
            state.config_dirty = true;
        }
    }
}
