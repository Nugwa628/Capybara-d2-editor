//! A blocking error window for problems that stop the editor from starting
//! (for example an unwritable data folder). It names the path and OS error.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

#[derive(Resource)]
struct FatalMessage {
    title: String,
    detail: String,
}

/// Show `detail` in a window with a Quit button; returns when it is closed.
pub fn run(title: &str, detail: &str) {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Capybara Map Editor".into(),
                resolution: bevy::window::WindowResolution::new(640, 320),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(FatalMessage {
            title: title.to_owned(),
            detail: detail.to_owned(),
        })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, ui)
        .run();
}

fn ui(
    mut contexts: EguiContexts,
    message: Res<FatalMessage>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let mut root = crate::ui::root_ui(&ctx);
    egui::CentralPanel::default().show(&mut root, |ui| {
        ui.heading(&message.title);
        ui.add_space(8.0);
        ui.label(&message.detail);
        ui.add_space(12.0);
        ui.label(
            "Nothing was written anywhere else. Move the program to a writable folder, \
             or set CAPY_HOME to one, and start it again.",
        );
        ui.add_space(16.0);
        if ui.button("Quit").clicked() {
            exit.write(AppExit::error());
        }
    });
    Ok(())
}
