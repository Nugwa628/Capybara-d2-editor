//! Capybara's D2 Editor binary: the GUI, and the `capy-cli` verbs when the
//! first argument is one (`CapybarasD2Editor.exe render ...`).

// Release builds are GUI programs (no console window); CLI verbs attach to
// the console they were started from, see `attach_parent_console`.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod branding;
mod browser;
mod config;
mod crash;
mod doc;
mod edit;
mod fatal;
mod history;
mod hooks;
mod index_task;
mod installs;
mod iso;
mod objects;
mod paths;
mod render;
mod script_run;
mod settings;
mod splash;
mod sprites;
mod state;
mod tasks;
mod ui;

use std::process::ExitCode;

/// The product name in window titles, dialogs and messages.
pub const PRODUCT: &str = "Capybara's D2 Editor";

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
            crash::install(
                Some(paths.root.join(crash::LOG_NAME)),
                crash::dialog_wanted(),
            );
            debug_panic_at_start();
            // A failed verification hook or script reports through the exit code.
            match app::run(app::prepare(paths)) {
                bevy::app::AppExit::Error(code) => ExitCode::from(code.get()),
                bevy::app::AppExit::Success => ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("{e}");
            fatal::run(
                &format!("{PRODUCT} cannot use its data folder"),
                &e.to_string(),
            );
            ExitCode::FAILURE
        }
    }
}

/// `CAPY_DEBUG_PANIC=1`: panic right after the crash hook is installed, to
/// test crash.log and the dialog without a GPU.
fn debug_panic_at_start() {
    if std::env::var_os("CAPY_DEBUG_PANIC").is_some_and(|v| v == "1") {
        #[allow(clippy::panic)]
        {
            panic!("CAPY_DEBUG_PANIC: test panic at startup");
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
