//! Runs the CLI without the GUI: `cargo run --release -p capy-cli --example capy -- help`.
//! Set `CAPY_HOME` to the `capybara\` folder to use (profiles, cache).

use std::ffi::OsString;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    capy_cli::run(&args).unwrap_or_else(|| {
        eprintln!("not a CLI verb; try `help`");
        ExitCode::from(2)
    })
}
