//! The Sprites list: every `.dc6`/`.dcc`/`.cof` among the profile's known
//! names, filtered and virtualised, plus "Open file..." for loose files.

use bevy_egui::egui::{self, Ui};
use d2_assets::GamePath;

use crate::sprites::dialog::Purpose;
use crate::state::AppState;

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
    ui.weak(format!("{shown} of {total} sprites"));
    ui.separator();

    let current = state
        .sprites
        .viewer
        .source
        .as_ref()
        .and_then(|s| s.game_path().cloned());
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let mut to_open: Option<GamePath> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, shown, |ui, range| {
            for row in range {
                let Some(path) = state
                    .sprites
                    .list
                    .filtered
                    .get(row)
                    .and_then(|&i| session.sprites.get(i))
                else {
                    continue;
                };
                let short = path
                    .as_str()
                    .strip_prefix("data\\global\\")
                    .unwrap_or(path.as_str());
                let selected = current.as_ref() == Some(path);
                if ui
                    .selectable_label(selected, short)
                    .on_hover_text(path.as_str())
                    .clicked()
                {
                    to_open = Some(path.clone());
                }
            }
        });
    if let Some(path) = to_open {
        let _ = state.open_sprite(&path);
    }
}
