//! The Sprites mode panels (browser, viewer, editor, converter), their
//! keyboard shortcuts and questions, and the map editor's Sprite preview box.
//!
//! | Keys (Sprites mode) | Action |
//! |---|---|
//! | Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z | editor undo, redo |
//! | Ctrl+S | save the editor's sprite to a game path |
//! | Ctrl+C, Ctrl+X, Ctrl+V | copy, cut, paste (editor) |
//! | Ctrl+Shift+V | paste an image from the OS clipboard (palette-mapped) |
//! | Ctrl+A, Ctrl+D, Ctrl+I | select all, deselect, invert selection |
//! | Delete | clear the selected pixels |
//! | Esc | cancel the gesture, else deselect |
//! | B E L R C F K I G H M U Q W V O | editor tools (see the tool strip) |
//! | X | swap primary and secondary colour |
//! | [ ] | brush size |
//! | , . | previous / next frame |
//! | Space | play / pause (viewer) |
//! | Ctrl+wheel, wheel, middle drag | zoom and pan (editor); wheel zooms the viewer |

mod browser;
pub mod convert_palette;
mod converter;
mod editor;
pub mod gif_dialog;
pub mod preview;
mod viewer;

use bevy_egui::egui::{
    self, Color32, ColorImage, Context, Id, Key, Modifiers, Painter, Rect, TextureHandle,
    TextureOptions, TextureWrapMode, Ui,
};

use super::SpritesPanel;
use crate::sprites::Focus;
use crate::sprites::palette::{PaletteChoice, choices};
use crate::state::AppState;

pub fn show(ui: &mut Ui, panel: SpritesPanel, state: &mut AppState) {
    if ui.rect_contains_pointer(ui.max_rect()) {
        match panel {
            SpritesPanel::Viewer => state.sprites.hot = Some(Focus::Viewer),
            SpritesPanel::Editor => state.sprites.hot = Some(Focus::Editor),
            _ => {}
        }
    }
    match panel {
        SpritesPanel::Browser => browser::show(ui, state),
        SpritesPanel::Viewer => viewer::show(ui, state),
        SpritesPanel::Editor => editor::show(ui, state),
        SpritesPanel::Converter => converter::show(ui, state),
        SpritesPanel::Log => super::log::show(ui, state),
    }
}

// ───────────────────────── shared widgets ─────────────────────────

/// A 2x2 checker texture drawn repeated behind sprites.
fn checker_texture(ctx: &Context) -> TextureHandle {
    let id = Id::new("sprites_checker_texture");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let (a, b) = (Color32::from_gray(58), Color32::from_gray(82));
    let image = ColorImage::new([2, 2], vec![a, b, b, a]);
    let options = TextureOptions {
        wrap_mode: TextureWrapMode::Repeat,
        ..TextureOptions::NEAREST
    };
    let tex = ctx.load_texture("sprites-checker", image, options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Fills `rect` with a checkerboard of `cell`-point squares.
pub fn paint_checker(painter: &Painter, rect: Rect, cell: f32) {
    let tex = checker_texture(painter.ctx());
    let cell = cell.max(2.0) * 2.0;
    let uv = Rect::from_min_max(
        egui::pos2(rect.min.x / cell, rect.min.y / cell),
        egui::pos2(rect.max.x / cell, rect.max.y / cell),
    );
    painter.image(tex.id(), rect, uv, Color32::WHITE);
}

/// Uploads straight-alpha RGBA pixels as a nearest-filtered texture.
pub fn rgba_texture(
    ctx: &Context,
    name: &str,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> Option<TextureHandle> {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba[..w * h * 4]);
    Some(ctx.load_texture(name, image, TextureOptions::NEAREST))
}

/// The palette-memory key of the file a panel shows.
pub fn panel_key(state: &AppState, panel: Focus) -> Option<String> {
    use crate::sprites::SpriteSource;
    match panel {
        Focus::Viewer => state
            .sprites
            .viewer
            .source
            .as_ref()
            .and_then(SpriteSource::memory_key),
        Focus::Editor => state
            .sprites
            .editor
            .as_ref()
            .and_then(|e| e.source.memory_key()),
        Focus::Converter => state
            .target_data(state.sprites.converter.target)
            .and_then(|d| d.source)
            .and_then(|s| s.memory_key()),
    }
}

fn palette_list(state: &AppState) -> Vec<PaletteChoice> {
    let extra = state
        .session
        .as_ref()
        .map(|s| s.palettes.clone())
        .unwrap_or_default();
    let mut list = choices(&extra);
    list.push(PaletteChoice::Grey);
    list
}

/// Palette picker and gamma, shared by the viewer, the editor and the
/// converter. A pick is remembered for the panel's file.
pub fn palette_controls(ui: &mut Ui, state: &mut AppState, panel: Focus) {
    let list = palette_list(state);
    let current = state.sprites.palette.choice.clone();
    let mut picked = None;
    ui.label("Palette");
    egui::ComboBox::from_id_salt(("sprite_palette", ui.id()))
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for c in list {
                if ui.selectable_label(c == current, c.label()).clicked() {
                    picked = Some(c);
                }
            }
        });
    if let Some(c) = picked {
        let key = panel_key(state, panel);
        state.pick_palette(c, key.as_deref());
    }
    ui.add(
        egui::DragValue::new(&mut state.sprites.palette.gamma)
            .range(0.5..=3.0)
            .speed(0.01)
            .prefix("gamma "),
    )
    .on_hover_text("Display only: gamma never changes the palette indices");
}

/// The badge on its own row of a panel header.
pub fn paint_badge_row(ui: &mut Ui, state: &mut AppState, panel: Focus) {
    palette_badge(ui, state, panel);
}

/// The palette badge of a panel's header: which palette is on and whether
/// it is the one remembered for the file. One click opens the palette list;
/// a remembered or folder palette that differs is offered as a button.
pub fn palette_badge(ui: &mut Ui, state: &mut AppState, panel: Focus) {
    use crate::sprites::palette_memory::Tone;
    let key = panel_key(state, panel);
    let badge = state.palette_badge(key.as_deref());
    let fill = match badge.tone {
        Tone::Good => Color32::from_rgb(38, 110, 52),
        Tone::Info => Color32::from_rgb(40, 82, 140),
        Tone::Warn => Color32::from_rgb(170, 110, 20),
        Tone::Danger => Color32::from_rgb(170, 36, 36),
    };
    let list = palette_list(state);
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        let button = egui::Button::new(
            egui::RichText::new(&badge.text)
                .color(Color32::WHITE)
                .strong(),
        )
        .fill(fill);
        let resp = ui
            .add(button)
            .on_hover_text("The palette this file is shown and saved with. Click to change it.");
        egui::Popup::menu(&resp).show(|ui| {
            for c in list {
                if ui.selectable_label(c == badge.current, c.label()).clicked() {
                    picked = Some(c);
                }
            }
        });
        if let Some(offer) = &badge.offer
            && ui
                .button(format!("Use {}", offer.label()))
                .on_hover_text("Switch to the palette this file (or folder) was used with")
                .clicked()
        {
            picked = Some(offer.clone());
        }
    });
    if let Some(c) = picked {
        state.pick_palette(c, key.as_deref());
    }
}

/// `label` of a colour for tooltips.
fn swatch_text(index: u8, rgb: [u8; 3]) -> String {
    format!("index {index}: #{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

// ───────────────────────── keyboard ─────────────────────────

fn pressed(ctx: &Context, mods: Modifiers, key: Key) -> bool {
    ctx.input_mut(|i| i.consume_key(mods, key))
}

/// Sprites-mode shortcuts (see the module docs). Skipped while typing or a
/// question is open.
pub fn shortcuts(ctx: &Context, state: &mut AppState) {
    if ctx.text_edit_focused() || state.sprites.modal_open() || state.edit.modal_open() {
        return;
    }
    let ctrl = Modifiers::COMMAND;
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let hot = state.sprites.hot.unwrap_or(Focus::Editor);
    if hot == Focus::Viewer || state.sprites.editor.is_none() {
        if pressed(ctx, Modifiers::NONE, Key::Space) {
            state.sprites.viewer.playing = !state.sprites.viewer.playing;
        }
        if pressed(ctx, Modifiers::NONE, Key::Comma) {
            state.sprites.viewer.step(-1);
        }
        if pressed(ctx, Modifiers::NONE, Key::Period) {
            state.sprites.viewer.step(1);
        }
        if pressed(ctx, ctrl, Key::S) && state.sprites.viewer.sprite().is_some() {
            state.begin_sprite_save(crate::sprites::Target::Viewer);
        }
        return;
    }
    // Editor.
    if pressed(ctx, ctrl_shift, Key::Z) || pressed(ctx, ctrl, Key::Y) {
        if let Some(e) = state.sprites.editor.as_mut() {
            e.redo();
        }
        return;
    }
    if pressed(ctx, ctrl, Key::Z) {
        if let Some(e) = state.sprites.editor.as_mut() {
            e.undo();
        }
        return;
    }
    if pressed(ctx, ctrl, Key::S) {
        state.begin_sprite_save(crate::sprites::Target::Editor);
        return;
    }
    if pressed(ctx, ctrl_shift, Key::V) {
        editor::paste_os_image(state);
        return;
    }
    // Ctrl+C/X/V arrive as events, not keys, on most platforms.
    let (copy, cut, paste) = ctx.input(|i| {
        let mut out = (false, false, false);
        for e in &i.events {
            match e {
                egui::Event::Copy => out.0 = true,
                egui::Event::Cut => out.1 = true,
                egui::Event::Paste(_) => out.2 = true,
                _ => {}
            }
        }
        out
    });
    let (copy, cut, paste) = (
        copy || pressed(ctx, ctrl, Key::C),
        cut || pressed(ctx, ctrl, Key::X),
        paste || pressed(ctx, ctrl, Key::V),
    );
    if copy {
        editor::copy(state);
    }
    if cut {
        editor::cut(state);
    }
    if paste {
        editor::paste(state);
    }
    let Some(e) = state.sprites.editor.as_mut() else {
        return;
    };
    let mut problem = None;
    if pressed(ctx, ctrl, Key::A) {
        e.select_all();
    }
    if pressed(ctx, ctrl, Key::D) {
        e.deselect();
    }
    if pressed(ctx, ctrl, Key::I) {
        e.invert_selection();
    }
    if pressed(ctx, Modifiers::NONE, Key::Delete) || pressed(ctx, Modifiers::NONE, Key::Backspace) {
        problem = e.clear_selection_pixels().err();
    }
    if pressed(ctx, Modifiers::NONE, Key::Escape) {
        if e.gesture.is_some() {
            e.cancel_gesture();
        } else {
            e.deselect();
        }
    }
    if pressed(ctx, Modifiers::NONE, Key::X) {
        e.swap_colors();
    }
    if pressed(ctx, Modifiers::NONE, Key::OpenBracket) {
        e.brush.size = e.brush.size.saturating_sub(1).max(1);
    }
    if pressed(ctx, Modifiers::NONE, Key::CloseBracket) {
        e.brush.size = (e.brush.size + 1).min(32);
    }
    if pressed(ctx, Modifiers::NONE, Key::Comma) {
        e.step_frame(-1);
    }
    if pressed(ctx, Modifiers::NONE, Key::Period) {
        e.step_frame(1);
    }
    let tool_keys = [
        (Key::B, 'B'),
        (Key::E, 'E'),
        (Key::L, 'L'),
        (Key::R, 'R'),
        (Key::C, 'C'),
        (Key::F, 'F'),
        (Key::K, 'K'),
        (Key::I, 'I'),
        (Key::G, 'G'),
        (Key::H, 'H'),
        (Key::M, 'M'),
        (Key::U, 'U'),
        (Key::Q, 'Q'),
        (Key::W, 'W'),
        (Key::V, 'V'),
        (Key::O, 'O'),
    ];
    for (key, c) in tool_keys {
        if pressed(ctx, Modifiers::NONE, key)
            && let Some(t) = crate::sprites::editor::PaintTool::from_key(c)
        {
            e.cancel_gesture();
            e.tool = t;
        }
    }
    if let Some(p) = problem {
        state.warn(p);
    }
}

// ───────────────────────── questions ─────────────────────────

pub fn dialogs(ctx: &Context, state: &mut AppState) {
    convert_palette::window(ctx, state);
    gif_dialog::window(ctx, state);
    save_prompt(ctx, state);
    palette_warning(ctx, state);
    dcc_warning(ctx, state);
    discard_prompt(ctx, state);
    quit_prompt(ctx, state);
}

fn save_prompt(ctx: &Context, state: &mut AppState) {
    let Some(prompt) = state.sprites.save_prompt.as_mut() else {
        return;
    };
    let profile = state
        .session
        .as_ref()
        .map_or_else(|| "(no profile)".to_owned(), |s| s.profile.id.clone());
    let palette_name = if prompt.converted {
        state
            .sprites
            .convert
            .result
            .as_ref()
            .map_or_else(String::new, |r| r.target.label())
    } else {
        state.sprites.palette.choice.label()
    };
    let mut save = false;
    let mut cancel = false;
    egui::Modal::new(Id::new("sprite_save_modal")).show(ctx, |ui| {
        ui.set_min_width(520.0);
        ui.heading(if prompt.converted {
            "Save the converted file to a game path"
        } else {
            "Save to game path"
        });
        if prompt.converted {
            ui.label(format!(
                "Writes the converted file as it is to capybara\\saves\\{profile}\\<game path>, \
                 lists it in CHANGED.txt and records a snapshot."
            ));
        } else {
            ui.label(format!(
                "Writes capybara\\saves\\{profile}\\<game path>, lists it in CHANGED.txt and \
                 records a snapshot. The extension (.dc6 or .dcc) picks the format."
            ));
        }
        ui.horizontal(|ui| {
            ui.label("Palette:");
            ui.strong(&palette_name);
        });
        ui.add_space(6.0);
        let edit = ui.add(
            egui::TextEdit::singleline(&mut prompt.text)
                .desired_width(f32::INFINITY)
                .hint_text("data\\global\\..."),
        );
        if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            save = true;
        }
        edit.request_focus();
        if let Some(e) = &prompt.error {
            ui.colored_label(Color32::from_rgb(235, 90, 90), e);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                save = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
        });
    });
    if cancel {
        state.sprites.save_prompt = None;
    } else if save && prompt.converted {
        let text = prompt.text.clone();
        let kind = state.sprites.convert.result.as_ref().map(|r| r.kind);
        let checked = kind
            .ok_or_else(|| "nothing was converted".to_owned())
            .and_then(|k| crate::sprites::convert_palette::check_converted_path(&text, k));
        match checked {
            Err(e) => {
                if let Some(p) = state.sprites.save_prompt.as_mut() {
                    p.error = Some(e);
                }
            }
            Ok(_) => {
                state.sprites.save_prompt = None;
                let _ = state.convert_save_game(&text);
            }
        }
    } else if save {
        let (target, text) = (prompt.target, prompt.text.clone());
        match state.check_save_path(target, &text) {
            Err(e) => {
                if let Some(p) = state.sprites.save_prompt.as_mut() {
                    p.error = Some(e);
                }
            }
            Ok(_) => {
                state.sprites.save_prompt = None;
                state.sprite_write_checked(target, crate::sprites::DccWrite::Game(text));
            }
        }
    }
}

/// "This file belongs to another palette": use it, keep the current one,
/// or cancel.
fn palette_warning(ctx: &Context, state: &mut AppState) {
    use crate::sprites::{PaletteAnswer, PendingWrite};
    let Some(w) = state.sprites.palette_warning.clone() else {
        return;
    };
    let (remembered, current) = (w.remembered.label(), w.current.label());
    let mut answer = None;
    let mut convert = false;
    egui::Modal::new(Id::new("sprite_palette_warning")).show(ctx, |ui| {
        ui.set_max_width(520.0);
        let what = match &w.pending {
            PendingWrite::Save { .. } => "Saving",
            PendingWrite::Import | PendingWrite::ImportFolder(_) => "Importing",
        };
        ui.heading(format!("{what} with {current}, but this file uses {remembered}"));
        if w.from_json {
            ui.label(format!(
                "The export's sprite.json says its colours are {remembered}; the palette now is {current}."
            ));
        } else {
            ui.label(format!(
                "{} was last used with {remembered}; the palette now is {current}.",
                w.key
            ));
        }
        ui.label(
            "Palette indices mean different colours in different palettes, so art made under the \
             wrong one shows the wrong colours in game.",
        );
        if matches!(w.pending, PendingWrite::Save { .. }) {
            ui.weak(format!(
                "If the art was drawn under {current} and must look the same under {remembered}, \
                 use Convert to palette instead."
            ));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(format!("Use {remembered}")).clicked() {
                answer = Some(PaletteAnswer::UseRemembered);
            }
            if ui.button(format!("Keep {current}")).clicked() {
                answer = Some(PaletteAnswer::KeepCurrent);
            }
            if matches!(w.pending, PendingWrite::Save { .. })
                && ui.button("Convert to palette...").clicked()
            {
                convert = true;
                answer = Some(PaletteAnswer::Cancel);
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                answer = Some(PaletteAnswer::Cancel);
            }
        });
    });
    if let Some(a) = answer {
        state.answer_palette_warning(a);
    }
    if convert {
        let target = match &w.pending {
            PendingWrite::Save { target, .. } => *target,
            _ => crate::sprites::Target::Viewer,
        };
        state.convert_open_target(target, w.current.clone(), w.remembered.clone());
    }
}

/// "N cells hold more than 4 values": reduce and save, look first, or not.
fn dcc_warning(ctx: &Context, state: &mut AppState) {
    use crate::sprites::{DccAnswer, DccWrite, Target};
    let Some(w) = state.sprites.dcc_warning.clone() else {
        return;
    };
    let mut answer = None;
    egui::Modal::new(Id::new("sprite_dcc_warning")).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading("Reduce crowded DCC cells?");
        let what = match &w.write {
            DccWrite::Game(p) => p.clone(),
            DccWrite::Export(p) => p.display().to_string(),
        };
        ui.label(format!(
            "{} 4x4 cell(s) hold more than 4 values. A DCC cell can store 4 at most, so writing {what} merges the nearest colours in those cells.",
            w.cells
        ));
        if w.target == Target::Editor {
            ui.weak("\"Reduce and save\" changes the editor's frames first (one undo step).");
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Reduce and save").clicked() {
                answer = Some(DccAnswer::ReduceAndSave);
            }
            if ui
                .button("Open the cell check")
                .on_hover_text("Lists the cells in the editor so you can fix them by hand")
                .clicked()
            {
                answer = Some(DccAnswer::OpenCellCheck);
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                answer = Some(DccAnswer::Cancel);
            }
        });
    });
    if let Some(a) = answer {
        state.answer_dcc_warning(a);
    }
}

fn discard_prompt(ctx: &Context, state: &mut AppState) {
    if state.sprites.pending_editor.is_none() {
        return;
    }
    let mut answer = None;
    egui::Modal::new(Id::new("sprite_discard_modal")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading("Discard the unsaved sprite edits?");
        ui.label(
            "The editor holds changes that were not saved. Opening another sprite replaces them.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Discard and open").clicked() {
                answer = Some(true);
            }
            if ui.button("Keep editing").clicked() {
                answer = Some(false);
            }
        });
    });
    if let Some(discard) = answer {
        state.answer_pending_editor(discard);
    }
}

fn quit_prompt(ctx: &Context, state: &mut AppState) {
    if !state.sprites.quit_prompt {
        return;
    }
    let mut answer = None;
    egui::Modal::new(Id::new("sprite_quit_modal")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading("The sprite editor has unsaved changes");
        ui.label("Quit anyway and lose them?");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Discard and quit").clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    match answer {
        Some(true) => {
            state.sprites.quit_prompt = false;
            state.sprites.quit_confirmed = true;
            state.request_quit();
        }
        Some(false) => state.sprites.quit_prompt = false,
        None => {}
    }
}
