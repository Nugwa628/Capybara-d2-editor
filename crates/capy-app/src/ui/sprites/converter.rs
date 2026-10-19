//! The Converter panel: save to a game path, import images, the exports and
//! batch conversion.

use bevy_egui::egui::{self, Color32, RichText, Ui};
use capy_sprite::{Dither, SpriteFormat};

use super::palette_controls;
use crate::sprites::Target;
use crate::sprites::convert::{Layout, MatchSettings};
use crate::sprites::dialog::Purpose;
use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            target(ui, state);
            ui.separator();
            egui::CollapsingHeader::new(RichText::new("Import images").strong())
                .default_open(true)
                .show(ui, |ui| import(ui, state));
            egui::CollapsingHeader::new(RichText::new("Export to...").strong())
                .default_open(true)
                .show(ui, |ui| export(ui, state));
            egui::CollapsingHeader::new(RichText::new("Batch conversion").strong())
                .default_open(true)
                .show(ui, |ui| batch(ui, state));
        });
}

fn target(ui: &mut Ui, state: &mut AppState) {
    let has_editor = state.sprites.editor.is_some();
    if !has_editor {
        state.sprites.converter.target = Target::Viewer;
    }
    ui.horizontal(|ui| {
        ui.label("Work on");
        let t = &mut state.sprites.converter.target;
        ui.radio_value(t, Target::Viewer, "Viewer sprite");
        ui.add_enabled_ui(has_editor, |ui| {
            ui.radio_value(t, Target::Editor, "Editor sprite");
        });
    });
    let target = state.sprites.converter.target;
    let summary = state.target_data(target).map(|d| {
        format!(
            "{}: {} directions x {} frames, {}",
            d.source.map_or_else(|| "?".to_owned(), |s| s.label()),
            d.sprite.directions.len(),
            d.sprite.frames_per_direction(),
            d.format.extension().to_ascii_uppercase()
        )
    });
    match summary {
        Some(s) => ui.weak(s),
        None => ui.weak("Nothing to convert yet: open or import a DC6/DCC (COFs are viewed only)."),
    };
    ui.horizontal_wrapped(|ui| palette_controls(ui, state));
    ui.horizontal(|ui| {
        let ok = state.target_data(target).is_some() && state.session.is_some();
        if ui
            .add_enabled(ok, egui::Button::new("Save to game path..."))
            .on_hover_text(r"capybara\saves\<profile>\<game path>, CHANGED.txt and a snapshot")
            .on_disabled_hover_text("Needs a profile and a DC6/DCC")
            .clicked()
        {
            state.begin_sprite_save(target);
        }
    });
}

fn matching(ui: &mut Ui, m: &mut MatchSettings, salt: &str) {
    ui.horizontal(|ui| {
        ui.label("Dither");
        egui::ComboBox::from_id_salt(("dither", salt))
            .selected_text(match m.dither {
                Dither::None => "None",
                Dither::FloydSteinberg => "Floyd-Steinberg",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut m.dither, Dither::None, "None");
                ui.selectable_value(&mut m.dither, Dither::FloydSteinberg, "Floyd-Steinberg");
            });
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut m.transparent_key, "Transparent colour");
        ui.add_enabled_ui(m.transparent_key, |ui| {
            ui.color_edit_button_srgb(&mut m.transparent_rgb);
        });
    });
    ui.horizontal(|ui| {
        ui.label("Alpha below");
        ui.add(egui::Slider::new(&mut m.alpha_threshold, 0..=255))
            .on_hover_text("Pixels with less alpha are transparent");
    });
    ui.checkbox(
        &mut m.protect,
        "Protect transparent (opaque never becomes index 0)",
    );
}

fn format_radio(ui: &mut Ui, f: &mut SpriteFormat) {
    ui.radio_value(f, SpriteFormat::Dcc, "DCC");
    ui.radio_value(f, SpriteFormat::Dc6, "DC6");
}

fn import(ui: &mut Ui, state: &mut AppState) {
    let busy = state.sprites.dialogs.is_open();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new("Pick images..."))
            .on_hover_text("PNG, BMP, GIF or PCX; several files are frames in name order")
            .clicked()
        {
            state.sprite_dialog(Purpose::ImportImages);
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Import folder..."))
            .on_hover_text("A folder exported with sprite.json comes back with exact offsets")
            .clicked()
        {
            state.sprite_dialog(Purpose::ImportFolder);
        }
    });
    let s = &mut state.sprites.converter.import;
    match s.files.as_slice() {
        [] => {
            ui.weak("No images picked.");
        }
        files => {
            let names: Vec<String> = files
                .iter()
                .take(4)
                .map(|f| {
                    f.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect();
            let more = files.len().saturating_sub(names.len());
            ui.weak(format!(
                "{} image(s): {}{}",
                files.len(),
                names.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ));
        }
    }
    ui.horizontal(|ui| {
        ui.label("Layout");
        ui.radio_value(&mut s.layout, Layout::Frames, "One image per frame");
        ui.radio_value(&mut s.layout, Layout::Sheet, "Sheet (rows x columns)");
    });
    ui.horizontal(|ui| {
        let (a, b) = match s.layout {
            Layout::Frames => ("Directions", "Frames (0 = auto)"),
            Layout::Sheet => ("Rows (directions)", "Columns (frames)"),
        };
        ui.label(a);
        ui.add(egui::DragValue::new(&mut s.directions).range(1..=64));
        ui.label(b);
        ui.add(egui::DragValue::new(&mut s.frames).range(0..=4096));
    });
    matching(ui, &mut s.matching, "import");
    ui.checkbox(&mut s.split, "Split frames larger than 256x256");
    ui.horizontal(|ui| {
        ui.label("Encode as");
        format_radio(ui, &mut s.format);
    });
    ui.checkbox(
        &mut state.sprites.converter.edit_after_import,
        "Open in the editor too",
    );
    let ready = !state.sprites.converter.import.files.is_empty();
    if ui.add_enabled(ready, egui::Button::new("Import")).clicked() {
        let _ = state.sprite_import();
    }
    ui.weak("The palette above is the one colours are matched against.");
}

fn export(ui: &mut Ui, state: &mut AppState) {
    let target = state.sprites.converter.target;
    let have = state.target_data(target).is_some();
    let busy = state.sprites.dialogs.is_open();
    let ok = have && !busy;
    let rows: [(Purpose, &str, &str); 5] = [
        (
            Purpose::ExportFrames,
            "Frames + sprite.json...",
            "One PNG per frame, exact offsets in sprite.json",
        ),
        (
            Purpose::ExportSheet,
            "Sprite sheet...",
            "One PNG: rows are directions, columns frames",
        ),
        (
            Purpose::ExportGif,
            "Animated GIF...",
            "The current direction at the viewer's speed",
        ),
        (
            Purpose::ExportPng,
            "Indexed PNG...",
            "The current frame with the palette embedded (GIMP/Photoshop round trips)",
        ),
        (
            Purpose::ExportSprite,
            "Sprite file (DC6/DCC)...",
            "Encoded with the format of the file name",
        ),
    ];
    for (purpose, label, hint) in rows {
        if ui
            .add_enabled(ok, egui::Button::new(label))
            .on_hover_text(hint)
            .clicked()
        {
            state.sprite_dialog(purpose);
        }
    }
    ui.horizontal(|ui| {
        ui.label("Palette");
        for (purpose, label) in [
            (Purpose::ExportPaletteGpl, ".gpl"),
            (Purpose::ExportPaletteAct, ".act"),
            (Purpose::ExportPalettePal, ".pal"),
        ] {
            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                state.sprite_dialog(purpose);
            }
        }
    });
    ui.weak("Exports go to the folder you pick; nothing else is written outside capybara\\.");
}

fn batch(ui: &mut Ui, state: &mut AppState) {
    let busy = state.sprites.dialogs.is_open();
    let running = state.sprites.converter.batch_task.is_some();
    ui.add_enabled_ui(!running, |ui| {
        let b = &state.sprites.converter.batch;
        let input = b
            .input
            .as_ref()
            .map_or("(none)".to_owned(), |p| p.display().to_string());
        let output = b
            .output
            .as_ref()
            .map_or("(none)".to_owned(), |p| p.display().to_string());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Input folder..."))
                .clicked()
            {
                state.sprite_dialog(Purpose::BatchInput);
            }
            ui.weak(input);
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Output folder..."))
                .clicked()
            {
                state.sprite_dialog(Purpose::BatchOutput);
            }
            ui.weak(output);
        });
        let b = &mut state.sprites.converter.batch;
        ui.horizontal(|ui| {
            ui.radio_value(&mut b.to_sprite, false, "Sprites to PNG frames");
            ui.radio_value(&mut b.to_sprite, true, "Images to sprites");
        });
        if b.to_sprite {
            ui.horizontal(|ui| {
                ui.label("Encode as");
                format_radio(ui, &mut b.format);
            });
            matching(ui, &mut b.matching, "batch");
            ui.checkbox(&mut b.split, "Split images larger than 256x256");
        }
        ui.checkbox(&mut b.recursive, "Include sub-folders");
        ui.checkbox(&mut b.overwrite, "Overwrite existing outputs");
        let ready = b.input.is_some() && b.output.is_some();
        if ui
            .add_enabled(ready, egui::Button::new("Start batch"))
            .clicked()
        {
            state.start_batch();
        }
    });
    if let Some(task) = &state.sprites.converter.batch_task {
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
    } else {
        if let Some((cancelled, text)) = &state.sprites.converter.batch_outcome {
            let color = if *cancelled {
                Color32::from_rgb(230, 150, 60)
            } else {
                Color32::from_rgb(110, 200, 110)
            };
            ui.colored_label(color, text);
        }
        ui.colored_label(Color32::from_gray(150), "The report goes to the Log panel.");
    }
}
