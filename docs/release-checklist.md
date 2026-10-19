# Release checklist

Steps to build and check a release of `CapybaraMapEditor.exe` before it is
zipped and shared. Each step says what to run and what to expect. The
results of the last dry run are at the end, together with open issues.

All commands run in PowerShell from the repository root unless noted.
`$S` is any empty scratch folder outside the repository.

## 1. Build

```
cargo build --release -p capy-app
```

- Profile (root `Cargo.toml`): `lto = "thin"`, `strip = true`,
  `codegen-units = 1`. Never enable the `dev` feature (it links Bevy as a DLL).
- Output: `target\release\CapybaraMapEditor.exe`.
- Expect: no warnings from workspace crates; an exe of about **58 MB**
  (60,306,944 bytes on 2026-10-19); 15 minutes from cold with 2 jobs.

See [Open issues](#open-issues) for the static-CRT build that is
recommended but not yet adopted.

## 2. PE header: GUI subsystem, imports

With the Visual Studio build tools:

```
dumpbin /headers   target\release\CapybaraMapEditor.exe | Select-String subsystem
dumpbin /dependents target\release\CapybaraMapEditor.exe
```

Expect:

- `2 subsystem (Windows GUI)`: no console window flashes on double-click.
  CLI verbs attach to the parent console (`src/main.rs`).
- Only DLLs that ship with Windows 10/11: `kernel32`, `user32`, `gdi32`,
  `advapi32`, `ole32`, `oleaut32`, `combase`, `shell32`, `shlwapi`,
  `userenv`, `setupapi`, `imm32`, `dwmapi`, `uxtheme`, `dxgi`,
  `uiautomationcore`, `bcryptprimitives`, `ntdll`, `api-ms-win-core-*`,
  and the Universal CRT `api-ms-win-crt-*` set.
- **No** `bevy_dylib*.dll`, `std-*.dll` or other Rust DLLs.
- `VCRUNTIME140.dll` is **not** part of Windows. Today's build imports it
  (see open issue 2). With a static-CRT build it must be gone.
- No `.rsrc` section today: the exe has no icon, version info or manifest
  (open issue 5).

## 3. No data folder: every editor asset is embedded

Search for runtime reads of editor assets:

```
rg -n "include_bytes!|include_str!|embedded_asset!|load_internal_asset!|asset_server\.load|ShaderRef::Path" crates
```

Expect (2026-10-19):

| Asset | How it ships |
|---|---|
| `assets/known-names/seed.txt` | `include_str!` in `d2-assets/src/names.rs` |
| `assets/editor/obj.txt` | `include_bytes!` in `d2-assets/src/objects.rs` |
| `capy-app/src/render/tile_material.wesl` | `embedded_asset!` + `embedded://` path in `capy-app/src/render/material.rs` |
| UI font | egui `default_fonts` feature (compiled in) |

Nothing may be loaded through the `AssetServer` from an `assets\` folder, and
no `std::fs::read` may point at a path inside the repository. All other file
reads are user data (`capybara\`), game data (MPQs, profile paths) or files
the user picked in a dialog. Do not enable Bevy's `file_watcher` or
`embedded_watcher` features in release: the latter makes embedded assets
read from the source tree.

## 4. Clean-folder run

Copy only the exe into an empty folder, with `CAPY_HOME` unset:

```
New-Item -ItemType Directory $S\clean
Copy-Item target\release\CapybaraMapEditor.exe $S\clean\
Remove-Item Env:CAPY_HOME -ErrorAction SilentlyContinue
cd $S\clean
```

Run each command through `Start-Process -Wait -PassThru` with
`-RedirectStandardOutput`/`-RedirectStandardError` (or `start "" /wait` in
cmd) so the exit code is real; a GUI-subsystem exe does not make the
console wait.

### 4a. CLI before the first GUI start

| Command | Expect |
|---|---|
| `CapybaraMapEditor.exe --help` | help text on stdout, exit 0 |
| `CapybaraMapEditor.exe profiles list` | `no profiles in <dir>\capybara\profiles`, exit 0 |
| `CapybaraMapEditor.exe render data\global\tiles\act1\town\towne1.ds1 --profile vanilla --zoom 1:4 --out t.png` | `error: cannot read profile ...vanilla.toml ...`, exit 1 |

The CLI never creates `capybara\` and never writes the default profiles;
only the GUI's first start does (open issue 3).

### 4b. GUI first start

```
$env:CAPY_SCREENSHOT = "$S\gui1.png"
$env:CAPY_PROFILE    = "vanilla"
$env:CAPY_OPEN_MAP   = "data\global\tiles\act1\town\towne1.ds1"
.\CapybaraMapEditor.exe   # exits by itself after the screenshot
```

Expect:

- exit 0, after roughly 8 s;
- `capybara\` with `config.toml`, `user.db`, `profiles\vanilla.toml`
  (+ `mxl.toml` if Median XL is installed), `saves\`, `history\`,
  `cache\known-names-vanilla.txt/.json`, `cache\index.db*`;
- the screenshot shows towne1 textured, with the Maps list filled and no
  error lines in the Log panel.

Full index build (optional, about 16 s for vanilla):
`$env:CAPY_EXIT_WHEN_INDEXED = "1"` with `CAPY_PROFILE=vanilla`; expect
`Asset index built: 45857 assets` and a ~200 MB `cache\index.db`.

### 4c. CLI after the GUI

| Command | Expect |
|---|---|
| `profiles list` | `mxl 1.13c Median XL` / `vanilla 1.14d Diablo II 1.14d`, exit 0 |
| `render ... --out t.png` | `rendered 1:4 gamma 1.30: 1960x980 px ...`, a PNG of the town, exit 0 |

## 5. Not-writable folder

Deny the current user "create file/folder" on a folder that holds a copy of
the exe. Deny only `WD,AD` on the folder itself: denying inherited `W`
also blocks starting the exe.

```
New-Item -ItemType Directory $S\ro
Copy-Item target\release\CapybaraMapEditor.exe $S\ro\
icacls $S\ro /deny "$env:USERDOMAIN\$env:USERNAME:(WD,AD)"
Start-Process $S\ro\CapybaraMapEditor.exe     # look at the window, then Quit
icacls $S\ro /remove:d "$env:USERDOMAIN\$env:USERNAME"   # always restore
```

Expect a small window titled "Capybara Map Editor" with the heading
"Capybara Map Editor cannot use its data folder", the path and OS error
(`Access is denied. (os error 5)`), the CAPY_HOME hint and a Quit button.
Nothing is created anywhere. The screenshot hook does not apply to this
window, so close it by hand.

## 6. Game folders untouched

After the runs above, the game install folders must have no new or changed
files (the editor opens MPQs read-only). Spot-check the folder's newest
`LastWriteTime`.

## 7. Packaging (`scripts/build-release.ps1`)

One script builds, checks and zips the release. It runs in Windows
PowerShell 5.1 from any folder:

```
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1            # build, check, zip
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 -SkipBuild # check + zip the last build
powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 -DryRun    # print the plan, read-only checks only
```

What it does, in order (any failed check stops it with exit code 1 and a
list of the failures):

1. `python -I scripts/third_party_licenses.py --check`: a stale
   `THIRD-PARTY-LICENSES.md` fails before anything is compiled.
2. Checks that `README.md`, `LICENSE*`, `NOTICE`, `THIRD-PARTY-LICENSES.md`,
   `docs\user-guide.md` and `docs\cli.md` exist.
3. Builds the static-CRT, path-remapped exe (open issues 2 and 4):
   ```
   RUSTFLAGS = -C target-feature=+crt-static
               --remap-path-prefix=<repo>=.
               --remap-path-prefix=<CARGO_HOME>=cargo
               --remap-path-prefix=<RUSTUP_HOME>=rustup
   cargo build --release -p capy-app --target x86_64-pc-windows-msvc --target-dir target\release-dist
   ```
   `CARGO_HOME`/`RUSTUP_HOME` default to `%USERPROFILE%\.cargo` and
   `\.rustup`. The separate target folder keeps these flags from
   invalidating the normal `target\release` and dev builds; a cold build
   takes about 12-15 minutes. `RUSTFLAGS` is restored afterwards and
   `CARGO_ENCODED_RUSTFLAGS` (which would override it) is cleared for the
   build. The exe is `target\release-dist\x86_64-pc-windows-msvc\release\`
   `CapybarasD2Editor.exe` (the old bin name `CapybaraMapEditor.exe` is
   accepted too).
4. Checks the exe without external tools (the PE headers are parsed in the
   script):
   - PE32+ x86-64 and subsystem 2 (Windows GUI);
   - no `VCRUNTIME*`, `MSVCP*`, `ucrtbase` or `api-ms-win-crt-*` in the
     import or delay-import tables (the import list is printed, compare it
     with step 2's list of system DLLs);
   - no occurrence of the builder's user folder (`%USERPROFILE%`), the
     repository, `CARGO_HOME` or `RUSTUP_HOME` in the exe's bytes, as ASCII or
     UTF-16, with `\`, `/` or `\\` separators, ignoring case;
   - prints the size (expect about 58 MB).
5. Writes `dist\CapybarasD2Editor-<version>-win64.zip` (`dist\` is
   gitignored; `<version>` is `[workspace.package] version` from
   `Cargo.toml`, or `-Version`). The zip has one top folder,
   `CapybarasD2Editor-<version>-win64\`, holding `CapybarasD2Editor.exe`,
   `README.md`, `LICENSE*`, `NOTICE`, `THIRD-PARTY-LICENSES.md`,
   `docs\user-guide.md` and `docs\cli.md`. Entry names use `/`
   (`Compress-Archive` in 5.1 writes `\`, which other unzip tools mis-read).
   The script re-opens the zip to count the entries and prints its SHA-256.

Options: `-SkipBuild` checks and packages the existing build; `-ExePath
<exe>` checks and packages another exe (implies `-SkipBuild`; it is renamed
to `CapybarasD2Editor.exe` in the zip); `-DryRun` builds and writes nothing
but still runs steps 1, 2 and 4 (when an exe exists); `-Version` and
`-OutDir` override the zip name and folder.

After the script, run sections 4-6 on the exe from the zip (unzipped into an
empty folder).

Tested on 2026-10-19 without the Bevy build: `-DryRun`; `-SkipBuild` with a
small static-CRT GUI exe built with the same flags (all checks pass, zip of 7
entries); a console exe with a dynamic CRT (fails on subsystem and
`VCRUNTIME140.dll` / `api-ms-win-crt-*`); an exe with an absolute source path
(fails on the user folder); the debug `CapybaraMapEditor.exe` (fails all
three, 8,060 `C:\Users\<name>\.cargo` paths); the old bin name in the release
folder (picked up and renamed); a stale `THIRD-PARTY-LICENSES.md` (fails at
step 1).

---

## Results of the dry run (2026-10-19, main at 20a108d)

| Check | Result |
|---|---|
| Build | OK, 15 min 10 s cold with `CARGO_BUILD_JOBS=2`, no warnings |
| Size | 60,306,944 bytes (57.5 MiB) |
| Subsystem | Windows GUI (2) |
| Imports | system DLLs + UCRT api-sets + **VCRUNTIME140.dll** |
| Embedded assets | all embedded; nothing read from `assets\` at run time |
| CLI before GUI | `--help` 0, `profiles list` 0 (no profiles), `render` 1 (no profile) |
| GUI first start | exit 0, 8.1 s wall to screenshot, peak working set 604 MB, textured towne1 |
| CLI after GUI | `profiles list` 0, `render` 0 (4.7 s, 45 MB, 1 object warning: `tanuhth.cof` not found) |
| Index build | 45,857 assets in 16.0 s, 7 warnings, `index.db` 200 MB |
| Not writable | clear error window, nothing created, no crash |
| No GPU (`WGPU_BACKEND=metal`) | **silent exit 101**, panic text only on stderr |
| Static-CRT build (open issues 2 and 4) | OK, 11 min 52 s, no warnings, 60,383,232 bytes; imports system DLLs only (no `VCRUNTIME140`, no `api-ms-win-crt-*`); 0 registry paths left; GUI screenshot identical in content, CLI `render` PNG byte-identical to the default build |

## Open issues

Ranked; each says whether it is a build/packaging change or a source change.

1. **Startup failures are silent in release (source).** With no usable GPU
   adapter Bevy panics ("Unable to find a GPU!"); the GUI-subsystem exe has
   no console, so a double-click shows nothing and exits with 101. There is
   no panic hook and no log file. Proposal: a panic hook in `main.rs` that
   writes `capybara\crash.log` and shows a native message box, and/or a
   `capybara\log.txt` for Bevy's log in release.
2. **VCRUNTIME140.dll dependency (packaging).** A clean Windows install
   without the Visual C++ 2015-2022 redistributable cannot start the exe.
   Recommended: build with a static CRT. This was tested in the dry run
   (together with issue 4's remap) and works:
   ```
   $env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$env:USERPROFILE\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f=crates.io"
   cargo build --release -p capy-app --target x86_64-pc-windows-msvc
   ```
   (exe in `target\x86_64-pc-windows-msvc\release\`; `--target` keeps the
   flags away from build scripts and proc macros. The index folder name
   comes from `%CARGO_HOME%\registry\src\`.) The exe grows by 76 KB. It can also live in a
   committed `[target.x86_64-pc-windows-msvc] rustflags` entry, but the
   repository root's `.cargo\config.toml` is a gitignored, machine-local
   file today, so the lead has to choose where. Otherwise the README must
   name the redistributable as a requirement.
3. **The CLI cannot be used before the GUI has run once (source/docs).** The
   CLI never seeds `capybara\profiles\`. Either seed the default profiles
   from the CLI too (sharing `installs::seed_profiles`), or say in
   `docs/cli.md` and the README that the editor must be started once first.
4. **Build-machine paths in the exe (packaging).** `strip = true` removes
   symbols but not panic locations: the exe holds 2,636 copies of
   `C:\Users\<name>\.cargo\registry\src\...`, which leak the builder's
   Windows user name. `trim-paths` is still unstable in Cargo 1.99; use
   `--remap-path-prefix=<CARGO_HOME>\registry\src\<index dir>=crates.io`
   in the release `RUSTFLAGS` (tested, see the command in issue 2: no
   registry paths remain; the `/rustc/<hash>/library/...` std paths that
   remain hold no user data).
5. **No icon, version info or manifest (packaging).** The exe has no
   `.rsrc` section, so Explorer shows a generic icon, Properties > Details
   is empty, and the window icon is Bevy's default. Needs an `.ico` (a
   design decision), then a `build.rs` with a resource crate such as
   `winresource` or `embed-resource`. Nothing breaks without a manifest
   today (winit sets DPI awareness at run time; rfd uses COM dialogs only).
6. **System requirements are not documented (docs).** README and user
   guide don't say Windows 10/11 x64, a GPU with Vulkan or DirectX 12
   drivers, or (until issue 2 is fixed) the VC++ redistributable.
7. **README is behind (docs).** It says the tile browser/Ctrl+K are "in
   development" and the history timeline/restore are "planned" (P3 and P4
   are done), and it lists `C:\games\median-xl` as a default install
   location, which PLAN.md says the docs should not present as standard.
8. **Interrupted index builds start over (source, minor).** Quitting while
   the index builds leaves a large `index.db-wal` (12-57 MB here); the next
   start rebuilds from scratch. A full vanilla build takes only 16 s, so
   this is cosmetic, but the WAL could be checkpointed or truncated on
   start.
9. **Release CLI output can trail the prompt (known, documented).** See
   `docs/cli.md`; PLAN.md still has the open decision about also shipping a
   console-subsystem `capy.exe`.
