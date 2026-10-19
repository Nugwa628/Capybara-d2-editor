//! Maps panel: every `.ds1` among the known names, filtered and virtualised.

use bevy_egui::egui::{self, Color32, RichText, Ui};

use crate::doc::SourceCategory;
use crate::state::AppState;

fn category_color(c: SourceCategory) -> Color32 {
    match c {
        SourceCategory::Saves => Color32::from_rgb(120, 200, 120),
        SourceCategory::Mod => Color32::from_rgb(220, 170, 90),
        SourceCategory::Base => Color32::from_rgb(140, 160, 200),
    }
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.label("Filter");
        ui.add(
            egui::TextEdit::singleline(&mut state.maps_list.filter)
                .hint_text("substring, e.g. town")
                .desired_width(f32::INFINITY),
        );
    });
    state.refresh_maps_filter();

    let Some(session) = &state.session else {
        ui.separator();
        match &state.loading {
            Some(l) => ui.label(format!("Loading {}: {}...", l.profile_id, l.stage)),
            None => ui.label("Pick a profile in the top bar."),
        };
        return;
    };
    let total = session.maps.len();
    let shown = state.maps_list.filtered.len();
    ui.weak(format!("{shown} of {total} maps"));
    ui.separator();

    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let mut to_open = None;
    let mut missing_sources: Vec<usize> = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, shown, |ui, range| {
            for row in range {
                let Some(&map_index) = state.maps_list.filtered.get(row) else {
                    continue;
                };
                let Some(path) = session.maps.get(map_index) else {
                    continue;
                };
                let cached = state.maps_list.source_cache.get(&map_index);
                if cached.is_none() {
                    missing_sources.push(map_index);
                }
                ui.horizontal(|ui| {
                    // Show the part after `data\global\tiles\`; the full path is the tooltip.
                    let short = path
                        .as_str()
                        .strip_prefix("data\\global\\tiles\\")
                        .unwrap_or(path.as_str());
                    let mut text = RichText::new(short);
                    if let Some((cat, _)) = cached {
                        text = text.color(category_color(*cat));
                    }
                    let mut resp = ui.selectable_label(false, text);
                    resp = resp.on_hover_text(match cached {
                        Some((cat, name)) => format!("{path}\nfrom {name} ({})", cat.label()),
                        None => path.to_string(),
                    });
                    if resp.clicked() {
                        to_open = Some(path.clone());
                    }
                    if let Some((cat, _)) = cached {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(cat.label());
                        });
                    }
                });
            }
        });

    // Resolve sources for the rows just drawn (shown from the next frame on).
    for index in missing_sources {
        let Some(path) = state
            .session
            .as_ref()
            .and_then(|s| s.maps.get(index))
            .cloned()
        else {
            continue;
        };
        if let Some(info) = state.source_of(&path) {
            state
                .maps_list
                .source_cache
                .insert(index, (info.category, info.layer_name));
        }
    }
    if let Some(path) = to_open {
        state.open_map(&path);
    }
}
