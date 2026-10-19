//! Crash reporting for the GUI.
//!
//! A panic hook appends a report (time, version, message, location,
//! backtrace) to `capybara\crash.log` and shows a native message box titled
//! "Capybara's D2 Editor", so failures such as "no compatible GPU" are no
//! longer silent in the console-less release exe. Rust's default hook still
//! runs too (stderr). The CLI verbs never install this hook.
//!
//! Panics that the editor catches and reports itself (background workers run
//! through [`catch`]) are logged but raise no dialog. The hook never panics
//! on purpose: every write ignores its error, and a second panic while the
//! dialog is up only adds to the log.

use std::cell::Cell;
use std::fmt::Write as _;
use std::io::Write as _;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::PRODUCT;

/// File name inside the data folder.
pub const LOG_NAME: &str = "crash.log";
/// A crash.log larger than this starts over instead of growing.
const MAX_LOG_BYTES: u64 = 1 << 20;
/// `CAPY_NO_CRASH_DIALOG=1`: log only (tests, automation).
pub const NO_DIALOG_ENV: &str = "CAPY_NO_CRASH_DIALOG";

/// Set once a dialog has been shown: one box per process is enough.
static DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Depth of [`catch`] calls on this thread.
    static CAUGHT: Cell<u32> = const { Cell::new(0) };
    /// The hook is running on this thread (a panic inside it).
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// Install the GUI panic hook. `log_file` is `capybara\crash.log` (none when
/// the data folder is unusable); `dialog` false logs without a message box.
pub fn install(log_file: Option<PathBuf>, dialog: bool) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let reentered = IN_HOOK.try_with(|f| f.replace(true)).unwrap_or(true);
        if reentered {
            previous(info);
            return;
        }
        let caught = CAUGHT.try_with(Cell::get).unwrap_or(0) > 0;
        let message = info.payload_as_str().unwrap_or("(no message)").to_owned();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let report = report_text(
            &now_text(),
            &message,
            location.as_deref(),
            &backtrace,
            caught,
        );
        let logged = match &log_file {
            Some(file) => append(file, &report).map(|()| file.as_path()),
            None => Err("the data folder is not usable".to_owned()),
        };
        previous(info);
        if dialog && !caught && !DIALOG_SHOWN.swap(true, Ordering::SeqCst) {
            show_error(&dialog_text(&message, logged));
        }
        let _ = IN_HOOK.try_with(|f| f.set(false));
    }));
}

/// `catch_unwind` for work whose panic the editor reports itself: the hook
/// logs it but shows no dialog.
pub fn catch<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    let _ = CAUGHT.try_with(|c| c.set(c.get().saturating_add(1)));
    let result = panic::catch_unwind(panic::AssertUnwindSafe(f));
    let _ = CAUGHT.try_with(|c| c.set(c.get().saturating_sub(1)));
    result
}

/// Whether automation or the user turned the dialog off.
pub fn dialog_wanted() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0");
    !(set(NO_DIALOG_ENV)
        || set("CAPY_SCRIPT")
        || set("CAPY_SCREENSHOT")
        || set("CAPY_EXIT_WHEN_INDEXED"))
}

/// A native, blocking error box (also used for the unusable-data-folder
/// error, which needs no GPU this way).
pub fn show_error(text: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(PRODUCT)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn now_text() -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "{}",
        jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S %:z")
    );
    s
}

/// The crash.log entry.
pub fn report_text(
    time: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
    caught: bool,
) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "==== {PRODUCT} {} crash report",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(s, "time:     {time}");
    let _ = writeln!(
        s,
        "os:       {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if caught {
        let _ = writeln!(
            s,
            "note:     caught and reported in the editor; it kept running"
        );
    }
    let _ = writeln!(s, "message:  {message}");
    let _ = writeln!(s, "location: {}", location.unwrap_or("(unknown)"));
    let backtrace = backtrace.trim_end();
    if backtrace.is_empty() || backtrace == "disabled backtrace" {
        let _ = writeln!(s, "backtrace: (not available)");
    } else {
        let _ = writeln!(s, "backtrace:\n{backtrace}");
    }
    s.push('\n');
    s
}

/// The message box text: what happened, a hint for GPU trouble, and where
/// the log is.
pub fn dialog_text(message: &str, logged: Result<&Path, String>) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "{PRODUCT} stopped because of an unexpected error:\n\n{message}\n\n"
    );
    if looks_like_gpu_trouble(message) {
        s.push_str(
            "This usually means no graphics driver with Vulkan or DirectX 12 support was \
             found. Install the latest driver for your graphics card and start the editor \
             again.\n\n",
        );
    }
    match logged {
        Ok(file) => {
            let _ = write!(s, "Details were written to:\n{}", file.display());
        }
        Err(e) => {
            let _ = write!(s, "The crash log could not be written: {e}");
        }
    }
    s
}

fn looks_like_gpu_trouble(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "gpu", "adapter", "wgpu", "vulkan", "directx", "dx12", "surface",
    ]
    .iter()
    .any(|k| m.contains(k))
}

/// Append `report` to `file`, starting over when the log has grown large.
fn append(file: &Path, report: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let too_big = std::fs::metadata(file).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if too_big {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    let mut f = options
        .open(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    f.write_all(report.as_bytes())
        .map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_time_version_message_location_and_backtrace() {
        let r = report_text(
            "2026-10-19 12:00:00 +02:00",
            "Unable to find a GPU!",
            Some("src/x.rs:1:2"),
            "   0: main",
            false,
        );
        assert!(r.contains(env!("CARGO_PKG_VERSION")), "{r}");
        assert!(r.contains("2026-10-19 12:00:00"), "{r}");
        assert!(r.contains("message:  Unable to find a GPU!"), "{r}");
        assert!(r.contains("location: src/x.rs:1:2"), "{r}");
        assert!(r.contains("backtrace:\n   0: main"), "{r}");
        assert!(!r.contains("caught"), "{r}");
        let caught = report_text("t", "m", None, "disabled backtrace", true);
        assert!(caught.contains("caught"), "{caught}");
        assert!(caught.contains("location: (unknown)"), "{caught}");
        assert!(caught.contains("backtrace: (not available)"), "{caught}");
    }

    #[test]
    fn dialog_names_the_log_and_hints_at_the_gpu() {
        let log = Path::new(r"C:\x\capybara\crash.log");
        let t = dialog_text(
            "Unable to find a GPU! Make sure you have a supported GPU",
            Ok(log),
        );
        assert!(t.contains("Vulkan or DirectX 12"), "{t}");
        assert!(t.contains(r"C:\x\capybara\crash.log"), "{t}");
        let t = dialog_text("index out of bounds", Err("denied".to_owned()));
        assert!(!t.contains("Vulkan"), "{t}");
        assert!(t.contains("could not be written: denied"), "{t}");
    }

    #[test]
    fn append_creates_grows_and_restarts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sub").join(LOG_NAME);
        append(&file, "one\n").expect("first");
        append(&file, "two\n").expect("second");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "one\ntwo\n");
        let big = "x".repeat(MAX_LOG_BYTES as usize + 1);
        std::fs::write(&file, big).expect("big");
        append(&file, "fresh\n").expect("restart");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "fresh\n");
    }

    #[test]
    fn catch_returns_the_value_or_the_panic() {
        assert_eq!(catch(|| 7).ok(), Some(7));
        #[allow(clippy::panic)]
        let r = catch(|| panic!("expected test panic"));
        assert!(r.is_err());
        assert_eq!(CAUGHT.with(Cell::get), 0);
    }
}
