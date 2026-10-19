//! The "Convert to palette" window: input, source and target palettes,
//! settings and presets, the plan table with overrides, before/after
//! previews, Convert, the converted file's save options, and the batch.

use std::hash::{Hash, Hasher};

use bevy_egui::egui::{self, Color32, Id, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use capy_paint::remap::{MatchKind, RemapPlan};
use d2_assets::GamePath;
use d2_formats::IndexedImage;
use d2_formats::palette::Palette;

use super::{paint_checker, rgba_texture};
use crate::sprites::convert_palette::settings::{
    ADJUST_RANGE, Allowed, Distance, HUE_RANGE, LIGHTNESS_RANGE, delete_preset, save_preset,
};
use crate::sprites::convert_palette::table::{self, Severity, how_label, severity};
use crate::sprites::convert_palette::{PreviewTex, TargetChoice, kind_label};
use crate::sprites::dialog::Purpose;
use crate::sprites::palette::{PaletteChoice, choices};
use crate::sprites::viewer::{MAX_ZOOM, Tex};
use crate::state::AppState;
use d2_formats::remap::SpriteKind;

const WARN: Color32 = Color32::from_rgb(235, 160, 60);
const BAD: Color32 = Color32::from_rgb(235, 80, 80);
const GOOD: Color32 = Color32::from_rgb(110, 200, 110);

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// A colour square; clickable.
fn swatch(ui: &mut Ui, c: [u8; 3], size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter().rect_filled(rect, 2.0, rgb(c));
    ui.painter().rect_stroke(
        rect,
        2.0,
        Stroke::new(
            1.0,
            Color32::from_gray(if resp.hovered() { 230 } else { 90 }),
        ),
        egui::StrokeKind::Inside,
    );
    resp
}

fn sev_color(s: Severity) -> Color32 {
    rgb(s.rgb())
}

pub fn window(ctx: &egui::Context, state: &mut AppState) {
    if !state.sprites.convert.open {
        return;
    }
    let mut open = true;
    egui::Window::new("Convert to palette")
        .id(Id::new("convert_palette_window"))
        .open(&mut open)
        .default_size([1180.0, 760.0])
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| body(ui, state));
    if !open {
        state.sprites.convert.open = false;
        state.sprites.convert.picker = None;
    }
    picker_window(ctx, state);
}

fn body(ui: &mut Ui, state: &mut AppState) {
    input_row(ui, state);
    palettes_row(ui, state);
    ui.separator();
    if state.sprites.convert.input.is_none() {
        ui.weak(
            "Pick a file to convert: the Viewer's sprite or tile set, a DT1 of the profile, \
             or a file on disk. Nothing changes until you press Convert.",
        );
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new("Batch").strong())
            .default_open(true)
            .show(ui, |ui| batch(ui, state));
        return;
    }
    let plan = state.convert_plan();
    if let Some(e) = state.sprites.convert.error.clone() {
        ui.colored_label(BAD, e);
    }
    state.sprites.convert.hovered = state.sprites.convert.script_hover;
    ui.columns(2, |cols| {
        let (left, right) = cols.split_at_mut(1);
        let left = &mut left[0];
        let right = &mut right[0];
        egui::ScrollArea::vertical()
            .id_salt("convert_left")
            .auto_shrink([false, false])
            .show(left, |ui| {
                settings(ui, state);
                ui.separator();
                if let Some(p) = &plan {
                    plan_table(ui, state, p);
                }
            });
        egui::ScrollArea::vertical()
            .id_salt("convert_right")
            .auto_shrink([false, false])
            .show(right, |ui| {
                if let Some(p) = &plan {
                    preview(ui, state, p);
                }
                ui.separator();
                convert_and_result(ui, state, plan.is_some());
                ui.separator();
                egui::CollapsingHeader::new(RichText::new("Batch").strong())
                    .default_open(false)
                    .show(ui, |ui| batch(ui, state));
            });
    });
}

// ───────────────────────── input ─────────────────────────

fn input_row(ui: &mut Ui, state: &mut AppState) {
    let busy = state.sprites.dialogs.is_open();
    ui.horizontal_wrapped(|ui| {
        ui.label("Input");
        match &state.sprites.convert.input {
            Some(i) => {
                ui.strong(i.origin.label());
                ui.weak(format!(
                    "{}, {} {}",
                    kind_label(i.kind),
                    i.items.len(),
                    if i.kind == SpriteKind::Dt1 {
                        "tiles"
                    } else {
                        "frames"
                    }
                ));
            }
            None => {
                ui.weak("(none)");
            }
        }
        ui.separator();
        let can_view =
            state.sprites.viewer.sprite().is_some() || state.sprites.viewer.dt1().is_some();
        if ui
            .add_enabled(can_view, egui::Button::new("From the Viewer"))
            .on_disabled_hover_text("Open a DC6, DCC or DT1 in the Viewer")
            .clicked()
        {
            let _ = state.convert_open_viewer();
        }
        let list = state.sprites.convert.dt1_list;
        if ui
            .add_enabled(
                state.session.is_some(),
                egui::Button::selectable(list, "Pick a DT1..."),
            )
            .clicked()
        {
            state.sprites.convert.dt1_list = !list;
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Open file..."))
            .clicked()
        {
            state.sprites.dialogs.open(Purpose::ConvertOpenFile, None);
        }
    });
    if state.sprites.convert.dt1_list {
        dt1_list(ui, state);
    }
}

fn dt1_list(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.label("DT1");
        ui.add(
            egui::TextEdit::singleline(&mut state.sprites.convert.dt1_filter)
                .hint_text("filter, e.g. act5\\lava")
                .desired_width(320.0),
        );
    });
    let needle = state
        .sprites
        .convert
        .dt1_filter
        .trim()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let found: Vec<GamePath> = state
        .session
        .as_ref()
        .map(|s| {
            s.sprites
                .iter()
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dt1")))
                .filter(|p| p.as_str().contains(&needle))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut pick = None;
    let row = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    egui::ScrollArea::vertical()
        .id_salt("convert_dt1_list")
        .max_height(160.0)
        .show_rows(ui, row, found.len(), |ui, range| {
            for p in &found[range] {
                if ui.selectable_label(false, p.as_str()).clicked() {
                    pick = Some(p.clone());
                }
            }
        });
    ui.weak(format!("{} DT1 file(s)", found.len()));
    if let Some(p) = pick {
        state.sprites.convert.dt1_list = false;
        let _ = state.convert_open_game(&p);
    }
}

fn palette_list(state: &AppState) -> Vec<PaletteChoice> {
    let extra = state
        .session
        .as_ref()
        .map(|s| s.palettes.clone())
        .unwrap_or_default();
    let mut list = choices(&extra);
    if state.session.is_none() {
        list.push(PaletteChoice::Grey);
    }
    list
}

fn palettes_row(ui: &mut Ui, state: &mut AppState) {
    let list = palette_list(state);
    let mut source = None;
    let mut target = None;
    ui.horizontal_wrapped(|ui| {
        let c = &state.sprites.convert;
        ui.label("Source palette");
        egui::ComboBox::from_id_salt("convert_source")
            .selected_text(c.source.label())
            .show_ui(ui, |ui| {
                for p in &list {
                    if ui.selectable_label(*p == c.source, p.label()).clicked() {
                        source = Some(p.clone());
                    }
                }
            })
            .response
            .on_hover_text("The palette the file was made for (remembered per file)");
        ui.label(RichText::new("\u{2192}").strong());
        ui.label("Target palette");
        egui::ComboBox::from_id_salt("convert_target")
            .selected_text(c.target.label())
            .width(220.0)
            .show_ui(ui, |ui| {
                for p in &list {
                    let t = TargetChoice::Palette(p.clone());
                    if ui.selectable_label(t == c.target, p.label()).clicked() {
                        target = Some(t);
                    }
                }
                ui.separator();
                let all = TargetChoice::AllActs;
                if ui
                    .selectable_label(c.target == all, all.label())
                    .on_hover_text(
                        "Act 1's colours without the act-specific indices 225-254: \
                         the result looks the same in every act (UI, items)",
                    )
                    .clicked()
                {
                    target = Some(all);
                }
            });
    });
    if let Some(s) = source {
        state.convert_set_source(s);
    }
    if let Some(t) = target {
        state.sprites.convert.target = t;
    }
    if state.convert_suggests_all_acts() {
        ui.horizontal(|ui| {
            ui.colored_label(
                WARN,
                "UI and item art is drawn in every act: \"All acts (shared colours only)\" keeps it \
                 from changing colour between acts.",
            );
            if ui.small_button("Use All acts").clicked() {
                state.sprites.convert.target = TargetChoice::AllActs;
            }
        });
    }
}

// ───────────────────────── settings ─────────────────────────

fn settings(ui: &mut Ui, state: &mut AppState) {
    egui::CollapsingHeader::new(RichText::new("Settings").strong())
        .default_open(true)
        .show(ui, |ui| {
            let s = &mut state.sprites.convert.settings;
            ui.horizontal(|ui| {
                ui.label("Distance");
                egui::ComboBox::from_id_salt("convert_distance")
                    .selected_text(s.distance.label())
                    .show_ui(ui, |ui| {
                        for d in Distance::ALL {
                            ui.selectable_value(&mut s.distance, d, d.label());
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Allowed targets");
                let is_range = matches!(s.allowed, Allowed::Range { .. });
                egui::ComboBox::from_id_salt("convert_allowed")
                    .selected_text(match s.allowed {
                        Allowed::Range { .. } => "Index range".to_owned(),
                        a => a.label(),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.allowed, Allowed::All, Allowed::All.label());
                        ui.selectable_value(
                            &mut s.allowed,
                            Allowed::Shared,
                            Allowed::Shared.label(),
                        );
                        if ui.selectable_label(is_range, "Index range").clicked() && !is_range {
                            s.allowed = Allowed::Range { from: 1, to: 224 };
                        }
                    });
                if let Allowed::Range { from, to } = &mut s.allowed {
                    ui.add(egui::DragValue::new(from).range(1..=255).prefix("from "));
                    ui.add(egui::DragValue::new(to).range(1..=255).prefix("to "));
                }
            });
            ui.checkbox(&mut s.keep_exact, "Keep exact matches")
                .on_hover_text("An index whose colour is the same in both palettes stays as it is");
            ui.add(egui::Slider::new(&mut s.brightness, ADJUST_RANGE).text("brightness"));
            ui.add(egui::Slider::new(&mut s.contrast, ADJUST_RANGE).text("contrast"));
            ui.add(egui::Slider::new(&mut s.saturation, ADJUST_RANGE).text("saturation"));
            ui.add(
                egui::Slider::new(&mut s.hue, HUE_RANGE)
                    .text("hue")
                    .suffix("\u{b0}"),
            );
            let adjusting = s.brightness != 0.0 || s.contrast != 0.0 || s.saturation != 0.0 || s.hue != 0.0;
            if adjusting && s.keep_exact {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        WARN,
                        "Keep exact matches is on: the sliders only change colours that are re-matched.",
                    );
                    if ui.small_button("Turn off keep exact").clicked() {
                        s.keep_exact = false;
                    }
                });
            }
            egui::CollapsingHeader::new("Advanced")
                .id_salt("convert_advanced")
                .show(ui, |ui| {
                    ui.add_enabled(
                        s.distance == Distance::OkLab,
                        egui::Slider::new(&mut s.lightness_weight, LIGHTNESS_RANGE)
                            .text("lightness weight"),
                    )
                    .on_hover_text("Above 1 favours matching lightness over hue (OkLab only)");
                });
            ui.horizontal(|ui| {
                if ui
                    .button("Reset")
                    .on_hover_text("Defaults (overrides stay; they have Clear all)")
                    .clicked()
                {
                    s.reset_keep_overrides();
                }
            });
            presets(ui, state);
        });
}

fn presets(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Presets");
        let names: Vec<String> = state
            .config
            .palette_presets
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let mut load = None;
        egui::ComboBox::from_id_salt("convert_presets")
            .selected_text(if names.is_empty() {
                "(none)"
            } else {
                "Load..."
            })
            .show_ui(ui, |ui| {
                for n in &names {
                    if ui.selectable_label(false, n).clicked() {
                        load = Some(n.clone());
                    }
                }
            });
        if let Some(n) = load
            && let Err(e) = state.convert_load_preset(&n)
        {
            state.error(e);
        }
        ui.add(
            egui::TextEdit::singleline(&mut state.sprites.convert.preset_name)
                .hint_text("name")
                .desired_width(120.0),
        );
        let name = state.sprites.convert.preset_name.clone();
        if ui
            .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
            .on_hover_text("Settings and overrides, in config.toml")
            .clicked()
        {
            let settings = state.sprites.convert.settings.clone();
            if save_preset(&mut state.config.palette_presets, &name, &settings) {
                state.config_dirty = true;
                state.notify(format!("Saved preset {}", name.trim()));
            }
        }
        let exists = names.iter().any(|n| n.eq_ignore_ascii_case(name.trim()));
        if ui
            .add_enabled(exists, egui::Button::new("Delete"))
            .clicked()
            && delete_preset(&mut state.config.palette_presets, &name)
        {
            state.config_dirty = true;
        }
    });
}

// ───────────────────────── plan table ─────────────────────────

fn plan_table(ui: &mut Ui, state: &mut AppState, plan: &RemapPlan) {
    let shared = state.sprites.convert.target.shared_only();
    let allowed = {
        let s = state.sprites.convert.settings.clone();
        move |i: u8| s.allows(i, shared)
    };
    let rows = table::rows(plan, allowed);
    let summary = table::summary(plan);
    ui.horizontal_wrapped(|ui| {
        ui.strong("Plan");
        ui.colored_label(sev_color(severity(summary.worst)), summary.text());
    });
    ui.horizontal(|ui| {
        let n = state.sprites.convert.settings.overrides.len();
        if ui
            .add_enabled(
                n > 0,
                egui::Button::new(format!("Clear all overrides ({n})")),
            )
            .clicked()
        {
            state.sprites.convert.settings.clear_overrides();
        }
        ui.weak("Click a target colour to override it. Hover a row to see its pixels.");
    });
    let mut open_picker = None;
    let mut clear = None;
    let mut hovered = None;
    egui::Grid::new("convert_plan_grid")
        .striped(true)
        .num_columns(7)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            for h in ["source", "", "target", "error", "pixels", "how", ""] {
                ui.weak(h);
            }
            ui.end_row();
            for r in &rows {
                let e = &r.entry;
                let mut row_hover = false;
                ui.horizontal(|ui| {
                    row_hover |= swatch(ui, e.source_rgb, 16.0)
                        .on_hover_text(super::swatch_text(e.source, e.source_rgb))
                        .hovered();
                    ui.monospace(format!("{:>3}", e.source));
                });
                ui.label(if r.changed { "\u{2192}" } else { "=" });
                ui.horizontal(|ui| {
                    let resp = swatch(ui, e.target_rgb, 16.0).on_hover_text(format!(
                        "{} (click: override)",
                        super::swatch_text(e.target, e.target_rgb)
                    ));
                    row_hover |= resp.hovered();
                    if resp.clicked() {
                        open_picker = Some(e.source);
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{:>3}", e.target)).monospace(),
                            )
                            .small(),
                        )
                        .clicked()
                    {
                        open_picker = Some(e.source);
                    }
                });
                ui.colored_label(
                    sev_color(r.severity),
                    format!("{:.1} {}", e.error, r.severity.label()),
                );
                ui.label(e.pixels.to_string());
                ui.horizontal(|ui| {
                    ui.label(how_label(e.how));
                    if r.outside_allowed {
                        ui.colored_label(WARN, "\u{26a0}").on_hover_text(
                            "This override is outside the allowed colours (it is used anyway)",
                        );
                    }
                });
                if e.how == MatchKind::Override {
                    if ui
                        .small_button("\u{2715}")
                        .on_hover_text("Clear this override")
                        .clicked()
                    {
                        clear = Some(e.source);
                    }
                } else {
                    ui.label("");
                }
                let row_rect = ui.min_rect();
                if row_hover || ui.rect_contains_pointer(row_rect.with_min_y(row_rect.max.y - 20.0))
                {
                    hovered = Some(e.source);
                }
                ui.end_row();
            }
        });
    if let Some(s) = open_picker {
        state.sprites.convert.picker = Some(s);
    }
    if let Some(s) = clear {
        state.sprites.convert.settings.set_override(s, None);
    }
    if hovered.is_some() {
        state.sprites.convert.hovered = hovered;
    }
}

fn picker_window(ctx: &egui::Context, state: &mut AppState) {
    let Some(source) = state.sprites.convert.picker else {
        return;
    };
    let Ok((src, dst)) = state.convert_palettes() else {
        state.sprites.convert.picker = None;
        return;
    };
    let cands = state.convert_candidates(source, 12);
    let current = state.sprites.convert.settings.override_for(source);
    let shared = state.sprites.convert.target.shared_only();
    let settings = state.sprites.convert.settings.clone();
    let mut open = true;
    let mut pick: Option<Option<u8>> = None;
    egui::Window::new(format!("Override for index {source}"))
        .id(Id::new("convert_override_picker"))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_pos([520.0, 160.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                swatch(ui, src.colors[usize::from(source)], 22.0);
                ui.label(super::swatch_text(source, src.colors[usize::from(source)]));
                if let Some(t) = current {
                    ui.separator();
                    ui.label("override:");
                    swatch(ui, dst.colors[usize::from(t)], 22.0);
                    ui.monospace(t.to_string());
                }
            });
            ui.separator();
            ui.strong("Best matches");
            egui::Grid::new("convert_candidates")
                .num_columns(4)
                .spacing([10.0, 2.0])
                .show(ui, |ui| {
                    for (t, err) in &cands {
                        if swatch(ui, dst.colors[usize::from(*t)], 18.0).clicked() {
                            pick = Some(Some(*t));
                        }
                        if ui
                            .button(RichText::new(format!("{t:>3}")).monospace())
                            .clicked()
                        {
                            pick = Some(Some(*t));
                        }
                        ui.colored_label(sev_color(severity(*err)), format!("{err:.1}"));
                        ui.label(severity(*err).label());
                        ui.end_row();
                    }
                });
            ui.separator();
            ui.strong("Target palette");
            ui.weak("Dimmed colours are outside the allowed set (an override may still use them).");
            let cell = 15.0;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(cell * 16.0), Sense::click());
            let painter = ui.painter_at(rect);
            for i in 0..=255u8 {
                let (x, y) = (f32::from(i % 16), f32::from(i / 16));
                let r = Rect::from_min_size(
                    rect.min + Vec2::new(x * cell, y * cell),
                    Vec2::splat(cell),
                );
                let mut c = rgb(dst.colors[usize::from(i)]);
                if !settings.allows(i, shared) {
                    c = c.gamma_multiply(0.3);
                }
                painter.rect_filled(r.shrink(0.5), 0.0, c);
                if Some(i) == current {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(2.0, Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            if let Some(p) = resp.hover_pos() {
                let i = (((p.y - rect.min.y) / cell) as u32).min(15) * 16
                    + (((p.x - rect.min.x) / cell) as u32).min(15);
                let i = i as u8;
                resp.clone()
                    .on_hover_text(super::swatch_text(i, dst.colors[usize::from(i)]));
                if resp.clicked() && i != 0 {
                    pick = Some(Some(i));
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(current.is_some(), egui::Button::new("Clear override"))
                    .clicked()
                {
                    pick = Some(None);
                }
                if ui.button("Close").clicked() {
                    state.sprites.convert.picker = None;
                }
            });
        });
    if let Some(t) = pick {
        state.sprites.convert.settings.set_override(source, t);
        state.sprites.convert.picker = None;
    }
    if !open {
        state.sprites.convert.picker = None;
    }
}

// ───────────────────────── preview ─────────────────────────

/// RGBA of `img` under `palette`, mapped by `table` when given; when a
/// highlight is on, pixels outside it are dimmed.
fn preview_rgba(
    img: &IndexedImage,
    palette: &Palette,
    table: Option<&[u8; 256]>,
    mark: Option<&[bool; 256]>,
) -> Vec<u8> {
    let mut out = vec![0u8; img.pixels.len() * 4];
    for (i, (&p, &a)) in img.pixels.iter().zip(&img.alpha).enumerate() {
        if a == 0 {
            continue;
        }
        let shown = table.map_or(p, |t| t[usize::from(p)]);
        let c = palette.colors[usize::from(shown)];
        let lit = mark.is_none_or(|m| m[usize::from(p)]);
        let px = if lit {
            [c[0], c[1], c[2], 255]
        } else {
            let g = ((u16::from(c[0]) + u16::from(c[1]) + u16::from(c[2])) / 12) as u8;
            [g, g, g, 255]
        };
        out[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    out
}

fn preview_textures(
    ui: &Ui,
    state: &mut AppState,
    plan: &RemapPlan,
) -> Option<(Option<Tex>, Option<Tex>)> {
    let (src, dst) = state.convert_palettes().ok()?;
    let gamma = state.sprites.palette.gamma;
    let (src, dst) = (src.with_gamma(gamma), dst.with_gamma(gamma));
    let c = &state.sprites.convert;
    let input = c.input.as_ref()?;
    let img = input.items.get(c.item)?;
    let mark: Option<[bool; 256]> = match c.hovered {
        Some(h) => {
            let mut m = [false; 256];
            m[usize::from(h)] = true;
            Some(m)
        }
        None if c.highlight_changed || c.highlight_worst => Some(table::highlight_mask(
            plan,
            c.highlight_changed,
            c.highlight_worst,
        )),
        None => None,
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        std::sync::Arc::as_ptr(&input.bytes) as usize,
        c.item,
        plan.table,
        src.colors,
        dst.colors,
        mark,
    )
        .hash(&mut h);
    let key = h.finish();
    if let Some(t) = &c.tex
        && t.key == key
    {
        return Some((t.before.clone(), t.after.clone()));
    }
    let make = |name: &str, rgba: Vec<u8>| {
        rgba_texture(ui.ctx(), name, img.width, img.height, &rgba).map(|handle| Tex {
            handle,
            offset: (img.offset_x, img.offset_y),
            size: (img.width, img.height),
        })
    };
    let before = make(
        "convert-before",
        preview_rgba(img, &src, None, mark.as_ref()),
    );
    // Marks follow source indices: the after image maps the source pixels
    // through the table.
    let after = make(
        "convert-after",
        preview_rgba(img, &dst, Some(&plan.table), mark.as_ref()),
    );
    state.sprites.convert.tex = Some(PreviewTex {
        key,
        before: before.clone(),
        after: after.clone(),
    });
    Some((before, after))
}

fn item_picker(ui: &mut Ui, state: &mut AppState) {
    let c = &mut state.sprites.convert;
    let Some(input) = &c.input else { return };
    let n = input.items.len();
    let mut item = c.item;
    ui.horizontal_wrapped(|ui| {
        if input.kind == SpriteKind::Dt1 {
            ui.label("Tile");
            ui.add(egui::DragValue::new(&mut item).range(0..=n.saturating_sub(1)));
            ui.weak(format!("of {n}"));
            if let Some(t) = input.tiles.get(item) {
                let (mut o, mut m, mut s) = (t.orientation, t.main, t.sub);
                let before = (o, m, s);
                ui.add(
                    egui::DragValue::new(&mut o)
                        .prefix("orient. ")
                        .range(0..=19),
                );
                ui.add(egui::DragValue::new(&mut m).prefix("main ").range(0..=255));
                ui.add(egui::DragValue::new(&mut s).prefix("sub ").range(0..=255));
                if (o, m, s) != before
                    && let Some(i) = input.find_tile(o, m, s, None)
                {
                    item = i;
                }
            }
        } else {
            let frames = input.frames.max(1);
            let (mut d, mut f) = (item / frames, item % frames);
            ui.label("Direction");
            ui.add(egui::DragValue::new(&mut d).range(0..=input.directions.saturating_sub(1)));
            ui.label("Frame");
            ui.add(egui::DragValue::new(&mut f).range(0..=frames - 1));
            item = d * frames + f;
        }
    });
    c.set_item(item);
}

fn preview(ui: &mut Ui, state: &mut AppState, plan: &RemapPlan) {
    ui.strong("Preview");
    item_picker(ui, state);
    ui.horizontal_wrapped(|ui| {
        let c = &mut state.sprites.convert;
        ui.add(
            egui::Slider::new(&mut c.zoom, 1..=MAX_ZOOM)
                .suffix("x")
                .text("zoom"),
        );
        ui.checkbox(&mut c.side_by_side, "Side by side");
        if !c.side_by_side {
            ui.selectable_value(&mut c.show_after, false, "Before");
            ui.selectable_value(&mut c.show_after, true, "After");
        }
        ui.checkbox(&mut c.highlight_changed, "Highlight changed pixels");
        ui.checkbox(&mut c.highlight_worst, "Highlight worst matches");
    });
    if let Some(input) = &state.sprites.convert.input {
        ui.weak(input.item_label(state.sprites.convert.item));
    }
    let Some((before, after)) = preview_textures(ui, state, plan) else {
        return;
    };
    let c = &state.sprites.convert;
    let src_name = c.source.label();
    let dst_name = c.target.label();
    let zoom = c.zoom as f32;
    let height = 380.0;
    let width = ui.available_width();
    let panes: Vec<(Option<Tex>, String)> = if c.side_by_side {
        vec![
            (before, format!("Before ({src_name})")),
            (after, format!("After ({dst_name})")),
        ]
    } else if c.show_after {
        vec![(after, format!("After ({dst_name})"))]
    } else {
        vec![(before, format!("Before ({src_name})"))]
    };
    let w = (width / panes.len() as f32 - 4.0).max(40.0);
    ui.horizontal(|ui| {
        for (tex, title) in panes {
            ui.vertical(|ui| {
                ui.label(title);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
                let painter = ui.painter_at(rect);
                paint_checker(&painter, rect, 8.0);
                if let Some(t) = tex {
                    let size = Vec2::new(t.size.0 as f32, t.size.1 as f32) * zoom;
                    let r = Rect::from_center_size(rect.center(), size);
                    painter.image(
                        t.handle.id(),
                        r,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "(empty)",
                        egui::FontId::proportional(14.0),
                        Color32::from_gray(200),
                    );
                }
            });
        }
    });
}

// ───────────────────────── convert and save ─────────────────────────

fn convert_and_result(ui: &mut Ui, state: &mut AppState, ready: bool) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                ready,
                egui::Button::new(RichText::new("Convert").strong().size(16.0))
                    .min_size(Vec2::new(120.0, 28.0)),
            )
            .on_hover_text("Applies the plan to a copy; the file is changed only when you save it")
            .clicked()
        {
            let _ = state.convert_apply();
        }
        ui.weak("Nothing is converted automatically.");
    });
    let Some(r) = &state.sprites.convert.result else {
        return;
    };
    let s = r.summary;
    let label = r.label();
    let saved = r.saved.clone();
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(GOOD, "\u{2714}");
        ui.label(&label);
        match &saved {
            Some(p) => ui.weak(format!("saved to {p}")),
            None => ui.colored_label(WARN, "not saved"),
        };
    });
    ui.weak(format!(
        "{} colour(s) changed, mean error {:.2}, worst {:.1}",
        s.changed, s.mean, s.worst
    ));
    let busy = state.sprites.dialogs.is_open();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                state.session.is_some(),
                egui::Button::new("Save to game path..."),
            )
            .on_hover_text(r"capybara\saves\<profile>\<game path>, CHANGED.txt and a snapshot")
            .clicked()
        {
            state.begin_converted_save();
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Save As..."))
            .clicked()
        {
            let name = state.converted_file_name();
            state
                .sprites
                .dialogs
                .open(Purpose::ConvertSaveAs, Some(name));
        }
        if ui.button("Show in the Viewer").clicked() {
            let _ = state.open_converted_in_viewer();
        }
    });
}

// ───────────────────────── batch ─────────────────────────

fn batch(ui: &mut Ui, state: &mut AppState) {
    let busy = state.sprites.dialogs.is_open();
    let running = state.sprites.convert.batch.task.is_some();
    ui.weak(
        "Converts every file with the source, target, settings and overrides above; \
         each file gets its own plan.",
    );
    ui.add_enabled_ui(!running, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Add files..."))
                .clicked()
            {
                state.sprites.dialogs.open(Purpose::ConvertBatchFiles, None);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Add folder..."))
                .clicked()
            {
                state
                    .sprites
                    .dialogs
                    .open(Purpose::ConvertBatchFolder, None);
            }
            ui.checkbox(
                &mut state.sprites.convert.batch.recursive,
                "with sub-folders",
            );
        });
        ui.horizontal(|ui| {
            ui.label("Game files containing");
            ui.add(
                egui::TextEdit::singleline(&mut state.sprites.convert.batch.game_filter)
                    .hint_text("e.g. tiles\\act5\\lava")
                    .desired_width(200.0),
            );
            let f = state.sprites.convert.batch.game_filter.clone();
            if ui
                .add_enabled(
                    state.session.is_some() && !f.trim().is_empty(),
                    egui::Button::new("Add"),
                )
                .clicked()
            {
                state.convert_batch_add_game(&f);
            }
        });
        let b = &mut state.sprites.convert.batch;
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{} file(s)", b.items.len()));
            if ui
                .add_enabled(!b.items.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                b.items.clear();
            }
        });
        if !b.items.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("convert_batch_items")
                .max_height(110.0)
                .show(ui, |ui| {
                    for i in b.items.iter().take(500) {
                        ui.weak(i.name());
                    }
                });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Output");
            ui.radio_value(&mut b.to_saves, true, "Game paths (saves)");
            ui.radio_value(&mut b.to_saves, false, "Folder");
        });
        if !state.sprites.convert.batch.to_saves {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Output folder..."))
                    .clicked()
                {
                    state
                        .sprites
                        .dialogs
                        .open(Purpose::ConvertBatchOutput, None);
                }
                let out = state
                    .sprites
                    .convert
                    .batch
                    .out_dir
                    .as_ref()
                    .map_or("(none)".to_owned(), |p| p.display().to_string());
                ui.weak(out);
            });
        }
        let b = &state.sprites.convert.batch;
        let ready = !b.items.is_empty() && (b.to_saves || b.out_dir.is_some());
        if ui
            .add_enabled(ready, egui::Button::new("Start batch"))
            .clicked()
        {
            let _ = state.convert_batch_start();
        }
    });
    if let Some(task) = &state.sprites.convert.batch.task {
        let p = task.progress();
        let frac = if p.total == 0 {
            0.0
        } else {
            p.done as f32 / p.total as f32
        };
        ui.add(
            egui::ProgressBar::new(frac)
                .show_percentage()
                .text(format!("{} / {}  {}", p.done, p.total, p.current)),
        );
        let cancelling = task.cancelled();
        if ui
            .add_enabled(
                !cancelling,
                egui::Button::new(if cancelling {
                    "Cancelling..."
                } else {
                    "Cancel"
                }),
            )
            .clicked()
        {
            task.cancel();
        }
        ui.ctx().request_repaint();
    } else if let Some((cancelled, text)) = &state.sprites.convert.batch.outcome {
        ui.colored_label(if *cancelled { WARN } else { GOOD }, text);
        ui.weak("Per-file results are in the Log.");
    }
}
