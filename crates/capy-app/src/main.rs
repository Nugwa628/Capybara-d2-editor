//! Capybara Map Editor binary: the GUI, and the `capy-cli` verbs when the
//! first argument is one (`CapybaraMapEditor.exe render ...`).

// Release builds are GUI programs (no console window); CLI verbs attach to
// the console they were started from, see `attach_parent_console`.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod browser;
mod config;
mod doc;
mod edit;
mod fatal;
mod history;
mod hooks;
mod index_task;
mod installs;
mod iso;
mod paths;
mod render;
mod script_run;
mod settings;
mod sprites;
mod state;
mod tasks;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if !args.is_empty() {
        attach_parent_console();
    }
    if let Some(code) = capy_cli::run(&args) {
        return code;
    }
    match paths::Paths::resolve() {
        Ok(paths) => {
            // A failed verification hook or script reports through the exit code.
            match app::run(app::prepare(paths)) {
                bevy::app::AppExit::Error(code) => ExitCode::from(code.get()),
                bevy::app::AppExit::Success => ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("{e}");
            fatal::run(
                "Capybara Map Editor cannot use its data folder",
                &e.to_string(),
            );
            ExitCode::FAILURE
        }
    }
}

/// A `windows_subsystem = "windows"` program has no console; when started
/// from one (cmd, PowerShell), attach to it so CLI output and errors show
/// there. Redirected output (pipes, files) works without this.
#[cfg(all(windows, not(debug_assertions)))]
#[allow(unsafe_code)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: AttachConsole takes a process id by value and touches no
    // memory of ours; it fails harmlessly (returns 0) when there is no
    // parent console or one is already attached, which we ignore.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Debug builds use the console subsystem: nothing to attach.
#[cfg(not(all(windows, not(debug_assertions))))]
fn attach_parent_console() {}
