//! The map list (left column): every `.ds1` among the known names, grouped
//! by folder (`act1/town`) with counts, filtered and virtualised. Click a
//! folder to open or close it, a map to open it. While a filter is typed
//! every folder is open.

use bevy_egui::egui::{self, Align2, FontId, Pos2, Rect, Sense, Ui, Vec2};

use super::theme::{self, icon};
use crate::state::AppState;

const ROW: f32 = 24.0;
const PREFIX: &str = "data\\global\\tiles\\";

/// Maps grouped by folder, for one filter and session.
pub struct Tree {
    key: (String, u64),
    /// (folder, indices into `Session::maps`), folders sorted.
    folders: Vec<(String, Vec<usize>)>,
}

enum Row {
    Folder(usize),
    Map(usize, usize),
}

/// `act1/town` and `townn1.ds1` of a game path.
fn split(path: &str) -> (String, &str) {
    let short = path.strip_prefix(PREFIX).unwrap_or(path);
    match short.rfind('\\') {
        Some(i) => (short[..i].replace('\\', "/"), &short[i + 1..]),
        None => (String::new(), short),
    }
}

fn build(state: &AppState) -> Tree {
    let mut folders: Vec<(String, Vec<usize>)> = Vec::new();
    if let Some(session) = &state.session {
        let mut by: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
        for &i in &state.maps_list.filtered {
            if let Some(p) = session.maps.get(i) {
                by.entry(split(p.as_str()).0).or_default().push(i);
            }
        }
        folders = by.into_iter().collect();
    }
    Tree {
        key: (state.maps_list.filter.clone(), state.session_epoch),
        folders,
    }
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    let fonts = state.shell.fonts_ready();
    state.refresh_maps_filter();
    let total = state.session.as_ref().map_or(0, |x| x.maps.len());

    // Header: MAPS, count, fold.
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(icon::LIST_TREE.to_string())
                .font(theme::icon_font(14.0, fonts))
                .color(s.muted),
        );
        ui.label(egui::RichText::new("MAPS").size(12.0).color(s.heading));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if super::shell::icon_button(ui, state, icon::PANEL_LEFT, "Fold the map list").clicked()
            {
                super::commands::run(state, "view.foldLeft");
            }
            ui.label(
                egui::RichText::new(group_digits(total))
                    .size(12.0)
                    .color(s.muted),
            );
        });
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.add(
            egui::TextEdit::singleline(&mut state.maps_list.filter)
                .hint_text("Filter maps, e.g. act1/town")
                .desired_width(ui.available_width() - 10.0),
        );
    });
    state.refresh_maps_filter();
    ui.add_space(6.0);

    if state.session.is_none() {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            match &state.loading {
                Some(l) => ui.weak(format!("Loading {}: {}...", l.profile_id, l.stage)),
                None => ui.weak("Pick a profile: Home > Profile."),
            };
        });
        return;
    }

    let key = (state.maps_list.filter.clone(), state.session_epoch);
    if state.shell.map_tree.as_ref().is_none_or(|t| t.key != key) {
        state.shell.map_tree = Some(build(state));
    }
    let filtering = !state.maps_list.filter.trim().is_empty();
    let active_path = state.active_doc().map(|d| d.path.clone());
    let active_folder = active_path.as_ref().map(|p| split(p.as_str()).0);
    // A newly shown map opens its folder once.
    let active_id = state.active_doc().map(|d| d.id);
    if active_id != state.shell.revealed_doc {
        state.shell.revealed_doc = active_id;
        if let Some(f) = &active_folder {
            state.shell.open_folders.insert(f.clone());
        }
    }
    let Some(tree) = state.shell.map_tree.take() else {
        return;
    };
    let mut rows: Vec<Row> = Vec::new();
    for (fi, (folder, maps)) in tree.folders.iter().enumerate() {
        rows.push(Row::Folder(fi));
        let open = filtering || state.shell.open_folders.contains(folder);
        if open {
            rows.extend((0..maps.len()).map(|mi| Row::Map(fi, mi)));
        }
    }
    if rows.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.weak("No map matches the filter.");
        });
    }

    let mut toggle: Option<String> = None;
    let mut to_open = None;
    egui::ScrollArea::vertical()
        .id_salt("map_tree")
        .auto_shrink([false, false])
        .show_rows(ui, ROW, rows.len(), |ui, range| {
            for row in &rows[range] {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                let painter = ui.painter();
                match row {
                    Row::Folder(fi) => {
                        let (folder, maps) = &tree.folders[*fi];
                        let open = filtering || state.shell.open_folders.contains(folder);
                        if resp.hovered() {
                            painter.rect_filled(rect, 0.0, s.hover);
                        }
                        painter.text(
                            Pos2::new(rect.left() + 16.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            if open {
                                icon::CHEVRON_DOWN
                            } else {
                                icon::CHEVRON_RIGHT
                            },
                            theme::icon_font(11.0, fonts),
                            s.muted,
                        );
                        let name = if folder.is_empty() { "(top)" } else { folder };
                        let bold = active_folder.as_deref() == Some(folder.as_str());
                        painter.text(
                            Pos2::new(rect.left() + 26.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            name,
                            FontId::proportional(13.0),
                            if bold { s.text } else { s.heading },
                        );
                        painter.text(
                            Pos2::new(rect.right() - 12.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            maps.len().to_string(),
                            FontId::proportional(11.5),
                            s.muted,
                        );
                        if resp.clicked() && !filtering {
                            toggle = Some(folder.clone());
                        }
                    }
                    Row::Map(fi, mi) => {
                        let index = tree.folders[*fi].1[*mi];
                        let Some(path) = state.session.as_ref().and_then(|x| x.maps.get(index))
                        else {
                            continue;
                        };
                        let current = active_path.as_ref() == Some(path);
                        if current {
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
                            Pos2::new(rect.left() + 30.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            split(path.as_str()).1,
                            FontId::proportional(13.0),
                            if current { s.accent_text } else { s.text },
                        );
                        let path = path.clone();
                        let source = state
                            .maps_list
                            .source_cache
                            .get(&index)
                            .map(|(cat, name)| format!("\nfrom {name} ({})", cat.label()));
                        if source.is_none()
                            && resp.hovered()
                            && let Some(info) = state.source_of(&path)
                        {
                            state
                                .maps_list
                                .source_cache
                                .insert(index, (info.category, info.layer_name));
                        }
                        let resp =
                            resp.on_hover_text(format!("{path}{}", source.unwrap_or_default()));
                        if resp.clicked() {
                            to_open = Some(path);
                        }
                    }
                }
            }
        });
    state.shell.map_tree = Some(tree);
    if let Some(folder) = toggle
        && !state.shell.open_folders.remove(&folder)
    {
        state.shell.open_folders.insert(folder);
    }
    if let Some(path) = to_open {
        state.open_map(&path);
    }
}

/// `2276` -> `2,276`, as ds1-studio shows counts.
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_folder_and_name() {
        assert_eq!(
            split(r"data\global\tiles\act1\town\townn1.ds1"),
            ("act1/town".to_owned(), "townn1.ds1")
        );
        assert_eq!(split("x.ds1"), (String::new(), "x.ds1"));
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(group_digits(2276), "2,276");
        assert_eq!(group_digits(13), "13");
        assert_eq!(group_digits(1_000_000), "1,000,000");
    }
}
