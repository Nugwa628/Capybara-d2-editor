//! The Sprites list: every `.dc6`/`.dcc`/`.cof`/`.dt1` among the profile's
//! known names as a folder tree (see `sprites::tree`), filtered and
//! virtualised, plus "Open file..." for loose files.

use bevy_egui::egui::{self, Align2, FontId, Pos2, Rect, Sense, Ui, Vec2};
use d2_assets::GamePath;

use crate::sprites::dialog::Purpose;
use crate::sprites::tree;
use crate::state::AppState;
use crate::ui::theme::{self, icon};

const ROW: f32 = 22.0;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !state.sprites.dialogs.is_open(),
                egui::Button::new("Open file..."),
            )
            .on_hover_text("A DC6, DCC, COF or DT1 anywhere on disk (opened read-only)")
            .clicked()
        {
            state.sprite_dialog(Purpose::OpenSprite);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Filter");
        ui.add(
            egui::TextEdit::singleline(&mut state.sprites.list.filter)
                .hint_text("substring, e.g. monsters\\zm")
                .desired_width(f32::INFINITY),
        );
    });
    ui.horizontal(|ui| {
        let l = &mut state.sprites.list;
        ui.checkbox(&mut l.show_dc6, "DC6");
        ui.checkbox(&mut l.show_dcc, "DCC");
        ui.checkbox(&mut l.show_cof, "COF");
        ui.checkbox(&mut l.show_dt1, "DT1");
    });
    state.refresh_sprite_filter();

    let Some(session) = &state.session else {
        ui.separator();
        match &state.loading {
            Some(l) => ui.label(format!("Loading {}: {}...", l.profile_id, l.stage)),
            None => ui.label("Pick a profile in the top bar, or open a file."),
        };
        return;
    };
    let total = session.sprites.len();
    let shown = state.sprites.list.filtered.len();
    let filtering = !state.sprites.list.filter.trim().is_empty();
    let s = state.shell.shades;
    let fonts = state.shell.fonts_ready();

    // Rebuild the tree when the filter, the toggles or the session changed.
    let l = &mut state.sprites.list;
    let key = (
        l.filter.clone(),
        [l.show_dc6, l.show_dcc, l.show_cof, l.show_dt1],
        state.session_epoch,
    );
    if l.tree.key.as_ref() != Some(&key) {
        l.tree.tree = tree::build(&session.sprites, &l.filtered);
        l.tree.key = Some(key);
    }
    let current = state
        .sprites
        .viewer
        .source
        .as_ref()
        .and_then(|s| s.game_path().cloned());
    // A newly opened sprite opens its folders once.
    if l.tree.revealed != current {
        if let Some(p) = &current {
            l.tree.open.extend(tree::ancestors(p.as_str()));
        }
        l.tree.revealed = current.clone();
    }

    ui.horizontal(|ui| {
        ui.weak(format!("{shown} of {total} sprites"));
        if ui.small_button("Expand all").clicked() {
            l.tree.open = l
                .tree
                .tree
                .nodes
                .iter()
                .filter_map(|n| match n {
                    tree::Node::Folder { key, .. } => Some(key.clone()),
                    tree::Node::File { .. } => None,
                })
                .collect();
        }
        if ui.small_button("Collapse all").clicked() {
            l.tree.open.clear();
        }
    });
    ui.separator();

    let tr = &l.tree;
    let rows = tree::rows(&tr.tree, &tr.open, filtering);
    if rows.is_empty() {
        ui.weak("No sprite matches the filter.");
    }
    let mut toggle: Option<String> = None;
    let mut to_open: Option<GamePath> = None;
    egui::ScrollArea::vertical()
        .id_salt("sprite_tree")
        .auto_shrink([false, false])
        .show_rows(ui, ROW, rows.len(), |ui, range| {
            for row in &rows[range] {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                let painter = ui.painter();
                let indent = 8.0 + row.depth as f32 * 14.0;
                match &tr.tree.nodes[row.node] {
                    tree::Node::Folder {
                        label, key, count, ..
                    } => {
                        let open = filtering || tr.open.contains(key);
                        if resp.hovered() {
                            painter.rect_filled(rect, 0.0, s.hover);
                        }
                        painter.text(
                            Pos2::new(rect.left() + indent + 6.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            if open {
                                icon::CHEVRON_DOWN
                            } else {
                                icon::CHEVRON_RIGHT
                            },
                            theme::icon_font(11.0, fonts),
                            s.muted,
                        );
                        painter.text(
                            Pos2::new(rect.left() + indent + 16.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(13.0),
                            s.heading,
                        );
                        painter.text(
                            Pos2::new(rect.right() - 12.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            count.to_string(),
                            FontId::proportional(11.5),
                            s.muted,
                        );
                        if resp.clicked() && !filtering {
                            toggle = Some(key.clone());
                        }
                    }
                    tree::Node::File { sprite, name } => {
                        let Some(path) = session.sprites.get(*sprite) else {
                            continue;
                        };
                        let selected = current.as_ref() == Some(path);
                        if selected {
                            painter.rect_filled(rect, 0.0, s.accent_dim);
                            painter.rect_filled(
                                Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
                                0.0,
                                s.accent,
                            );
                        } else if resp.hovered() {
                            painter.rect_filled(rect, 0.0, s.hover);
                        }
                        painter.text(
                            Pos2::new(rect.left() + indent + 16.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            name,
                            FontId::proportional(13.0),
                            if selected { s.accent_text } else { s.text },
                        );
                        painter.text(
                            Pos2::new(rect.right() - 12.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            path.extension().unwrap_or("").to_ascii_uppercase(),
                            FontId::proportional(10.5),
                            s.muted,
                        );
                        if resp.on_hover_text(path.as_str()).clicked() {
                            to_open = Some(path.clone());
                        }
                    }
                }
            }
        });
    if let Some(key) = toggle {
        let open = &mut state.sprites.list.tree.open;
        if !open.remove(&key) {
            open.insert(key);
        }
    }
    if let Some(path) = to_open {
        let _ = state.open_sprite(&path);
    }
}
