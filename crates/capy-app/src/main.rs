//! Capybara Map Editor binary.

mod app;
mod config;
mod doc;
mod fatal;
mod hooks;
mod iso;
mod paths;
mod render;
mod state;
mod tasks;
mod ui;

fn main() {
    match paths::Paths::resolve() {
        Ok(paths) => app::run(app::prepare(paths)),
        Err(e) => {
            eprintln!("{e}");
            fatal::run(
                "Capybara Map Editor cannot use its data folder",
                &e.to_string(),
            );
        }
    }
}
