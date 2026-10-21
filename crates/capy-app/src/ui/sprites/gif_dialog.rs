//! The Export GIF window shared by the Viewer, the Converter and the Editor.
//! Logic (ranges, directions, encoding) is in `crate::sprites::gif_export`.

use std::hash::{Hash, Hasher};

use bevy_egui::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use capy_sprite::UnitKind;

use super::{paint_checker, rgba_texture};
use crate::sprites::dialog::Purpose;
use crate::sprites::gif_export::{
    DirChoice, GifDialog, OutputMode, ThumbTex, parse_range, range_label, selected_dirs, source_of,
    sync_sequence,
};
use crate::state::AppState;

const THUMB: f32 = 56.0;

pub fn window(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut dlg) = state.sprites.gif.take() else {
        return;
    };
    {
        let overlay = state.sprite_overlay();
        let names: &[d2_assets::GamePath] = state.session.as_ref().map_or(&[], |s| &s.sprites);
        sync_sequence(&mut dlg, &state.sprites.viewer, overlay.as_deref(), names);
    }
    let palette = state.sprite_palette();
    let epoch = state.session_epoch;
    let pal_key = state.sprites.palette.key(epoch);
    let mut open = dlg.open;
    let mut export = false;
    egui::Window::new("Export GIF")
        .id(Id::new("gif_export_window"))
        .open(&mut open)
        .default_size([620.0, 640.0])
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                body(ui, state, &mut dlg, &palette, pal_key, &mut export);
            });
        });
    dlg.open = open;
    if dlg.open {
        state.sprites.gif = Some(dlg);
        if export {
            state.sprite_dialog(Purpose::ExportGif);
        }
    }
}

fn body(
    ui: &mut Ui,
    state: &AppState,
    dlg: &mut GifDialog,
    palette: &d2_formats::palette::Palette,
    pal_key: u64,
    export: &mut bool,
) {
    let viewer = &state.sprites.viewer;
    let Some((dirs, frames)) = source_of(viewer, dlg).map(|s| (s.directions(), s.frames())) else {
        ui.colored_label(Color32::from_rgb(235, 90, 90), "The sprite is gone.");
        return;
    };
    if frames == 0 || dirs == 0 {
        ui.label("The sprite has no frames.");
        return;
    }
    let is_cof = !dlg.from_editor && viewer.cof().is_some();
    ui.horizontal(|ui| {
        ui.strong(&dlg.stem);
        ui.weak(format!("{dirs} directions x {frames} frames"));
    });
    if dlg.settings.ticked.len() != dirs {
        dlg.settings.ticked.resize(dirs, false);
    }
    let (s0, e0) = dlg.settings.range(frames);
    dlg.settings.start = s0;
    dlg.settings.end = e0;
    let chosen = selected_dirs(&dlg.settings, dlg.current_dir, dirs);
    let thumb_dir = chosen.first().copied().unwrap_or(0);

    // Rebuild the frame cache when what it shows changes.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        viewer.revision,
        pal_key,
        thumb_dir,
        dlg.settings.part,
        dlg.from_editor,
        dlg.settings.seqnum,
        dlg.seq_plan.as_ref().map(|p| p.seqnum),
    )
        .hash(&mut h);
    let key = h.finish();
    if dlg.thumb_key != key {
        dlg.thumb_key = key;
        let built = source_of(viewer, dlg).map_or_else(
            || Ok(Vec::new()),
            |s| s.frames_of(thumb_dir, (0, frames - 1), palette),
        );
        dlg.thumbs = match built {
            Ok(list) => list
                .iter()
                .enumerate()
                .map(|(i, img)| {
                    let rgba = capy_sprite::to_rgba(img, palette);
                    rgba_texture(
                        ui.ctx(),
                        &format!("gif-thumb-{i}"),
                        rgba.width,
                        rgba.height,
                        &rgba.pixels,
                    )
                    .map(|handle| ThumbTex {
                        handle,
                        offset: (rgba.offset_x, rgba.offset_y),
                        size: (rgba.width, rgba.height),
                    })
                })
                .collect(),
            Err(e) => {
                dlg.error = Some(e);
                Vec::new()
            }
        };
    }

    if is_cof {
        let layers: Vec<(usize, String)> = viewer
            .cof()
            .map(|c| {
                c.layers
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.sprite.is_some())
                    .map(|(i, l)| (i, format!("{} ({})", l.code, l.armor)))
                    .collect()
            })
            .unwrap_or_default();
        ui.add_enabled_ui(dlg.settings.seqnum.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Part");
                let text = match dlg.settings.part {
                    None => "Composed (all visible layers, blend modes)".to_owned(),
                    Some(i) => layers
                        .iter()
                        .find(|(j, _)| *j == i)
                        .map_or_else(|| "?".to_owned(), |(_, t)| t.clone()),
                };
                egui::ComboBox::from_id_salt("gif_part")
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut dlg.settings.part,
                            None,
                            "Composed (all visible layers, blend modes)",
                        );
                        for (i, t) in &layers {
                            ui.selectable_value(&mut dlg.settings.part, Some(*i), t);
                        }
                    });
            });
        });
        if viewer.cof().is_some_and(|c| c.kind == UnitKind::Character) {
            sequence_choice(ui, dlg);
        }
    }

    ui.separator();
    ui.strong("Frames");
    ui.weak("Click a thumbnail to set the first frame, Shift+click to set the last.");
    let mut changed_range = false;
    egui::ScrollArea::horizontal()
        .id_salt("gif_thumbs")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for i in 0..frames {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(THUMB), Sense::click());
                    let inside = i >= dlg.settings.start && i <= dlg.settings.end;
                    let p = ui.painter_at(rect);
                    p.rect_filled(
                        rect,
                        2.0,
                        if inside {
                            Color32::from_rgb(40, 70, 110)
                        } else {
                            Color32::from_gray(30)
                        },
                    );
                    if let Some(Some(t)) = dlg.thumbs.get(i) {
                        let (w, h) = (t.size.0.max(1) as f32, t.size.1.max(1) as f32);
                        let k = ((THUMB - 4.0) / w).min((THUMB - 4.0) / h).min(1.0);
                        let r = Rect::from_center_size(rect.center(), Vec2::new(w * k, h * k));
                        p.image(
                            t.handle.id(),
                            r,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            if inside {
                                Color32::WHITE
                            } else {
                                Color32::from_gray(120)
                            },
                        );
                    }
                    p.text(
                        rect.left_top() + Vec2::new(3.0, 1.0),
                        egui::Align2::LEFT_TOP,
                        (i + 1).to_string(),
                        egui::FontId::proportional(10.0),
                        Color32::from_gray(210),
                    );
                    let edge = if i == dlg.settings.start || i == dlg.settings.end {
                        Stroke::new(2.0, Color32::from_rgb(240, 200, 60))
                    } else {
                        Stroke::new(1.0, Color32::from_gray(70))
                    };
                    p.rect_stroke(rect, 2.0, edge, egui::StrokeKind::Inside);
                    if resp.clicked() {
                        let shift = ui.input(|inp| inp.modifiers.shift);
                        if shift {
                            dlg.settings.end = i;
                            if dlg.settings.start > i {
                                dlg.settings.start = i;
                            }
                        } else {
                            dlg.settings.start = i;
                            if dlg.settings.end < i {
                                dlg.settings.end = i;
                            }
                        }
                        changed_range = true;
                    }
                }
            });
        });
    ui.horizontal(|ui| {
        ui.label("Range");
        let r = ui.add(egui::TextEdit::singleline(&mut dlg.range_text).desired_width(70.0));
        if r.changed()
            && let Some((a, b)) = parse_range(&dlg.range_text, frames)
        {
            dlg.settings.start = a;
            dlg.settings.end = b;
        }
        if ui.button("All").clicked() {
            dlg.settings.start = 0;
            dlg.settings.end = frames - 1;
            changed_range = true;
        }
        ui.weak(format!(
            "of 1-{frames} ({} frames)",
            dlg.settings.end - dlg.settings.start + 1
        ));
    });
    if changed_range {
        dlg.range_text = range_label(&dlg.settings);
    }

    ui.separator();
    ui.strong("Directions");
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(
            &mut dlg.settings.dirs,
            DirChoice::Current,
            format!("Current ({})", dlg.current_dir),
        );
        ui.radio_value(&mut dlg.settings.dirs, DirChoice::All, "All");
        ui.radio_value(&mut dlg.settings.dirs, DirChoice::Ticked, "Ticked");
    });
    if dlg.settings.dirs == DirChoice::Ticked {
        ui.horizontal_wrapped(|ui| {
            for d in 0..dirs {
                ui.checkbox(&mut dlg.settings.ticked[d], d.to_string());
            }
        });
    }
    if chosen.len() > 1 {
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut dlg.settings.output,
                OutputMode::PerDirection,
                "One GIF per direction (name_dNN.gif)",
            );
            ui.radio_value(
                &mut dlg.settings.output,
                OutputMode::Sequence,
                "One GIF, directions in sequence",
            );
        });
    }

    ui.separator();
    ui.strong("Playback and look");
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::DragValue::new(&mut dlg.settings.fps)
                .range(0.5..=100.0)
                .speed(0.1)
                .suffix(" fps"),
        )
        .on_hover_text(format!("The Viewer's speed: {}", viewer.fps_note));
        if ui.small_button("Viewer speed").clicked() {
            dlg.settings.fps = viewer.fps;
        }
        ui.checkbox(&mut dlg.settings.looping, "Loop");
        ui.label("Scale");
        ui.add(egui::Slider::new(&mut dlg.settings.scale, 1..=4).suffix("x"));
    });
    ui.horizontal(|ui| {
        let mut solid = dlg.settings.solid.is_some();
        if ui.checkbox(&mut solid, "Solid background").changed() {
            dlg.settings.solid = solid.then_some([0, 0, 0]);
        }
        if let Some(rgb) = &mut dlg.settings.solid {
            ui.color_edit_button_srgb(rgb);
        } else {
            ui.weak("transparent");
        }
    });

    ui.separator();
    ui.strong("Preview");
    preview(ui, dlg);

    ui.separator();
    if let Some(e) = &dlg.error {
        ui.colored_label(Color32::from_rgb(235, 90, 90), e);
    }
    let files = if dlg.settings.output == OutputMode::Sequence {
        1
    } else {
        chosen.len()
    };
    let n = dlg.settings.end - dlg.settings.start + 1;
    ui.horizontal(|ui| {
        let busy = state.sprites.dialogs.is_open();
        if ui
            .add_enabled(
                !chosen.is_empty() && !busy && dlg.sequence_problem().is_none(),
                egui::Button::new("Export..."),
            )
            .clicked()
        {
            dlg.error = None;
            *export = true;
        }
        let per = if dlg.settings.output == OutputMode::Sequence {
            n * chosen.len()
        } else {
            n
        };
        ui.weak(format!(
            "{files} file(s), {per} frames each, {} cs per frame, {}x",
            dlg.settings.options().delay_cs,
            dlg.settings.options().scale
        ));
    });
}

/// The "Skill sequence" list of a character COF: None, or a skill whose
/// sequence replaces the COF's own frame order. Problems are shown in orange.
fn sequence_choice(ui: &mut Ui, dlg: &mut GifDialog) {
    let choices = dlg.seq_choices.clone().unwrap_or_default();
    let before = dlg.settings.seqnum;
    ui.horizontal(|ui| {
        ui.label("Skill sequence");
        let text = match before {
            None => "None (the COF's frame order)".to_owned(),
            Some(n) => choices
                .iter()
                .find(|c| c.seqnum == n)
                .map_or_else(|| format!("seq {n}"), |c| c.label.clone()),
        };
        egui::ComboBox::from_id_salt("gif_skill_seq")
            .selected_text(text)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut dlg.settings.seqnum,
                    None,
                    "None (the COF's frame order)",
                );
                for c in &choices {
                    ui.selectable_value(&mut dlg.settings.seqnum, Some(c.seqnum), &c.label);
                }
            })
            .response
            .on_hover_text(
                "Skills with anim SQ have no COF of their own: the game plays these frames of \
                 the character's attack/cast COFs in a fixed order.",
            );
    });
    if dlg.settings.seqnum != before {
        ui.ctx().request_repaint();
    }
    let orange = Color32::from_rgb(235, 150, 50);
    if let Some(p) = &dlg.seq_plan {
        for line in &p.problems {
            ui.colored_label(orange, line);
        }
        for line in &p.notes {
            ui.weak(line);
        }
    }
}

/// Plays the chosen range at the chosen speed on the chosen background.
fn preview(ui: &mut Ui, dlg: &mut GifDialog) {
    let (a, b) = (dlg.settings.start, dlg.settings.end);
    let list: Vec<&ThumbTex> = (a..=b)
        .filter_map(|i| dlg.thumbs.get(i).and_then(Option::as_ref))
        .collect();
    let n = b - a + 1;
    // Advance the clock.
    let now = ui.input(|i| i.time);
    let last = dlg.preview_clock.replace(now);
    if n > 1 {
        if let Some(last) = last {
            dlg.preview_accum += (now - last).clamp(0.0, 1.0);
        }
        let step = f64::from(dlg.settings.options().delay_cs) / 100.0;
        let mut guard = 0;
        while dlg.preview_accum >= step && guard < 1000 {
            dlg.preview_accum -= step;
            dlg.preview_frame += 1;
            guard += 1;
        }
        if !dlg.settings.looping && dlg.preview_frame >= n {
            dlg.preview_frame = n - 1;
        }
        ui.ctx().request_repaint();
    }
    let idx = dlg.preview_frame % n.max(1);
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for t in &list {
        x0 = x0.min(t.offset.0);
        y0 = y0.min(t.offset.1);
        x1 = x1.max(t.offset.0 + t.size.0 as i32);
        y1 = y1.max(t.offset.1 + t.size.1 as i32);
    }
    let (bw, bh) = if x1 > x0 && y1 > y0 {
        ((x1 - x0) as f32, (y1 - y0) as f32)
    } else {
        (1.0, 1.0)
    };
    let scale = dlg.settings.scale.clamp(1, 4) as f32;
    let k = scale.min(260.0 / bh).min(560.0 / bw).max(0.25);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new((bw * k).max(60.0), (bh * k).max(40.0)),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    match dlg.settings.solid {
        Some([r, g, bl]) => {
            painter.rect_filled(rect, 0.0, Color32::from_rgb(r, g, bl));
        }
        None => paint_checker(&painter, rect, 8.0),
    }
    if let Some(t) = a_frame(dlg, a + idx) {
        let min = rect.min + Vec2::new((t.offset.0 - x0) as f32 * k, (t.offset.1 - y0) as f32 * k);
        let r = Rect::from_min_size(min, Vec2::new(t.size.0 as f32 * k, t.size.1 as f32 * k));
        painter.image(
            t.handle.id(),
            r,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    ui.weak(format!(
        "frame {} of {} (source frame {})",
        idx + 1,
        n,
        a + idx + 1
    ));
}

fn a_frame(dlg: &GifDialog, i: usize) -> Option<&ThumbTex> {
    dlg.thumbs.get(i).and_then(Option::as_ref)
}
