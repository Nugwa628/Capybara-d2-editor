//! Capybara Map Editor binary.

mod app;
mod config;
mod doc;
mod edit;
mod fatal;
mod hooks;
mod index_task;
mod iso;
mod paths;
mod render;
mod script_run;
mod state;
mod tasks;
mod ui;

fn main() {
    match paths::Paths::resolve() {
        Ok(paths) => {
            // A failed verification hook or script reports through the exit code.
            if let bevy::app::AppExit::Error(code) = app::run(app::prepare(paths)) {
                std::process::exit(i32::from(code.get()));
            }
        }
        Err(e) => {
            eprintln!("{e}");
            fatal::run(
                "Capybara Map Editor cannot use its data folder",
                &e.to_string(),
            );
        }
    }
}
