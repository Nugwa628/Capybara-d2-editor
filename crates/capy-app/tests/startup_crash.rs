//! Crash reporting without a GPU: `CAPY_DEBUG_PANIC=1` makes the GUI path
//! panic right after the crash hook is installed. The exe must exit with
//! Rust's panic code, append the report to `capybara\crash.log` and still
//! print the panic to stderr. `CAPY_NO_CRASH_DIALOG=1` keeps the message box
//! away (it would wait for a click).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

#[test]
fn a_startup_panic_writes_crash_log() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("capybara");
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", &data)
        .env("CAPY_DEBUG_PANIC", "1")
        .env("CAPY_NO_CRASH_DIALOG", "1")
        .output()
        .expect("start the exe");
    assert_eq!(out.status.code(), Some(101), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("CAPY_DEBUG_PANIC: test panic at startup"),
        "{stderr}"
    );
    let log = std::fs::read_to_string(data.join("crash.log")).expect("crash.log");
    assert!(log.contains("Capybara's D2 Editor"), "{log}");
    assert!(log.contains(env!("CARGO_PKG_VERSION")), "{log}");
    assert!(
        log.contains("message:  CAPY_DEBUG_PANIC: test panic at startup"),
        "{log}"
    );
    assert!(log.contains("location: crates"), "{log}");
    assert!(log.contains("backtrace:"), "{log}");

    // A second crash appends.
    let again = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .env("CAPY_HOME", &data)
        .env("CAPY_DEBUG_PANIC", "1")
        .env("CAPY_NO_CRASH_DIALOG", "1")
        .output()
        .expect("start the exe");
    assert_eq!(again.status.code(), Some(101));
    let log = std::fs::read_to_string(data.join("crash.log")).expect("crash.log");
    assert_eq!(log.matches("crash report").count(), 2, "{log}");
}

#[test]
fn the_cli_keeps_the_default_panic_output() {
    // CLI verbs never install the hook: nothing is written to a crash.log.
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("capybara");
    let out = Command::new(env!("CARGO_BIN_EXE_CapybarasD2Editor"))
        .arg("--help")
        .env("CAPY_HOME", &data)
        .env("CAPY_DEBUG_PANIC", "1")
        .output()
        .expect("start the exe");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!data.join("crash.log").exists());
}
