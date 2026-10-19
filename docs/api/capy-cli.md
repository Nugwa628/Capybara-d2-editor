# `capy-cli` public API (owner: lead)

The command-line side of the single exe. `capy-app`'s `main` calls
`capy_cli::run(args)` first; `None` means "no CLI verb, start the GUI".

```rust
/// Parses `args` (without argv[0]). Returns None when the first argument is
/// not a CLI verb (the GUI starts). Otherwise runs the verb and returns its
/// exit code (0 ok, 1 runtime error, 2 usage error). Output goes to stdout;
/// errors to stderr. Never panics on bad input.
pub fn run(args: &[std::ffi::OsString]) -> Option<std::process::ExitCode>;

/// `<dir of exe>\capybara\` or `CAPY_HOME`; same rule as the app.
pub fn capybara_root() -> std::io::Result<std::path::PathBuf>;
```

Verbs (case-insensitive), all read-only towards game data:
```
render <map> --profile <id|path.toml> [--lvltype N [--def N]] [--zoom 1:N] [--gamma G]
       [--layers floors,walls,shadow,roofs,special] [--variant reference|documented] --out <file.png>
       <map> is a game path (data\global\tiles\...\x.ds1, resolved through the
       profile overlay incl. saves) or a filesystem path to a .ds1.
index status|build --profile <id>          (build shows a text progress line)
maps list --profile <id> [--filter S]      (one game path per line, with the MapCatalog source)
profiles list
help [verb]
```
Profiles resolve by id from `capybara\profiles\<id>.toml`, or a TOML path.
Writes only `--out` targets and `capybara\cache\`.
