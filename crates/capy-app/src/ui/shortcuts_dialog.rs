//! Help > Shortcuts: every command and its key, grouped as in ds1-studio.
//! Click a key to change it (then press the new key; Esc cancels), "Clear"
//! removes it, "Reset all" restores the defaults. A key taken by another
//! command moves to the new one. Commands that come in a later version are
//! listed with their reserved key, greyed out.

use bevy_egui::egui::{self, Id, Key, RichText};

use super::commands::{self, COMMANDS, Combo};
use crate::state::AppState;

/// The key pressed this frame, if any (Escape alone cancels).
fn captured(ctx: &egui::Context) -> Option<Option<Combo>> {
    ctx.input_mut(|i| {
        let mut out = None;
        i.events.retain(|e| {
            if out.is_some() {
                return true;
            }
            if let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = e
            {
                if *key == Key::Escape && modifiers.is_none() {
                    out = Some(None);
                } else if commands::bindable(*key) {
                    out = Some(Some(Combo::of_event(*key, *physical_key, *modifiers)));
                }
                return false;
            }
            true
        });
        out
    })
}

fn save(state: &mut AppState) {
    state.config.ui.keys = state.shell.bindings.changes();
    state.config_dirty = true;
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if !state.shell.shortcuts.open {
        state.shell.shortcuts.capturing = None;
        return;
    }
    if let Some(id) = state.shell.shortcuts.capturing
        && let Some(answer) = captured(ctx)
    {
        if let Some(combo) = answer {
            state.shell.bindings.bind(id, Some(combo));
            save(state);
        }
        state.shell.shortcuts.capturing = None;
    }
    let mut open = true;
    egui::Window::new("Shortcuts")
        .id(Id::new("shortcuts_window"))
        .open(&mut open)
        .default_size([520.0, 560.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .show(ctx, |ui| body(ui, state));
    if !open {
        state.shell.shortcuts.open = false;
        state.shell.shortcuts.capturing = None;
    }
}

fn body(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.shell.shortcuts.filter)
                .hint_text("Filter")
                .desired_width(220.0),
        );
        if ui
            .button("Reset all")
            .on_hover_text("Every command back to its default key")
            .clicked()
        {
            state.shell.bindings = commands::Bindings::new(&Default::default());
            save(state);
        }
    });
    ui.weak("Click a key to change it, then press the new key (Esc cancels).");
    ui.separator();
    let filter = state.shell.shortcuts.filter.to_ascii_lowercase();
    let muted = state.shell.shades.muted;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut groups: Vec<&str> = Vec::new();
            for c in COMMANDS {
                if !groups.contains(&c.group) {
                    groups.push(c.group);
                }
            }
            let ordered = groups
                .into_iter()
                .flat_map(|g| COMMANDS.iter().filter(move |c| c.group == g));
            let mut group = "";
            egui::Grid::new("shortcuts_grid")
                .num_columns(3)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for c in ordered {
                        let key = state.shell.key_of(c.id);
                        if !filter.is_empty()
                            && !c.label.to_ascii_lowercase().contains(&filter)
                            && !key.to_ascii_lowercase().contains(&filter)
                        {
                            continue;
                        }
                        if c.group != group {
                            group = c.group;
                            ui.label(RichText::new(group).strong());
                            ui.end_row();
                        }
                        let label = if c.available {
                            RichText::new(c.label)
                        } else {
                            RichText::new(format!("{} (coming later)", c.label)).color(muted)
                        };
                        ui.label(label);
                        let capturing = state.shell.shortcuts.capturing == Some(c.id);
                        let text = if capturing {
                            "press a key...".to_owned()
                        } else if key.is_empty() {
                            "\u{2014}".to_owned()
                        } else {
                            key
                        };
                        if ui
                            .add_enabled(c.available, egui::Button::new(text).selected(capturing))
                            .on_hover_text("Change the key")
                            .clicked()
                        {
                            state.shell.shortcuts.capturing = Some(c.id);
                        }
                        if ui
                            .add_enabled(
                                c.available && state.shell.bindings.get(c.id).is_some(),
                                egui::Button::new("Clear").small(),
                            )
                            .clicked()
                        {
                            state.shell.bindings.bind(c.id, None);
                            save(state);
                        }
                        ui.end_row();
                    }
                });
        });
}
