//! Command-line side of the Capybara Map Editor (no Bevy).
//!
//! `capy-app`'s `main` calls [`run`] first; `None` means "no CLI verb, start
//! the GUI". The verbs are documented in `docs/api/capy-cli.md`; all of them
//! are read-only towards game data and write only `--out` targets and
//! `capybara\cache\`.

mod args;
mod help;
mod index_cmd;
mod maps_cmd;
mod names;
mod overrides;
mod profiles;
mod render_cmd;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::Command;

/// Folder name next to the executable.
const HOME_DIR_NAME: &str = "capybara";
/// Environment variable that overrides the root (development, tests).
const HOME_ENV: &str = "CAPY_HOME";

/// A failed command: `Usage` exits 2, `Runtime` exits 1.
#[derive(Debug)]
pub(crate) enum Fail {
    Usage(String),
    Runtime(String),
}

pub(crate) type CmdResult = Result<(), Fail>;

/// Where a command finds `profiles\`, `saves\` and `cache\`, and where it
/// prints.
pub(crate) struct Ctx<'a> {
    pub root: PathBuf,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl Ctx<'_> {
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.root.join("saves")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn say(&mut self, text: impl AsRef<str>) {
        let _ = writeln!(self.out, "{}", text.as_ref());
    }

    pub fn warn(&mut self, text: impl AsRef<str>) {
        let _ = writeln!(self.err, "warning: {}", text.as_ref());
    }
}

/// `<dir of exe>\capybara\`, or `CAPY_HOME` when set and non-empty; the same
/// rule as the app.
pub fn capybara_root() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(root_from(std::env::var_os(HOME_ENV), &exe))
}

fn root_from(env_override: Option<OsString>, exe: &Path) -> PathBuf {
    match env_override {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
            .join(HOME_DIR_NAME),
    }
}

/// Parses `args` (without argv[0]). Returns `None` when the first argument is
/// not a CLI verb (the GUI starts); `help`, `--help`, `-h` and `/?` are
/// verbs. Otherwise runs the verb and returns its exit code (0 ok, 1 runtime
/// error, 2 usage error). Output goes to stdout; errors to stderr. Never
/// panics on bad input.
pub fn run(args: &[OsString]) -> Option<ExitCode> {
    // Decide "is this a verb" before touching the filesystem or environment.
    let needs_root = match args::parse(args) {
        Ok(None) => return None,
        Ok(Some(Command::Help(_))) | Err(_) => false,
        Ok(Some(_)) => true,
    };
    let root = match capybara_root() {
        Ok(root) => root,
        // Help and usage errors never look at the folder.
        Err(_) if !needs_root => PathBuf::new(),
        Err(e) => {
            eprintln!("error: cannot locate the capybara folder: {e}");
            return Some(ExitCode::from(1));
        }
    };
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let code = run_in(args, root, &mut stdout.lock(), &mut stderr.lock());
    code.map(ExitCode::from)
}

/// [`run`] against an explicit `capybara\` root and explicit output streams
/// (used by tests, which must not touch process-wide state). Returns the
/// numeric exit code, or `None` when `args` is not a CLI verb.
pub fn run_in(
    args: &[OsString],
    root: PathBuf,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Option<u8> {
    let command = match args::parse(args) {
        Ok(None) => return None,
        Ok(Some(command)) => command,
        Err(usage) => {
            let _ = writeln!(err, "error: {}", usage.message);
            let _ = writeln!(err, "{}", help::usage_line(usage.verb));
            return Some(2);
        }
    };
    let mut ctx = Ctx { root, out, err };
    let result = dispatch(&mut ctx, command);
    if let Err(Fail::Usage(message) | Fail::Runtime(message)) = &result {
        let _ = writeln!(ctx.err, "error: {message}");
    }
    Some(exit_code(&result))
}

/// Exit code of a command's result: 0 ok; 1 for a runtime condition (a file
/// that cannot be read or written, a profile that will not open, a map whose
/// tile set cannot be found); 2 only for usage errors (bad or missing flags,
/// bad values).
pub(crate) fn exit_code(result: &CmdResult) -> u8 {
    match result {
        Ok(()) => 0,
        Err(Fail::Runtime(_)) => 1,
        Err(Fail::Usage(_)) => 2,
    }
}

fn dispatch(ctx: &mut Ctx<'_>, command: Command) -> CmdResult {
    match command {
        Command::Render(a) => render_cmd::run(ctx, &a),
        Command::IndexStatus { profile } => index_cmd::status(ctx, &profile),
        Command::IndexBuild { profile } => index_cmd::build(ctx, &profile),
        Command::MapsList { profile, filter } => maps_cmd::list(ctx, &profile, filter.as_deref()),
        Command::ProfilesList => profiles::list(ctx),
        Command::Help(verb) => help::print(ctx, verb.as_deref()),
    }
}

#[cfg(test)]
mod tests;
