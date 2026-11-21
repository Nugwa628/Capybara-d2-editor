# `capy-cli` public API (owner: lead)

The command-line side of the single exe. `capy-app`'s `main` calls
`capy_cli::run(args)` first; `None` means "no CLI verb, start the GUI".
The user-facing reference is `docs/cli.md`.

```rust
/// Parses `args` (without argv[0]). Returns None when the first argument is
/// not a CLI verb (the GUI starts); `help`, `--help`, `-h` and `/?` are
/// verbs. Otherwise runs the verb and returns its exit code (0 ok, 1 runtime
/// error, 2 usage error). Output goes to stdout; errors to stderr. Never
/// panics on bad input.
pub fn run(args: &[std::ffi::OsString]) -> Option<std::process::ExitCode>;

/// `run` against an explicit `capybara\` root and output streams (tests).
/// Returns the numeric exit code, or None when `args` is not a CLI verb.
/// Looks for installs like the exe (`InstallSearch::system`).
pub fn run_in(
    args: &[std::ffi::OsString],
    root: std::path::PathBuf,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
) -> Option<u8>;

/// `run_in` with the install search injected (tests never read the registry):
/// `installs` is called only when a verb other than `help` finds no profile.
pub fn run_in_with(
    args: &[std::ffi::OsString],
    root: std::path::PathBuf,
    installs: &dyn Fn() -> d2_assets::installs::InstallSearch,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
) -> Option<u8>;

/// `<dir of exe>\capybara\` or `CAPY_HOME`; same rule as the app.
pub fn capybara_root() -> std::io::Result<std::path::PathBuf>;
```

Verbs (case-insensitive), all read-only towards game data:
```
render <map> --profile <id|path.toml> [--lvltype N [--def N]] [--zoom 1:N] [--gamma G]
       [--layers floors,walls,shadow,roofs,special,objects|all] [--variant reference|documented]
       [--object-frame N] --out <file.png>
       <map> is a game path (data\global\tiles\...\x.ds1, resolved through the
       profile overlay incl. saves) or a filesystem path to a .ds1.
       Objects draw (ObjectCatalog::build + ObjectSprites + render_with_objects)
       while the objects layer is visible (the default); --layers without
       objects is terrain only. --object-frame (0..=255, default 0) pins every
       object's frame through RenderOptions::object_frames (wrapped per COF).
       The result line ends "missing tiles: N, objects: D drawn, M missing, wrote F"
       ("objects: off" when hidden); missing objects are warnings (exit 0).
index status|build --profile <id>          (build shows a text progress line)
maps list --profile <id> [--filter S]      (one game path per line, with the MapCatalog source)
profiles list                              (id, version 1.13c|1.14d|d2r, name; tab-separated)
help [verb]                                (also --help, -h, /? as the first argument)
```
Profiles resolve by id from `capybara\profiles\<id>.toml`, or a TOML path.
Writes only `--out` targets, `capybara\cache\` and the first-run profiles.

**First run.** Before every verb except `help`, when `capybara\profiles\` holds
no `*.toml`, the CLI calls `d2_assets::installs::seed_default_profiles` (the
GUI's first-run seeding: default locations + read-only registry lookup) and
prints `note: wrote default profile <file>` on stderr per file written. A
seeding error is printed as a warning and the verb still runs. So `profiles
list`, `maps list`, `render` and `index` work on a clean folder.

Product name: Capybara's D2 Editor, exe `CapybarasD2Editor.exe` (crate names
and the `capybara\` folder are unchanged).

`maps list` and `render` (without `--lvltype`) apply the app's per-profile
overrides: `[[overrides]]` entries of `<capybara root>\config.toml` whose
`profile` is the profile id go to `MapCatalog::set_override` before `best()`,
so `Override` shows as the source. The file is read leniently and never
written: a missing file or `overrides` key is no overrides; an unreadable or
malformed file, or a malformed entry, is skipped with a warning on stderr.

Exit codes: 0 ok (missing tiles and objects are warnings); 1 runtime
condition (I/O, a profile that will not open, a map whose tile set cannot be
found without `--lvltype`, render or write failure); 2 usage error only (bad,
missing, unknown or repeated flags, bad values, unknown verb for `help`).

## `mcp` verb (P13)

`mcp --profile <id|path.toml>` serves MCP (JSON-RPC 2.0, one message per
line, protocol versions 2025-06-18 / 2025-03-26 / 2024-11-05) on stdin/stdout:
`initialize`, `ping`, `tools/list`, `tools/call`. `capy_cli::mcp::serve(root,
profile, &mut input, &mut output)` is the testable entry point. Tools:
`list_maps open_map new_map map_info get_cells paint set_cell copy_area
find_replace reroll resize_map set_cell_flags list_objects add_object
edit_objects undo redo check_map render_map save_map` (more below). One map is open at a
time and edited through `capy_edit::Document` (so a session equals the same
direct calls, tested); `save_map` writes `capybara\saves\<profile>` +
`CHANGED.txt` + a history snapshot and never the game folder. Tool failures are
results with `isError`; protocol errors are JSON-RPC errors. `render_map`
returns a PNG image content. Also `regions`, `no_spawn_area`, `list_tiles`,
`list_placeable`, `add_layer`, `clear` and
`history` are ported; overlay rendering in `render_map` is not.

Tile creation (`mcp/session/tiles.rs`, on `capy_mod::tiles_create`; no
creation logic of its own): `tile_template`, `create_tile`,
`import_tile_image`, `reshape_tile`, `new_dt1`, `list_own_tiles`,
`inspect_tile`. Shapes are `orientation` (0-19) + `height` (wall-like
tiles, rounded up to 32) + `roof_height` (roofs, default 80) + `corner_pair`
(3/4: `build_corner_pair` from one 160-wide picture). `create_tile` pixels are
`{data: base64, width, height}` (at most 2048 a side) in the plain top-left
space of `TileTemplate::rect` (placed with `place_indexed`), index 0 or
`transparent_index` empty, over an optional `fill` of `TileTemplate::mask`.
`import_tile_image` reads `path` (a file, .png/.bmp/.gif/.pcx, at most 16 MB)
or `data` (base64), refuses pictures over 4096 a side, and calls
`import_tile_image` with the act palette. Targets: no `dt1` = the open map's
own DT1 via `add_new_tiles_to_map` (number from `free_tile_number` over every
DT1 of the type, or `main`/`sub`; the core renumbers clashes); `dt1` = a DT1
the profile's saves already hold (others are refused), numbers avoiding the
other DT1s the open map loads. Tables are read and saved as the app's "Add to
this map" does (`tables_for_tiles` / `save_registration`: decoded `.bin` and
in-place `patch_bin_field` on 1.13c, `.txt` mirrored via
`mirror_registration`; else the `.txt`). The DS1 file list change is one
`Patch::replace` undo step. After a write the map's `MapInfo` is re-resolved
and its `TileSet` dropped, so placement tools see the tiles. Every write is
`SavesDir::save` + a `SnapshotStore` record; `inspect_tile` `render: "file"`
writes `capybara\renders\<name>.png` (`is_safe_name`). DT1 paths are under
`data\global\tiles` (no `..`, drive, or reserved characters); `new_dt1` names
go through `custom_name_problem`.
