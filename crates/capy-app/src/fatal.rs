//! A blocking error box for problems that stop the editor from starting
//! (for example an unwritable data folder). It names the path and OS error.
//! It is a native message box, so it also works without a usable GPU.

/// The box text: the heading, the detail and what to do about it.
pub fn text(heading: &str, detail: &str) -> String {
    format!(
        "{heading}\n\n{detail}\n\nNothing was written anywhere else. Move the program to a \
         writable folder, or set CAPY_HOME to one, and start it again."
    )
}

/// Show the error and return once the user closed it (`CAPY_NO_CRASH_DIALOG=1`
/// skips the box; the caller has already printed the error to stderr).
pub fn run(heading: &str, detail: &str) {
    if std::env::var_os(crate::crash::NO_DIALOG_ENV).is_some_and(|v| !v.is_empty() && v != "0") {
        return;
    }
    crate::crash::show_error(&text(heading, detail));
}

#[cfg(test)]
mod tests {
    #[test]
    fn text_has_heading_detail_and_hint() {
        let t = super::text("cannot use its data folder", "Access is denied.");
        assert!(t.starts_with("cannot use its data folder\n\nAccess is denied."));
        assert!(t.contains("CAPY_HOME"));
    }
}
