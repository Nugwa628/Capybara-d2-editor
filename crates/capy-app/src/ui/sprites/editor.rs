//! The sprite editor panel: top bar, tool strip and options, palette and
//! ramps, DCC cell check, frames strip and the canvas. The state machine is
//! `crate::sprites::editor`.

use std::hash::{Hash, Hasher};

use bevy_egui::egui::{
    self, Color32, PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use capy_paint::{
    BrushShape, Dither, FillMode, GradientShape, Mask, OutlineSide, ShapeStyle, onion_composite,
};
use d2_formats::palette::Palette;

use super::{paint_checker, palette_controls, rgba_texture, swatch_text};
use crate::sprites::Target;
use crate::sprites::dialog::Purpose;
use crate::sprites::editor::{EditorState, Gesture, GradientMode, Mods, PaintTool};
use crate::state::AppState;

const MAX_ZOOM: f32 = 64.0;

/// A highlighted cell: (dir, frame, (x0, y0, x1, y1)).
type Highlight = (usize, usize, (i32, i32, i32, i32));

/// Panel-local view state that is not part of the editor's model.
#[derive(Clone, Copy, Default)]
struct CanvasMemory {
    /// Cell highlighted by the DCC check.
    highlight: Option<Highlight>,
}

fn memory(ui: &Ui) -> CanvasMemory {
    ui.ctx()
        .data(|d| d.get_temp::<CanvasMemory>(egui::Id::new("sprite_editor_memory")))
        .unwrap_or_default()
}

fn set_memory(ui: &Ui, m: CanvasMemory) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("sprite_editor_memory"), m));
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    if state.sprites.editor.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(
                    "Nothing to edit yet.\nOpen a DC6/DCC in the Viewer and press \"Open in editor\",\n\
                     or import images in the Converter with \"Open in the editor too\".",
                )
                .size(16.0)
                .weak(),
            );
        });
        return;
    }
    egui::Panel::top("sprite_editor_top").show(ui, |ui| top_bar(ui, state));
    egui::Panel::left("sprite_editor_tools")
        .resizable(true)
        .default_size(190.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("sprite_editor_tools_scroll")
                .show(ui, |ui| tools(ui, state));
        });
    egui::Panel::right("sprite_editor_palette")
        .resizable(true)
        .default_size(250.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("sprite_editor_palette_scroll")
                .show(ui, |ui| palette_panel(ui, state));
        });
    egui::Panel::bottom("sprite_editor_frames")
        .resizable(true)
        .default_size(96.0)
        .show(ui, |ui| frames_strip(ui, state));
    egui::CentralPanel::no_frame().show(ui, |ui| canvas(ui, state));
}

fn editor(state: &mut AppState) -> Option<&mut EditorState> {
    state.sprites.editor.as_mut()
}

fn report(state: &mut AppState, r: Result<(), String>) {
    if let Err(e) = r {
        state.warn(format!("Editor: {e}"));
    }
}

// ───────────────────────── top bar ─────────────────────────

fn top_bar(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        let Some(e) = state.sprites.editor.as_ref() else {
            return;
        };
        let dirty = if e.is_dirty() { "\u{25CF} " } else { "" };
        ui.strong(format!("{dirty}{}", e.source.label()));
        ui.weak(format!(
            "{} | {} dirs x {} frames",
            e.format.extension().to_ascii_uppercase(),
            e.directions(),
            e.frames()
        ));
        let undo = e.session.undo_label().map(str::to_owned);
        let redo = e.session.redo_label().map(str::to_owned);
        ui.separator();
        if ui
            .add_enabled(undo.is_some(), egui::Button::new("Undo"))
            .on_hover_text(format!("Ctrl+Z {}", undo.clone().unwrap_or_default()))
            .clicked()
            && let Some(e) = editor(state)
        {
            e.undo();
        }
        if ui
            .add_enabled(redo.is_some(), egui::Button::new("Redo"))
            .on_hover_text(format!("Ctrl+Y {}", redo.clone().unwrap_or_default()))
            .clicked()
            && let Some(e) = editor(state)
        {
            e.redo();
        }
        if let Some(u) = &undo {
            ui.weak(format!("last: {u}"));
        }
        ui.separator();
        if ui
            .add_enabled(
                state.session.is_some(),
                egui::Button::new("Save to game path..."),
            )
            .on_hover_text("Ctrl+S")
            .clicked()
        {
            state.begin_sprite_save(Target::Editor);
        }
        ui.menu_button("Export", |ui| {
            let busy = state.sprites.dialogs.is_open();
            for (p, label) in [
                (Purpose::ExportFrames, "Frames + sprite.json..."),
                (Purpose::ExportSheet, "Sprite sheet..."),
                (Purpose::ExportGif, "Animated GIF (this direction)..."),
                (Purpose::ExportPng, "Indexed PNG (this frame)..."),
                (Purpose::ExportSprite, "Sprite file (DC6/DCC)..."),
            ] {
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    state.sprites.converter.target = Target::Editor;
                    state.sprite_dialog(p);
                    ui.close();
                }
            }
        });
        ui.menu_button("Format", |ui| {
            if let Some(e) = editor(state) {
                ui.radio_value(&mut e.format, capy_sprite::SpriteFormat::Dcc, "DCC");
                ui.radio_value(&mut e.format, capy_sprite::SpriteFormat::Dc6, "DC6");
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        palette_controls(ui, state);
        ui.separator();
        let Some(e) = editor(state) else { return };
        let mut z = e.zoom.round();
        if ui
            .add(
                egui::Slider::new(&mut z, 1.0..=MAX_ZOOM)
                    .suffix("x")
                    .text("zoom"),
            )
            .changed()
        {
            e.zoom = z;
        }
        ui.checkbox(&mut e.grid, "Grid");
        ui.checkbox(&mut e.checker, "Checker");
        ui.checkbox(&mut e.show_anchor, "Anchor");
        ui.checkbox(&mut e.onion, "Onion skin");
        ui.add_enabled(
            e.onion,
            egui::Slider::new(&mut e.onion_fade, 0.05..=0.9).text("fade"),
        );
    });
}

// ───────────────────────── tools ─────────────────────────

fn tools(ui: &mut Ui, state: &mut AppState) {
    let mut result: Result<(), String> = Ok(());
    {
        let Some(e) = editor(state) else { return };
        ui.strong("Tools");
        egui::Grid::new("sprite_tools_grid")
            .num_columns(2)
            .spacing([4.0, 4.0])
            .show(ui, |ui| {
                for (i, t) in PaintTool::ALL.into_iter().enumerate() {
                    let label = format!("{} ({})", t.label(), t.key());
                    if ui
                        .add_sized(
                            [88.0, 22.0],
                            egui::Button::selectable(e.tool == t, short(t)),
                        )
                        .on_hover_text(label)
                        .clicked()
                    {
                        e.cancel_gesture();
                        e.tool = t;
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.separator();
        ui.strong(e.tool.label());
        match e.tool {
            PaintTool::Pencil
            | PaintTool::Eraser
            | PaintTool::Line
            | PaintTool::Rect
            | PaintTool::Ellipse
            | PaintTool::Shade => {
                ui.horizontal(|ui| {
                    ui.label("Brush");
                    ui.add(egui::DragValue::new(&mut e.brush.size).range(1..=32));
                    ui.radio_value(&mut e.brush.shape, BrushShape::Square, "Square");
                    ui.radio_value(&mut e.brush.shape, BrushShape::Round, "Round");
                });
                if matches!(e.tool, PaintTool::Rect | PaintTool::Ellipse) {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut e.shape_style, ShapeStyle::Outline, "Outline");
                        ui.radio_value(&mut e.shape_style, ShapeStyle::Filled, "Filled");
                    });
                }
                if e.tool == PaintTool::Shade {
                    ui.horizontal(|ui| {
                        ui.label("Steps");
                        ui.add(egui::DragValue::new(&mut e.shade_steps).range(1..=8));
                    });
                    ui.weak("Left button lightens, right darkens, along each pixel's own ramp.");
                }
            }
            PaintTool::Fill | PaintTool::Wand => {
                ui.radio_value(&mut e.fill_mode, FillMode::Contiguous, "Contiguous");
                ui.radio_value(&mut e.fill_mode, FillMode::Global, "Global (same index)");
            }
            PaintTool::Replace => {
                ui.checkbox(&mut e.replace_everywhere, "In every frame");
                ui.weak("Click a colour: it becomes the primary (right: secondary).");
            }
            PaintTool::Picker => {
                ui.weak("Left: primary colour, right: secondary.");
            }
            PaintTool::Gradient => {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut e.gradient.shape, GradientShape::Linear, "Linear");
                    ui.radio_value(&mut e.gradient.shape, GradientShape::Radial, "Radial");
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut e.gradient.mode, GradientMode::Two, "Two colours");
                    ui.radio_value(&mut e.gradient.mode, GradientMode::Ramp, "Ramp");
                });
                ui.horizontal(|ui| {
                    ui.label("Dither");
                    ui.radio_value(&mut e.gradient.dither, Dither::None, "None");
                    ui.radio_value(&mut e.gradient.dither, Dither::Bayer4, "Bayer");
                    ui.radio_value(&mut e.gradient.dither, Dither::FloydSteinberg, "F-S");
                });
                ui.checkbox(&mut e.lock_alpha, "Lock alpha (opaque pixels only)");
                ui.weak("Drag from start to end. Ramp uses the primary colour's ramp.");
            }
            t if t.is_selection() || t == PaintTool::Move => {
                ui.weak("Shift adds, Ctrl subtracts, both intersect.");
            }
            PaintTool::Offset => {
                ui.weak("Drag to move the frame against the anchor.");
            }
            _ => {}
        }
        ui.separator();
        ui.strong("Selection");
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("All").clicked() {
                e.select_all();
            }
            if ui.small_button("None").clicked() {
                e.deselect();
            }
            if ui.small_button("Invert").clicked() {
                e.invert_selection();
            }
            if ui.small_button("Grow").clicked() {
                e.grow_selection(1);
            }
            if ui.small_button("Shrink").clicked() {
                e.grow_selection(-1);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Flip H").clicked() {
                result = e.flip(true);
            }
            if ui.small_button("Flip V").clicked() {
                result = e.flip(false);
            }
            if ui.small_button("Rot CW").clicked() {
                result = e.rotate(true);
            }
            if ui.small_button("Rot CCW").clicked() {
                result = e.rotate(false);
            }
        });
        if let Some(m) = e.mask() {
            ui.weak(format!("{} pixels selected", m.count()));
        }
        ui.separator();
        ui.strong("Effects");
        ui.checkbox(&mut e.apply_all_frames, "All frames of this direction")
            .on_hover_text("Outline, clean strays and offset drags");
        ui.horizontal(|ui| {
            ui.checkbox(&mut e.outline.auto, "Auto darker");
            if e.outline.auto {
                ui.add(egui::DragValue::new(&mut e.outline.steps).range(-8..=8))
                    .on_hover_text("Ramp steps (negative = darker)");
            } else {
                ui.weak("primary colour");
            }
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut e.outline.side, OutlineSide::Outside, "Outside");
            ui.radio_value(&mut e.outline.side, OutlineSide::Inside, "Inside");
        });
        ui.horizontal(|ui| {
            ui.label("Thickness");
            ui.add(egui::DragValue::new(&mut e.outline.thickness).range(1..=8));
            ui.checkbox(&mut e.outline.diagonals, "Diagonals");
        });
        ui.horizontal(|ui| {
            if ui.button("Outline").clicked() {
                result = e.apply_outline(None);
            }
            if ui.button("Clean strays").clicked() {
                result = e.clean_strays();
            }
        });
        ui.separator();
        ui.strong("Canvas");
        if let Some(f) = e.frame()
            && e.canvas_size == (0, 0)
        {
            e.canvas_size = (f.width, f.height);
        }
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut e.canvas_size.0).range(1..=4096));
            ui.label("x");
            ui.add(egui::DragValue::new(&mut e.canvas_size.1).range(1..=4096));
            if ui
                .button("Resize")
                .on_hover_text("Bottom-centre stays put, so the anchor does not move")
                .clicked()
            {
                let (w, h) = e.canvas_size;
                result = e.resize_canvas(w, h);
            }
        });
    }
    report(state, result);
}

fn short(t: PaintTool) -> &'static str {
    match t {
        PaintTool::Pencil => "Pencil",
        PaintTool::Eraser => "Eraser",
        PaintTool::Line => "Line",
        PaintTool::Rect => "Rect",
        PaintTool::Ellipse => "Ellipse",
        PaintTool::Fill => "Fill",
        PaintTool::Replace => "Replace",
        PaintTool::Picker => "Picker",
        PaintTool::Gradient => "Gradient",
        PaintTool::Shade => "Shade",
        PaintTool::SelectRect => "Sel rect",
        PaintTool::SelectEllipse => "Sel ellipse",
        PaintTool::Lasso => "Lasso",
        PaintTool::Wand => "Wand",
        PaintTool::Move => "Move",
        PaintTool::Offset => "Offset",
    }
}

// ───────────────────────── palette ─────────────────────────

fn color(p: &Palette, i: u8) -> Color32 {
    let [r, g, b] = p.colors[usize::from(i)];
    Color32::from_rgb(r, g, b)
}

fn palette_panel(ui: &mut Ui, state: &mut AppState) {
    let palette = state.sprite_display_palette();
    let Some(e) = editor(state) else { return };
    // Primary / secondary.
    ui.horizontal(|ui| {
        for (label, idx) in [("Primary", e.primary()), ("Secondary", e.secondary())] {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, color(&palette, idx));
            ui.painter().rect_stroke(
                r,
                2.0,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Outside,
            );
            ui.label(format!("{label}\n{idx}"))
                .on_hover_text(swatch_text(idx, palette.colors[usize::from(idx)]));
        }
        if ui.small_button("Swap (X)").clicked() {
            e.swap_colors();
        }
    });
    if ui
        .button("Transparent (eraser)")
        .on_hover_text("Index 0 is the transparent key; it is never a drawing colour")
        .clicked()
    {
        e.cancel_gesture();
        e.tool = PaintTool::Eraser;
    }
    ui.separator();
    // 16x16 grid.
    let cell = ((ui.available_width() - 4.0) / 16.0).clamp(8.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(cell * 16.0), Sense::click());
    let painter = ui.painter_at(rect);
    for i in 0..=255u8 {
        let (x, y) = (f32::from(i % 16), f32::from(i / 16));
        let r = Rect::from_min_size(rect.min + Vec2::new(x * cell, y * cell), Vec2::splat(cell));
        if i == 0 {
            paint_checker(&painter, r.shrink(1.0), 3.0);
            painter.line_segment(
                [r.left_bottom(), r.right_top()],
                Stroke::new(1.0, Color32::from_rgb(220, 60, 60)),
            );
            continue;
        }
        painter.rect_filled(r.shrink(0.5), 0.0, color(&palette, i));
        if i == e.primary() {
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        } else if i == e.secondary() {
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(2.0, Color32::from_gray(90)),
                StrokeKind::Inside,
            );
        }
    }
    let hit = |pos: Option<Pos2>| -> Option<u8> {
        let p = pos?;
        if !rect.contains(p) {
            return None;
        }
        let (x, y) = (
            ((p.x - rect.min.x) / cell) as u32,
            ((p.y - rect.min.y) / cell) as u32,
        );
        u8::try_from(y.min(15) * 16 + x.min(15)).ok()
    };
    if let Some(i) = hit(response.hover_pos()) {
        let text = if i == 0 {
            "index 0: transparent key (not a colour)".to_owned()
        } else {
            format!(
                "{}  (left: primary, right: secondary)",
                swatch_text(i, palette.colors[usize::from(i)])
            )
        };
        response.clone().on_hover_text(text);
    }
    if response.clicked()
        && let Some(i) = hit(response.interact_pointer_pos())
    {
        e.set_primary(i);
    }
    if response.secondary_clicked()
        && let Some(i) = hit(response.interact_pointer_pos())
    {
        e.set_secondary(i);
    }
    // Ramps.
    egui::CollapsingHeader::new("Ramps")
        .default_open(true)
        .show(ui, |ui| {
            let ramps: Vec<Vec<u8>> = e.session.ramps().all().map(|(_, r)| r.to_vec()).collect();
            let primary_ramp = e.session.ramps().ramp_of(e.primary()).map(|(id, _)| id);
            let sw = 10.0;
            for (n, ramp) in ramps.iter().enumerate() {
                let width = (ramp.len() as f32 * sw).min(ui.available_width());
                let (r, resp) = ui.allocate_exact_size(Vec2::new(width, sw + 2.0), Sense::click());
                let per = (width / ramp.len().max(1) as f32).max(1.0);
                for (k, &i) in ramp.iter().enumerate() {
                    let cr = Rect::from_min_size(
                        r.min + Vec2::new(k as f32 * per, 0.0),
                        Vec2::new(per, sw),
                    );
                    ui.painter().rect_filled(cr, 0.0, color(&palette, i));
                }
                if primary_ramp.is_some_and(|id| usize::from(id.0) == n) {
                    ui.painter().rect_stroke(
                        r,
                        0.0,
                        Stroke::new(1.0, Color32::WHITE),
                        StrokeKind::Outside,
                    );
                }
                let pick = |p: Option<Pos2>| -> Option<u8> {
                    let p = p?;
                    let k = ((p.x - r.min.x) / per) as usize;
                    ramp.get(k.min(ramp.len().saturating_sub(1))).copied()
                };
                if resp.clicked()
                    && let Some(i) = pick(resp.interact_pointer_pos())
                {
                    e.set_primary(i);
                }
                if resp.secondary_clicked()
                    && let Some(i) = pick(resp.interact_pointer_pos())
                {
                    e.set_secondary(i);
                }
            }
        });
    dcc_check(ui, state);
}

fn dcc_check(ui: &mut Ui, state: &mut AppState) {
    let mut jump = None;
    let mut reduce = false;
    {
        let Some(e) = editor(state) else { return };
        let is_dcc = e.format == capy_sprite::SpriteFormat::Dcc;
        egui::CollapsingHeader::new(format!(
            "DCC cell check{}",
            if is_dcc { "" } else { " (sprite is DC6)" }
        ))
        .default_open(is_dcc)
        .show(ui, |ui| {
            let list = e.violations().to_vec();
            if list.is_empty() {
                ui.colored_label(
                    Color32::from_rgb(110, 200, 110),
                    "Every 4x4 cell fits (4 values at most).",
                );
                return;
            }
            ui.colored_label(
                Color32::from_rgb(230, 150, 60),
                format!("{} cell(s) hold more than 4 values", list.len()),
            );
            if ui
                .button("Reduce all")
                .on_hover_text("Merges the nearest colours in each cell (one undo step)")
                .clicked()
            {
                reduce = true;
            }
            egui::ScrollArea::vertical()
                .id_salt("dcc_violations")
                .max_height(160.0)
                .show(ui, |ui| {
                    for v in list.iter().take(500) {
                        let text = format!(
                            "dir {} frame {}  cell {},{}  {} values",
                            v.frame.dir, v.frame.frame, v.cell_x, v.cell_y, v.colors
                        );
                        if ui.selectable_label(false, text).clicked() {
                            jump = Some(v.clone());
                        }
                    }
                });
        });
    }
    if reduce && let Some(e) = editor(state) {
        match e.reduce_all() {
            Ok(n) => state.info(format!("Editor: reduced {n} DCC cell(s)")),
            Err(err) => state.warn(format!("Editor: {err}")),
        }
    }
    if let Some(v) = jump
        && let Some(e) = editor(state)
    {
        e.go_to(v.frame.dir, v.frame.frame);
        let r = crate::sprites::editor::cell_rect(&v);
        // Centre the view on the cell.
        if let Some(f) = e.frame() {
            let c = Vec2::new((r.0 + r.2) as f32 / 2.0, (r.1 + r.3) as f32 / 2.0);
            let half = Vec2::new(f.width as f32, f.height as f32) / 2.0;
            e.pan = (half - c) * e.zoom;
        }
        let mut m = memory(ui);
        m.highlight = Some((v.frame.dir, v.frame.frame, r));
        set_memory(ui, m);
    }
}

// ───────────────────────── frames ─────────────────────────

fn frames_strip(ui: &mut Ui, state: &mut AppState) {
    let palette = state.sprite_display_palette();
    let pal_key = state.sprites.palette.key(state.session_epoch);
    let mut result: Result<(), String> = Ok(());
    {
        let Some(e) = editor(state) else { return };
        ui.horizontal_wrapped(|ui| {
            ui.label("Direction");
            let dirs = e.directions();
            let mut dir = e.cur.dir;
            egui::ComboBox::from_id_salt("editor_dir")
                .selected_text(format!("{dir} / {dirs}"))
                .show_ui(ui, |ui| {
                    for d in 0..dirs {
                        ui.selectable_value(&mut dir, d, d.to_string());
                    }
                });
            if dir != e.cur.dir {
                e.go_to(dir, e.cur.frame);
            }
            ui.label(format!("Frame {} / {}", e.cur.frame, e.frames()));
            if ui.small_button("<").on_hover_text(",").clicked() {
                e.step_frame(-1);
            }
            if ui.small_button(">").on_hover_text(".").clicked() {
                e.step_frame(1);
            }
            ui.separator();
            if ui
                .small_button("Insert")
                .on_hover_text("A blank frame after this one (every direction)")
                .clicked()
            {
                result = e.insert_frame(false);
            }
            if ui.small_button("Duplicate").clicked() {
                result = e.insert_frame(true);
            }
            if ui.small_button("Delete").clicked() {
                result = e.delete_frame();
            }
            if ui.small_button("Move left").clicked() {
                result = e.move_frame(-1);
            }
            if ui.small_button("Move right").clicked() {
                result = e.move_frame(1);
            }
            ui.separator();
            ui.label("Mirror into");
            let mut target = e.mirror_target.min(dirs.saturating_sub(1));
            egui::ComboBox::from_id_salt("mirror_target")
                .width(50.0)
                .selected_text(target.to_string())
                .show_ui(ui, |ui| {
                    for d in 0..dirs {
                        ui.selectable_value(&mut target, d, d.to_string());
                    }
                });
            e.mirror_target = target;
            if ui
                .small_button("Mirror direction")
                .on_hover_text("Writes a horizontally mirrored copy of this direction")
                .clicked()
            {
                result = e.mirror_direction();
            }
        });
        // Thumbnails.
        let rev = e.session.revision() ^ pal_key.rotate_left(17);
        let dir = e.cur.dir;
        let mut go = None;
        egui::ScrollArea::horizontal()
            .id_salt("editor_frames_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for frame in 0..e.frames() {
                        let fresh = e.thumbs.get(&(dir, frame)).is_some_and(|(k, _)| *k == rev);
                        if !fresh {
                            let tex = e
                                .session
                                .frames()
                                .get(dir)
                                .and_then(|d| d.get(frame))
                                .and_then(|img| {
                                    rgba_texture(
                                        ui.ctx(),
                                        &format!("thumb-{dir}-{frame}"),
                                        img.width,
                                        img.height,
                                        &img.to_rgba(&palette),
                                    )
                                });
                            match tex {
                                Some(t) => {
                                    e.thumbs.insert((dir, frame), (rev, t));
                                }
                                None => {
                                    e.thumbs.remove(&(dir, frame));
                                }
                            }
                        }
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::click());
                        paint_checker(&ui.painter_at(r), r, 4.0);
                        if let Some((_, t)) = e.thumbs.get(&(dir, frame)) {
                            let sz = t.size_vec2();
                            let k = (48.0 / sz.x.max(sz.y)).min(4.0);
                            let ir = Rect::from_center_size(r.center(), sz * k);
                            ui.painter().image(
                                t.id(),
                                ir,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        let stroke = if frame == e.cur.frame {
                            Stroke::new(2.0, Color32::from_rgb(240, 200, 60))
                        } else {
                            Stroke::new(1.0, Color32::from_gray(80))
                        };
                        ui.painter().rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
                        if resp.on_hover_text(format!("frame {frame}")).clicked() {
                            go = Some(frame);
                        }
                    }
                });
            });
        if let Some(f) = go {
            e.go_to(dir, f);
        }
    }
    report(state, result);
}

// ───────────────────────── clipboard ─────────────────────────

pub fn copy(state: &mut AppState) {
    let palette = state.sprite_palette();
    let os = state.sprites.use_os_clipboard;
    let Some(e) = editor(state) else { return };
    let Some(clip) = e.copy().cloned() else {
        return;
    };
    if os {
        let rgba = clip.to_rgba(&palette);
        let image = arboard::ImageData {
            width: clip.width as usize,
            height: clip.height as usize,
            bytes: std::borrow::Cow::Owned(rgba),
        };
        if let Err(err) = arboard::Clipboard::new().and_then(|mut c| c.set_image(image)) {
            state.warn(format!(
                "Copied inside the editor; the OS clipboard refused the image: {err}"
            ));
        }
    }
}

pub fn cut(state: &mut AppState) {
    let palette = state.sprite_palette();
    let os = state.sprites.use_os_clipboard;
    let Some(e) = editor(state) else { return };
    let r = e.cut();
    let clip = e.clip.clone();
    report(state, r);
    if os && let Some(clip) = clip {
        let image = arboard::ImageData {
            width: clip.width as usize,
            height: clip.height as usize,
            bytes: std::borrow::Cow::Owned(clip.to_rgba(&palette)),
        };
        let _ = arboard::Clipboard::new().and_then(|mut c| c.set_image(image));
    }
}

pub fn paste(state: &mut AppState) {
    let has_clip = state
        .sprites
        .editor
        .as_ref()
        .is_some_and(|e| e.clip.is_some());
    if !has_clip {
        paste_os_image(state);
        return;
    }
    let Some(e) = editor(state) else { return };
    let r = e.paste();
    report(state, r);
}

/// Pastes an RGBA image from the OS clipboard, mapped to the palette.
pub fn paste_os_image(state: &mut AppState) {
    if !state.sprites.use_os_clipboard {
        return;
    }
    let image = arboard::Clipboard::new().and_then(|mut c| c.get_image());
    let Some(e) = editor(state) else { return };
    let r = match image {
        Ok(img) => e.paste_rgba(
            img.width as u32,
            img.height as u32,
            &img.bytes,
            Dither::None,
        ),
        Err(err) => Err(format!("the clipboard holds no image ({err})")),
    };
    report(state, r);
}

// ───────────────────────── canvas ─────────────────────────

/// The canvas texture: the current frame, or the onion composite.
fn canvas_texture(ui: &Ui, state: &mut AppState) -> Option<(egui::TextureHandle, (i32, i32))> {
    let pal_key = state.sprites.palette.key(state.session_epoch);
    let palette = state.sprite_display_palette();
    let e = editor(state)?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        e.session.revision(),
        e.cur,
        e.onion,
        e.onion_fade.to_bits(),
        pal_key,
    )
        .hash(&mut h);
    let key = h.finish();
    if let Some((k, t, o)) = &e.canvas_tex
        && *k == key
    {
        return Some((t.clone(), *o));
    }
    let cur = e.frame()?.clone();
    let (w, hgt, rgba, origin) = if e.onion && e.frames() > 1 {
        let n = e.frames();
        let dir = &e.session.frames()[e.cur.dir];
        let prev = dir.get((e.cur.frame + n - 1) % n);
        let next = dir.get((e.cur.frame + 1) % n);
        let (w, h, rgba) = onion_composite(&cur, prev, next, &palette, e.onion_fade);
        let ox = [Some(&cur), prev, next]
            .iter()
            .flatten()
            .filter(|i| i.width > 0 && i.height > 0)
            .map(|i| i.offset_x)
            .min()
            .unwrap_or(cur.offset_x);
        let oy = [Some(&cur), prev, next]
            .iter()
            .flatten()
            .filter(|i| i.width > 0 && i.height > 0)
            .map(|i| i.offset_y)
            .min()
            .unwrap_or(cur.offset_y);
        (w, h, rgba, (ox - cur.offset_x, oy - cur.offset_y))
    } else {
        (cur.width, cur.height, cur.to_rgba(&palette), (0, 0))
    };
    let tex = rgba_texture(ui.ctx(), "sprite-editor-canvas", w, hgt, &rgba)?;
    e.canvas_tex = Some((key, tex.clone(), origin));
    Some((tex, origin))
}

/// Frame pixel <-> screen mapping.
#[derive(Clone, Copy)]
struct View {
    origin: Pos2,
    zoom: f32,
}

impl View {
    fn to_screen(self, x: f32, y: f32) -> Pos2 {
        Pos2::new(self.origin.x + x * self.zoom, self.origin.y + y * self.zoom)
    }
    fn to_frame(self, p: Pos2) -> (f32, f32) {
        (
            (p.x - self.origin.x) / self.zoom,
            (p.y - self.origin.y) / self.zoom,
        )
    }
    fn px_rect(self, x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::from_min_max(
            self.to_screen(x0 as f32, y0 as f32),
            self.to_screen(x1 as f32, y1 as f32),
        )
    }
}

fn canvas(ui: &mut Ui, state: &mut AppState) {
    let size = ui.available_size().max(Vec2::splat(1.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(32));
    let tex = canvas_texture(ui, state);
    let mem = memory(ui);
    let (pointer, zoom_delta, scroll, space, pressed, down, shift, ctrl) = ui.input(|i| {
        (
            i.pointer.interact_pos().or(i.pointer.hover_pos()),
            i.zoom_delta(),
            i.smooth_scroll_delta,
            i.key_down(egui::Key::Space),
            [
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.button_pressed(PointerButton::Secondary),
            ],
            [
                i.pointer.button_down(PointerButton::Primary),
                i.pointer.button_down(PointerButton::Secondary),
            ],
            i.modifiers.shift,
            i.modifiers.command,
        )
    });
    let mut problem: Option<String> = None;
    let Some(e) = editor(state) else { return };
    let Some((fw, fh, ox, oy)) = e
        .frame()
        .map(|f| (f.width, f.height, f.offset_x, f.offset_y))
    else {
        return;
    };
    // View: the frame centred, then panned.
    let zoom = e.zoom.round().clamp(1.0, MAX_ZOOM);
    let origin = (rect.center() + e.pan - Vec2::new(fw as f32, fh as f32) * zoom / 2.0).round();
    let mut view = View { origin, zoom };
    let hovered = response.hovered();
    // Ctrl+wheel zooms at the cursor; the wheel pans.
    if hovered && zoom_delta != 1.0 {
        let new_zoom = (e.zoom * zoom_delta).clamp(1.0, MAX_ZOOM);
        if let Some(p) = pointer {
            let (fx, fy) = view.to_frame(p);
            let nz = new_zoom.round();
            // keep (fx, fy) under the cursor
            let new_origin = p - Vec2::new(fx, fy) * nz;
            e.pan = new_origin - (rect.center() - Vec2::new(fw as f32, fh as f32) * nz / 2.0);
        }
        e.zoom = new_zoom;
    } else if hovered && scroll != Vec2::ZERO {
        e.pan += scroll;
    }
    let panning = response.dragged_by(PointerButton::Middle)
        || (space && response.dragged_by(PointerButton::Primary));
    if panning {
        e.pan += response.drag_delta();
    }
    let zoom = e.zoom.round().clamp(1.0, MAX_ZOOM);
    view = View {
        origin: (rect.center() + e.pan - Vec2::new(fw as f32, fh as f32) * zoom / 2.0).round(),
        zoom,
    };
    // Gestures.
    if !space && let Some(p) = pointer {
        let fp = view.to_frame(p);
        let mods = Mods { shift, ctrl };
        if e.gesture.is_none() && hovered {
            if pressed[0] {
                problem = e.press(fp, false, mods).err();
            } else if pressed[1] {
                problem = e.press(fp, true, mods).err();
            }
        } else if e.gesture.is_some() {
            if down[0] || down[1] {
                problem = e.drag(fp).err();
            } else {
                problem = e.release(fp).err();
            }
        }
    }
    // Picture.
    let frame_rect = view.px_rect(0, 0, fw as i32, fh as i32);
    if e.checker {
        paint_checker(&painter, frame_rect, (zoom * 4.0).clamp(4.0, 16.0));
    } else {
        painter.rect_filled(frame_rect, 0.0, Color32::BLACK);
    }
    if let Some((t, (tx, ty))) = &tex {
        let sz = t.size();
        let r = view.px_rect(*tx, *ty, *tx + sz[0] as i32, *ty + sz[1] as i32);
        painter.image(
            t.id(),
            r,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.rect_stroke(
        frame_rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(120)),
        StrokeKind::Outside,
    );
    if e.grid && zoom >= 4.0 {
        let grid = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 28));
        let vis = frame_rect.intersect(rect);
        let (x0, _) = view.to_frame(vis.min);
        let (x1, _) = view.to_frame(vis.max);
        for x in (x0.floor().max(0.0) as i32)..=(x1.ceil().min(fw as f32) as i32) {
            let sx = view.to_screen(x as f32, 0.0).x;
            painter.line_segment([Pos2::new(sx, vis.min.y), Pos2::new(sx, vis.max.y)], grid);
        }
        let (_, y0) = view.to_frame(vis.min);
        let (_, y1) = view.to_frame(vis.max);
        for y in (y0.floor().max(0.0) as i32)..=(y1.ceil().min(fh as f32) as i32) {
            let sy = view.to_screen(0.0, y as f32).y;
            painter.line_segment([Pos2::new(vis.min.x, sy), Pos2::new(vis.max.x, sy)], grid);
        }
    }
    // Selection outline.
    if let Some(m) = e.mask() {
        let shift_by = match &e.gesture {
            Some(Gesture::Move { start, end }) => (end.0 - start.0, end.1 - start.1),
            _ => (0, 0),
        };
        draw_mask(&painter, view, m, shift_by);
    }
    // Gesture feedback.
    let ant = Stroke::new(1.5, Color32::from_rgb(120, 220, 255));
    match &e.gesture {
        Some(Gesture::Shape { start, end, .. }) | Some(Gesture::Select { start, end, .. }) => {
            let (a, b) = (
                (start.0.min(end.0), start.1.min(end.1)),
                (start.0.max(end.0) + 1, start.1.max(end.1) + 1),
            );
            let r = view.px_rect(a.0, a.1, b.0, b.1);
            match e.tool {
                PaintTool::Line => {
                    painter.line_segment(
                        [
                            view.to_screen(start.0 as f32 + 0.5, start.1 as f32 + 0.5),
                            view.to_screen(end.0 as f32 + 0.5, end.1 as f32 + 0.5),
                        ],
                        ant,
                    );
                }
                PaintTool::Ellipse | PaintTool::SelectEllipse => {
                    painter.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, ant));
                }
                _ => {
                    painter.rect_stroke(r, 0.0, ant, StrokeKind::Inside);
                }
            }
        }
        Some(Gesture::Gradient { start, end }) => {
            let (a, b) = (
                view.to_screen(start.0, start.1),
                view.to_screen(end.0, end.1),
            );
            painter.arrow(a, b - a, ant);
        }
        Some(Gesture::Lasso { points, .. }) => {
            let pts: Vec<Pos2> = points.iter().map(|p| view.to_screen(p.0, p.1)).collect();
            painter.add(egui::Shape::line(pts, ant));
        }
        _ => {}
    }
    // Anchor (moves with an offset drag).
    if e.show_anchor {
        let d = match &e.gesture {
            Some(Gesture::Offset { start, end }) => (end.0 - start.0, end.1 - start.1),
            _ => (0, 0),
        };
        let a = view.to_screen((-ox - d.0) as f32, (-oy - d.1) as f32);
        let red = Stroke::new(1.5, Color32::from_rgb(255, 70, 70));
        painter.line_segment([a - Vec2::new(10.0, 0.0), a + Vec2::new(10.0, 0.0)], red);
        painter.line_segment([a - Vec2::new(0.0, 10.0), a + Vec2::new(0.0, 10.0)], red);
    }
    // DCC cell highlight.
    if let Some((d, f, (x0, y0, x1, y1))) = mem.highlight
        && d == e.cur.dir
        && f == e.cur.frame
    {
        painter.rect_stroke(
            view.px_rect(x0, y0, x1, y1),
            0.0,
            Stroke::new(2.0, Color32::from_rgb(255, 60, 200)),
            StrokeKind::Outside,
        );
    }
    // Brush cursor.
    if hovered
        && let Some(p) = pointer
        && matches!(
            e.tool,
            PaintTool::Pencil | PaintTool::Eraser | PaintTool::Shade
        )
    {
        let (fx, fy) = view.to_frame(p);
        let s = e.brush.size as i32;
        let (cx, cy) = (
            fx.floor() as i32 - (s - 1) / 2,
            fy.floor() as i32 - (s - 1) / 2,
        );
        painter.rect_stroke(
            view.px_rect(cx, cy, cx + s, cy + s),
            0.0,
            Stroke::new(1.0, Color32::WHITE),
            StrokeKind::Outside,
        );
    }
    // Status line.
    let at = pointer
        .filter(|_| hovered)
        .map(|p| {
            let (fx, fy) = view.to_frame(p);
            let (x, y) = (fx.floor() as i32, fy.floor() as i32);
            let idx = e
                .frame()
                .filter(|_| x >= 0 && y >= 0 && (x as u32) < fw && (y as u32) < fh)
                .map(|f| {
                    let i = y as usize * fw as usize + x as usize;
                    if f.alpha[i] == 0 {
                        "transparent".to_owned()
                    } else {
                        format!("index {}", f.pixels[i])
                    }
                })
                .unwrap_or_default();
            format!("  ({x}, {y}) {idx}")
        })
        .unwrap_or_default();
    painter.text(
        rect.left_bottom() + Vec2::new(6.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        format!(
            "dir {} frame {}  {}x{} offset ({}, {})  {}x{}{at}",
            e.cur.dir, e.cur.frame, fw, fh, ox, oy, zoom, "",
        ),
        egui::FontId::monospace(12.0),
        Color32::from_gray(230),
    );
    if e.gesture.is_some() {
        ui.ctx().request_repaint();
    }
    if let Some(p) = problem {
        state.warn(format!("Editor: {p}"));
    }
}

/// Outlines the selected pixels (edges between selected and unselected).
fn draw_mask(painter: &egui::Painter, view: View, m: &Mask, shift: (i32, i32)) {
    let stroke = Stroke::new(1.0, Color32::from_rgb(255, 255, 120));
    let (w, h) = (m.width as i32, m.height as i32);
    let mut segments = 0usize;
    let (sx, sy) = shift;
    for y in 0..h {
        for x in 0..w {
            if !m.get(x, y) {
                continue;
            }
            let (px, py) = ((x + sx) as f32, (y + sy) as f32);
            for (dx, dy, a, b) in [
                (-1, 0, (px, py), (px, py + 1.0)),
                (1, 0, (px + 1.0, py), (px + 1.0, py + 1.0)),
                (0, -1, (px, py), (px + 1.0, py)),
                (0, 1, (px, py + 1.0), (px + 1.0, py + 1.0)),
            ] {
                if !m.get(x + dx, y + dy) {
                    painter
                        .line_segment([view.to_screen(a.0, a.1), view.to_screen(b.0, b.1)], stroke);
                    segments += 1;
                }
            }
            if segments > 40_000 {
                // A very ragged selection: show its bounds instead.
                if let Some((x0, y0, x1, y1)) = m.bounds() {
                    painter.rect_stroke(
                        view.px_rect(
                            x0 as i32 + sx,
                            y0 as i32 + sy,
                            x1 as i32 + 1 + sx,
                            y1 as i32 + 1 + sy,
                        ),
                        0.0,
                        stroke,
                        StrokeKind::Outside,
                    );
                }
                return;
            }
        }
    }
}
