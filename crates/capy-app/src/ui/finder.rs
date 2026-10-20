//! Ctrl+K: the command finder. Type part of a command's name, Up/Down to
//! pick, Enter to run, Esc to close. Commands that cannot run right now are
//! listed greyed out. The last row searches every asset with the same text
//! (Capybara's query language, as in the Tiles filter box).

use bevy_egui::egui::{self, Align2, FontId, Id, Key, Modifiers, Sense, Vec2};

use super::commands::{self, COMMANDS, Command};
use super::theme;
use crate::state::AppState;

const ROW: f32 = 28.0;

/// The available commands matching `query`, best first: label prefix, then
/// word prefix, then anywhere in the label, group or id.
pub fn matches(query: &str) -> Vec<&'static Command> {
    let q = query.trim().to_ascii_lowercase();
    let mut scored: Vec<(u8, usize, &'static Command)> = COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, c)| c.available)
        .filter_map(|(i, c)| {
            let label = c.label.to_ascii_lowercase();
            let score = if q.is_empty() || label.starts_with(&q) {
                0
            } else if label.split([' ', '/', '(']).any(|w| w.starts_with(&q)) {
                1
            } else if label.contains(&q)
                || c.group.to_ascii_lowercase().contains(&q)
                || c.id.to_ascii_lowercase().contains(&q)
            {
                2
            } else {
                return None;
            };
            Some((score, i, c))
        })
        .collect();
    scored.sort_by_key(|(score, i, _)| (*score, *i));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if !state.shell.finder.open {
        return;
    }
    let (delta, enter, escape) = ctx.input_mut(|i| {
        let mut d = 0i64;
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            d += 1;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            d -= 1;
        }
        (
            d,
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape)
                || i.consume_key(Modifiers::COMMAND, Key::K),
        )
    });
    let items = matches(&state.shell.finder.query);
    // One extra row: search every asset.
    let n = items.len() + 1;
    {
        let f = &mut state.shell.finder;
        let cur = i64::try_from(f.selected).unwrap_or(0);
        let max = i64::try_from(n - 1).unwrap_or(0);
        f.selected = usize::try_from((cur + delta).clamp(0, max)).unwrap_or(0);
    }
    let selected = state.shell.finder.selected;
    let s = state.shell.shades;
    let fonts = state.shell.fonts_ready();
    let mut run: Option<&'static str> = None;
    if enter {
        run = Some(items.get(selected).map_or("app.assetSearch", |c| c.id));
    }
    let width = (ctx.content_rect().width() - 80.0).clamp(320.0, 620.0);
    let modal = egui::Modal::new(Id::new("command_finder")).show(ctx, |ui| {
        ui.set_width(width);
        let focus = std::mem::take(&mut state.shell.finder.focus);
        let edit = ui.add(
            egui::TextEdit::singleline(&mut state.shell.finder.query)
                .id(Id::new("command_finder_query"))
                .hint_text("Find a command...")
                .desired_width(f32::INFINITY),
        );
        if focus || !edit.has_focus() {
            edit.request_focus();
        }
        if edit.changed() {
            state.shell.finder.selected = 0;
        }
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() * 0.55).max(200.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, c) in items.iter().enumerate() {
                    let enabled = commands::enabled(state, c.id);
                    let key = state.shell.key_of(c.id);
                    let (rect, resp) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                    if i == selected {
                        ui.painter().rect_filled(rect, 4.0, s.accent_dim);
                        if delta != 0 {
                            resp.scroll_to_me(None);
                        }
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 4.0, s.hover);
                    }
                    let fg = if enabled { s.text } else { s.muted };
                    ui.painter().text(
                        rect.left_center() + Vec2::new(14.0, 0.0),
                        Align2::CENTER_CENTER,
                        c.icon,
                        theme::icon_font(14.0, fonts),
                        if enabled { s.heading } else { s.muted },
                    );
                    ui.painter().text(
                        rect.left_center() + Vec2::new(30.0, 0.0),
                        Align2::LEFT_CENTER,
                        c.label,
                        FontId::proportional(13.0),
                        fg,
                    );
                    ui.painter().text(
                        rect.right_center() - Vec2::new(8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        if key.is_empty() {
                            c.group.to_owned()
                        } else {
                            format!("{key}    {}", c.group)
                        },
                        FontId::proportional(11.5),
                        s.muted,
                    );
                    if resp.clicked() {
                        run = Some(c.id);
                    }
                }
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                if selected == items.len() {
                    ui.painter().rect_filled(rect, 4.0, s.accent_dim);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, s.hover);
                }
                let q = state.shell.finder.query.trim();
                ui.painter().text(
                    rect.left_center() + Vec2::new(14.0, 0.0),
                    Align2::CENTER_CENTER,
                    theme::icon::SCAN_SEARCH,
                    theme::icon_font(14.0, fonts),
                    s.heading,
                );
                ui.painter().text(
                    rect.left_center() + Vec2::new(30.0, 0.0),
                    Align2::LEFT_CENTER,
                    if q.is_empty() {
                        "Search every asset...".to_owned()
                    } else {
                        format!("Search every asset for \u{201c}{q}\u{201d}")
                    },
                    FontId::proportional(13.0),
                    s.text,
                );
                if resp.clicked() {
                    run = Some("app.assetSearch");
                }
            });
        ui.add_space(4.0);
        ui.weak("Up/Down: pick   Enter: run   Esc: close");
    });
    if let Some(id) = run {
        if id != "app.assetSearch" {
            state.shell.finder.open = false;
        }
        if commands::run(state, id) {
            state.shell.ran.push(id);
        }
    } else if escape || modal.should_close() {
        state.shell.finder.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_label_prefix_first_and_hides_unavailable() {
        let hits = matches("undo");
        assert_eq!(hits.first().map(|c| c.id), Some("edit.undo"));
        assert!(matches("npc paths").is_empty(), "not in this version");
        assert!(matches("walkability").iter().any(|c| c.id == "mode.walk"));
        let all = matches("");
        assert!(all.iter().all(|c| c.available));
        assert!(matches("paint").iter().any(|c| c.id == "tool.paint"));
        assert!(
            matches("tools").iter().any(|c| c.id == "tool.select"),
            "by group"
        );
    }
}
