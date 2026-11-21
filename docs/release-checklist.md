# Release checklist

Steps to build and check a release before it is packed and shared. Each step
says what to run and what to expect. A release has two archives:

- `CapybarasD2Editor-<version>-win64.zip`: sections 1-7 (Windows, built on
  Windows with MSVC), or [Windows exe from Linux](#windows-exe-from-linux).
- `CapybarasD2Editor-<version>-linux-x86_64.tar.gz`: [Linux](#linux).

The results of the last runs are at the end, together with open issues.

Sections 1-7 run in PowerShell from the repository root unless noted.
`$S` is any empty scratch folder outside the repository.

## 1. Build

```
cargo build --release -p capy-app
```

- Profile (root `Cargo.toml`): `lto = "thin"`, `strip = true`,
  `codegen-units = 1`. Never enable the `dev` feature (it links Bevy as a DLL).
- Output: `target\release\CapybarasD2Editor.exe`.
- Expect: no warnings from workspace crates; an exe of about **58 MB**
  (60,306,944 bytes on 2026-10-19); 15 minutes from cold with 2 jobs.

See [Open issues](#open-issues) for the static-CRT build that is
recommended but not yet adopted.

## 2. PE header: GUI subsystem, imports

With the Visual Studio build tools:

```
dumpbin /headers   target\release\CapybarasD2Editor.exe | Select-String subsystem
dumpbin /dependents target\release\CapybarasD2Editor.exe
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
- A `.rsrc` section with the capybara icon and the version information
  (`crates/capy-app/build.rs`, needs `rc.exe` from the Windows SDK; without
  it the build prints `warning: no icon or version info in the exe`). Check:
  ```
  (Get-Item target\release\CapybarasD2Editor.exe).VersionInfo | Format-List ProductName, FileDescription, FileVersion, ProductVersion, OriginalFilename
  ```
  Expect `Capybara's D2 Editor` (twice), the workspace version (`0.1.0`)
  twice and `CapybarasD2Editor.exe`; Explorer shows the capybara icon. There
  is no manifest (none is needed: winit sets DPI awareness at run time).

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
| `assets/branding/capybara.ico` | `include_bytes!` in `capy-app/src/branding.rs` (window/taskbar icon, loading-screen capybara) and the exe resource (`build.rs`) |

The loading screen's door animation, palette and font are read from the
start profile's game files at run time (read-only), not shipped.

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
Copy-Item target\release\CapybarasD2Editor.exe $S\clean\
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
| `CapybarasD2Editor.exe --help` | help text on stdout, exit 0 |
| `CapybarasD2Editor.exe profiles list` | `note: wrote default profile ...vanilla.toml` (and `mxl.toml` when Median XL is found) on stderr, then the profiles, exit 0 |
| `CapybarasD2Editor.exe render data\global\tiles\act1\town\towne1.ds1 --profile vanilla --zoom 1:4 --out t.png` | a PNG of the town, exit 0 |

The CLI writes the default profiles itself (into `capybara\profiles\`
only; `profiles list` checked on the debug build 2026-10-19), so it works
before the GUI has ever run.

### 4b. GUI first start

```
$env:CAPY_SCREENSHOT = "$S\gui1.png"
$env:CAPY_PROFILE    = "vanilla"
$env:CAPY_OPEN_MAP   = "data\global\tiles\act1\town\towne1.ds1"
.\CapybarasD2Editor.exe   # exits by itself after the screenshot
```

Expect:

- exit 0, after roughly 8 s;
- `capybara\` with `config.toml`, `user.db`, `profiles\vanilla.toml`
  (+ `mxl.toml` if Median XL is installed), `saves\`, `history\`,
  `cache\known-names-vanilla.txt/.json`, `cache\index.db*`;
- the screenshot shows towne1 textured, with the Maps list filled and no
  error lines in the Log panel. (Verification hooks such as
  `CAPY_SCREENSHOT` start without the loading screen.)

Then start the exe by double-click, without the variables: the window is
titled "Capybara's D2 Editor" with the capybara in the title bar and
taskbar, and the loading screen (door animation, capybara in the door, gold
title on the plate) plays until the Maps list is ready, then fades out. A
click or Esc skips it; **File > Settings... > Show loading screen** turns it
off (`show_loading_screen` in `config.toml`).

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
Copy-Item target\release\CapybarasD2Editor.exe $S\ro\
icacls $S\ro /deny "$env:USERDOMAIN\$env:USERNAME:(WD,AD)"
Start-Process $S\ro\CapybarasD2Editor.exe     # read the message box, then OK
icacls $S\ro /remove:d "$env:USERDOMAIN\$env:USERNAME"   # always restore
```

Expect a native error box titled "Capybara's D2 Editor" with the heading
"Capybara's D2 Editor cannot use its data folder", the path and OS error
(`Access is denied. (os error 5)`) and the CAPY_HOME hint. Nothing is
created anywhere (so there is no crash.log either). It needs no GPU.

## 5b. Crash report

```
$env:CAPY_HOME = "$S\nogpu\capybara"; $env:WGPU_BACKEND = "metal"
.\CapybarasD2Editor.exe
Remove-Item Env:WGPU_BACKEND, Env:CAPY_HOME
```

Expect an error box titled "Capybara's D2 Editor": "stopped because of an
unexpected error: Unable to find a GPU! ...", the Vulkan/DirectX 12 hint and
the path of `capybara\crash.log`. After OK the exe exits with 101 and
`crash.log` holds the time, version, message, location and backtrace
(frames without symbols in a stripped release build).
`CAPY_NO_CRASH_DIALOG=1` skips the box (the automated test `startup_crash`
uses it with `CAPY_DEBUG_PANIC=1`, which panics right after the hook is
installed). Checked on the debug build 2026-10-19.

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
(fails on the user folder); the debug `CapybaraMapEditor.exe` of that time (fails all
three, 8,060 `C:\Users\<name>\.cargo` paths); the old bin name in the release
folder (picked up and renamed); a stale `THIRD-PARTY-LICENSES.md` (fails at
step 1).

## Linux

Built and checked on Linux (Linux Mint 22, glibc 2.39, rustc 1.100) from
the repository root in bash. `$S` is any empty scratch folder.

### L1. Build and package (`scripts/build-release.sh`)

```
scripts/build-release.sh                 # build, check, pack the Linux archive
scripts/build-release.sh --smoke         # ... and run it headless (L4)
scripts/build-release.sh --platform both # ... and the Windows zip (see below)
scripts/build-release.sh --skip-build    # check + pack the last build
scripts/build-release.sh --dry-run       # plan + read-only checks only
```

Needs `python3`, `binutils` (`readelf`, `objdump`), `tar`, `gzip` and
coreutils; `--smoke` also needs `xvfb` and `mesa-vulkan-drivers`
(`sudo apt install xvfb mesa-vulkan-drivers`). What it does, in order (any
failed check stops it with exit code 1 and a list of the failures):

1. `python3 -I scripts/third_party_licenses.py --check`. The notice covers
   the union of the Windows and Linux packages (the Linux binary adds x11rb,
   x11-dl, xkbcommon-dl, wayland-client and others).
2. Checks that the documents, `assets/branding/capybara-256.png` and
   `packaging/linux/*` exist.
3. Builds with remapped paths into the same separate target folder as the
   Windows script:
   ```
   RUSTFLAGS = --remap-path-prefix=<repo>=.
               --remap-path-prefix=<CARGO_HOME>=cargo
               --remap-path-prefix=<RUSTUP_HOME>=rustup
   cargo build --release -p capy-app --locked --target x86_64-unknown-linux-gnu --target-dir target/release-dist
   ```
   Without the remap the binary holds about 1,500 copies of
   `/home/<name>/.cargo/registry/src/...` (panic locations). No static CRT
   flag: glibc must stay dynamic (it is what the X11/Vulkan libraries opened
   at run time use).
4. Checks the binary:
   - ELF64 x86-64, stripped (no `.symtab`);
   - `NEEDED` is only `libc.so.6`, `libm.so.6`, `libgcc_s.so.1` and the
     loader. Everything else is opened with `dlopen` when used:
     `libvulkan.so.1` (wgpu), `libX11*`, `libXcursor`, `libXi`,
     `libxkbcommon*` (winit, X11 only: Bevy's `wayland` feature is off, so a
     Wayland session uses XWayland), `libdbus-1.so.3` (file dialogs through
     the XDG desktop portal), `libwayland-client` (rfd, portal window
     handle). No ALSA, no libudev (no audio or gamepad features);
   - the glibc floor: the highest non-weak `GLIBC_x.y` version in
     `readelf -V` must not exceed `--max-glibc` (default 2.35). Today it is
     **2.35** (`hypot`/`hypotf` from libm; `pidfd_spawnp` and
     `pidfd_getpid` need 2.39 but are weak, so std falls back on older
     glibc). 2.35 means Ubuntu 22.04, Linux Mint 21, Debian 12 (2.36),
     Fedora 36 and later; older distributions fail at start with
     ``version `GLIBC_2.35' not found``;
   - no occurrence of `$HOME`, the repository, `CARGO_HOME` or `RUSTUP_HOME`
     (UTF-8 and UTF-16, any case);
   - `--help` runs (no display needed) and exits 0.
5. Writes `dist/CapybarasD2Editor-<version>-linux-x86_64.tar.gz` with one top
   folder holding `CapybarasD2Editor` (0755), `README.md`, `LICENSE*`,
   `NOTICE`, `THIRD-PARTY-LICENSES.md`, `docs/user-guide.md`, `docs/cli.md`,
   `capybara.png` (256x256, from the `.ico`), `capybaras-d2-editor.desktop`
   and `install-desktop-entry.sh` (0755). The tar is reproducible: names
   sorted, owner 0:0, every time set to the last commit's
   (`SOURCE_DATE_EPOCH` overrides), `gzip -n`. Two runs give the same
   SHA-256. It is re-read to count the files and its SHA-256 is printed.
6. With `--smoke`, L4 below on the unpacked archive.

### L2. Embedded assets

Section 3 applies unchanged: the same `include_bytes!`/`embedded_asset!`
set, nothing is read from `assets/`. The Linux archive's `capybara.png` is
only for the desktop entry; the window icon comes from the embedded `.ico`.
On Linux the UI font is egui's (the log says `Segoe UI not found`).

### L3. Desktop entry

`install-desktop-entry.sh` (no root) writes
`${XDG_DATA_HOME:-~/.local/share}/applications/capybaras-d2-editor.desktop`
from the template, with `Exec` set to the quoted absolute path of the
binary next to it, `Icon` to its `capybara.png` and `Path` to its folder;
`--remove` deletes it. `StartupWMClass=CapybarasD2Editor` matches the X11
`WM_CLASS` winit sets (the binary name), so the taskbar groups the window
under the entry. Check with `desktop-file-validate` (from
`desktop-file-utils`): no errors. Paths with spaces, `&` and `|` work;
paths with `"`, `` ` ``, `$`, `\` or a newline are refused.

### L4. Clean-folder run, headless

**Never start the GUI on your own display (`:0`) for these checks, and never
drive it with xdotool/wmctrl there.** Use a virtual X server and Mesa's
software Vulkan driver (lavapipe):

```
mkdir -p $S && tar -xzf dist/CapybarasD2Editor-*-linux-x86_64.tar.gz -C $S
cd $S/CapybarasD2Editor-*-linux-x86_64
unset CAPY_HOME
./CapybarasD2Editor --help                   # exit 0
./CapybarasD2Editor profiles list            # seeds vanilla/mxl from Wine prefixes, exit 0
./CapybarasD2Editor render data/global/tiles/act1/town/towne1.ds1 --profile vanilla --zoom 1:4 --out t.png
env -u DISPLAY -u WAYLAND_DISPLAY \
  CAPY_SCREENSHOT=$S/gui.png CAPY_PROFILE=vanilla \
  CAPY_OPEN_MAP='data\global\tiles\act1\town\towne1.ds1' \
  WGPU_BACKEND=vulkan VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a -s "-screen 0 1920x1080x24" ./CapybarasD2Editor
```

Expect exit 0 each time, `capybara/` next to the binary with the same
contents as section 4b, and a screenshot of towne1 textured with the Maps
list filled. `--smoke` runs `profiles list` and the GUI step.

### L5. Not-writable folder and errors

```
mkdir $S/ro && cp CapybarasD2Editor $S/ro/ && chmod a-w $S/ro
env -u DISPLAY -u WAYLAND_DISPLAY CAPY_NO_CRASH_DIALOG=1 $S/ro/CapybarasD2Editor
chmod u+w $S/ro
```

Expect `Cannot create folder ".../ro/capybara": Permission denied (os error
13)` on stderr, exit 1, nothing created. Without `CAPY_NO_CRASH_DIALOG` the
error box is a `zenity --error` window (rfd has no other message box on
Linux), so check that only under `xvfb-run`. Without zenity there is no box,
only stderr. File pickers use the XDG desktop portal over D-Bus and fall
back to zenity.

### L6. Data home in an unpacked folder

The default-home rule is unchanged (an open owner decision): `capybara/`
next to the executable, or `CAPY_HOME`. On Linux `current_exe()` reads
`/proc/self/exe`, so:

- unpacked under `$HOME` (for example `~/Apps/...`): works; the data lives in
  the versioned folder, so a new version unpacked next to it starts empty
  until `capybara/` is moved over or `CAPY_HOME` is set (same as the Windows
  zip, whose top folder is versioned too);
- a symbolic link (for example `~/.local/bin/CapybarasD2Editor`) uses the
  folder next to the real file;
- a copy in `/opt` or `/usr/local/bin` (root-owned) stops with the L5
  error; set `CAPY_HOME` or keep it in a user folder. The XDG way
  (`~/.local/share/...`) would avoid this but is not the current design.

### L7. AppImage: not made

An AppImage adds nothing the tar does not already give: the binary links
only glibc and libgcc, and the libraries it opens at run time (Vulkan loader
and driver, X11, D-Bus) must come from the host anyway, so there is nothing
to bundle. It would cost: `appimagetool` downloaded from the web, a FUSE 2
dependency that Ubuntu 22.04+/Mint 21+ no longer install by default
(`libfuse2`), and a read-only squashfs mount as the executable's folder, so
`capybara/` next to the exe cannot be created and every user would need
`CAPY_HOME` (it would force the open default-home decision). Revisit if the
data home moves to XDG folders.

## Windows exe from Linux

Sections 1-7 build the exe on Windows with MSVC. It can also be cross-built
on Linux, so one PC makes both archives:

```
rustup target add x86_64-pc-windows-gnu
scripts/build-release.sh --platform both [--windows-linker mingw|zig]
```

The script builds `x86_64-pc-windows-gnu` into `target/release-dist`, checks
the exe with binutils' `objdump -p` (PE32+ x86-64, subsystem 2, imports, no
`vcruntime*`, `msvcp*`, `libgcc*`, `libwinpthread*` or other non-system
DLLs; `.rsrc` with the UTF-16 version strings; no builder paths) and writes
`dist/CapybarasD2Editor-<version>-win64.zip` with the same layout as
`build-release.ps1` (forward-slash entry names, fixed times, reproducible).

The only C code is SQLite (`libsqlite3-sys`, `bundled`; `blake3` uses
`pure`, `wayland-backend` is Linux-only and compiles nothing without `log`),
so a cross C compiler is needed, plus a resource compiler for the icon and
version information. `build.rs` picks, on a non-Windows host: `CAPY_RC` (an
rc.exe-style command such as `zig rc` or `llvm-rc`, whose `.res` lld links),
then `WINDRES`/`x86_64-w64-mingw32-windres` for `-gnu`, then `llvm-rc` for
`-msvc`; without one the exe has no icon and the script fails its `.rsrc`
check.

Toolchains compared (2026-11-21):

| | mingw-w64 (gnu) | zig + cargo-zigbuild (gnu) | cargo-xwin (msvc) |
|---|---|---|---|
| Root needed | yes, `sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64` | no: `pip install --user ziglang` (or a zig on `PATH`), `cargo install --locked cargo-zigbuild` | yes, `sudo apt install clang lld llvm` (clang-cl, lld-link, llvm-rc) |
| Downloads | distro packages only | zig from PyPI | the MSVC CRT and Windows SDK from Microsoft at build time |
| License | GPL toolchain, runtime under permissive mingw-w64 terms (exception for linked code) | MIT; mingw-w64 libs as left | Microsoft's Visual Studio / Build Tools license must be accepted by the builder; `xwin` asks for it, using it on Linux is the builder's responsibility |
| CRT | `msvcrt.dll` (every Windows) | UCRT (`api-ms-win-crt-*`, Windows 10+) | static CRT, same as today's release |
| Same as current releases | no (gnu ABI) | no (gnu ABI) | yes (`x86_64-pc-windows-msvc`, `+crt-static`) |
| Tested here | no (no root) | **yes** | no (no root, license) |

Result with zig (zig 0.17.1 via `ziglang`, cargo-zigbuild 0.24.1): cold
build 3 min 51 s; `CapybarasD2Editor.exe` 58,385,408 bytes, PE32+ x86-64,
GUI subsystem, imports only system DLLs and the UCRT api sets (which
Windows 10/11 ship; no VC++ redistributable), icon and version resource
(all six `.ico` images present as `RT_ICON`), no builder paths. Under Wine
9.0 in a scratch prefix inside `xvfb-run` (Vulkan through lavapipe):
`--help`, `profiles list` and `render towne1 --zoom 1:4` exit 0 with the
same output as on Windows; the GUI renders towne1 textured and exits 0 with
a Wine virtual desktop (`HKCU\Software\Wine\Explorer` `Desktop=Default`,
`Explorer\Desktops` `Default=1600x900`). Without the virtual desktop winit
panics in `monitor.rs` (`has_flag(mode.dmFields, REQUIRED_FIELDS)`) because
Wine reports Xvfb's display modes without all fields: a Wine/Xvfb quirk,
not a defect of the exe. Use a scratch `WINEPREFIX`, never your game prefix,
and write profiles with `Z:\...` paths to reach the game files.

Recommendation: release the Windows zip from Linux with the **gnu target**,
built with **mingw-w64 from apt** once installed (distro-maintained, no
license to accept, `msvcrt.dll` works on every Windows); zig is the no-root
fallback and is what was verified here. Before the first such release, run
sections 4-6 once on a real Windows 10/11 PC (Explorer icon and Details tab,
DirectX 12 and Vulkan start, file dialogs, crash box), since Wine only
approximates them. Keep `build-release.ps1` (MSVC) for releases made on
Windows. cargo-xwin is the choice only if the exe must stay byte-for-byte
MSVC; it needs the owner to accept Microsoft's license.

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
| No GPU (`WGPU_BACKEND=metal`) | **silent exit 101**, panic text only on stderr (fixed since: error box + crash.log, see 5b) |
| Static-CRT build (open issues 2 and 4) | OK, 11 min 52 s, no warnings, 60,383,232 bytes; imports system DLLs only (no `VCRUNTIME140`, no `api-ms-win-crt-*`); 0 registry paths left; GUI screenshot identical in content, CLI `render` PNG byte-identical to the default build |

## Results of the release build (2026-10-19, main at 5b3f01b)

| Check | Result |
|---|---|
| `scripts/build-release.ps1` | full build 7.9 min, exit 0 |
| Exe | `CapybarasD2Editor.exe`, 61,089,792 bytes (58.3 MiB), PE32+ x86-64, GUI subsystem |
| Imports | system DLLs only; no VCRUNTIME, MSVCP, ucrtbase or api-ms-win-crt (static CRT) |
| Builder paths | none of the 12 patterns (user folder, repository, cargo, rustup) |
| Version info | ProductName and FileDescription "Capybara's D2 Editor", 0.1.0, OriginalFilename `CapybarasD2Editor.exe`; capybara icon |
| Zip | `dist\CapybarasD2Editor-0.1.0-win64.zip`, 21,409,439 bytes, 7 entries |
| CLI from the unzipped folder, no `capybara\` yet | `profiles list` seeds vanilla and mxl, exit 0; `render towne1 --zoom 1:4` 1960x980, 0 missing tiles, 42 objects drawn, exit 0 |
| GUI from the unzipped folder | loading screen frame renders (door, capybara, gold title); towne1 opens with 84 object sprites, catalog ready in 2.1 s |

## Results of the Linux release build (2026-11-21, main at cf00ab9 + this branch)

`PATH=<zig venv>:$PATH scripts/build-release.sh --platform both --smoke` on
Linux Mint 22 (32 threads, `CARGO_BUILD_JOBS=16`):

| Check | Result |
|---|---|
| License notice | up to date, 409 packages (374 Windows-only before) |
| Linux build | 3 min 18 s (warm registry), no warnings |
| Linux binary | 65,019,336 bytes (62.0 MiB), ELF64 x86-64, stripped |
| NEEDED | `libc.so.6`, `libm.so.6`, `libgcc_s.so.1`, `ld-linux-x86-64.so.2` |
| glibc floor | 2.35 (weak 2.39 symbols only) |
| Builder paths | none after the remap (1,572 x `/home/<name>/.cargo/registry/src` without it) |
| Archive | `CapybarasD2Editor-1.2.3-linux-x86_64.tar.gz`, 24,066,811 bytes, 10 files, identical SHA-256 on a second run |
| Smoke (xvfb + lavapipe) | `profiles list` seeds vanilla and mxl from Wine prefixes; GUI exit 0, catalog 3.2 s, towne1 2724 tiles and 84 object sprites, 0 missing; CLI `render` 1960x980, 0 missing tiles, 42 objects (same `tanuhth.cof` warning as Windows) |
| Not-writable folder | stderr message, exit 1, nothing created; zenity box under xvfb |
| Windows build (zig) | 3 min 51 s; 58,385,408 bytes; GUI subsystem; system DLLs + UCRT api sets; icon and version resource; no builder paths |
| Windows zip | `CapybarasD2Editor-1.2.3-win64.zip`, 22,751,283 bytes, 7 files, reproducible |
| Windows exe under Wine 9.0 (xvfb) | CLI `--help`, `profiles list`, `render` exit 0; GUI exit 0 with a virtual desktop, towne1 textured |

Seen on Linux, not release blockers: `--help` prints the usage line as
`CapybarasD2Editor.exe <verb>` (capy-cli); the README Quick start says
**File > Add game install...** and two user-guide tables **Add game
install...**, while the button is **Home > Add game...** (UI docs lane).

## Open issues

Ranked; each says whether it is a build/packaging change or a source change.

1. **Startup failures are silent in release (source).** Done (app release
   round): a panic hook writes `capybara\crash.log` and shows a native
   message box (section 5b). Still open: a `capybara\log.txt` for Bevy's
   log in release.
2. **VCRUNTIME140.dll dependency (packaging).** Done: `scripts/build-release.ps1`
   builds with a static CRT and checks the imports (results below). A clean Windows install
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
3. **The CLI cannot be used before the GUI has run once (source/docs).** Done:
   the CLI seeds the default profiles through `d2_assets::installs`. The
   CLI never seeds `capybara\profiles\`. Either seed the default profiles
   from the CLI too (sharing `installs::seed_profiles`), or say in
   `docs/cli.md` and the README that the editor must be started once first.
4. **Build-machine paths in the exe (packaging).** Done: the script remaps the
   repository, cargo and rustup paths and checks the exe for 12 path patterns. `strip = true` removes
   symbols but not panic locations: the exe holds 2,636 copies of
   `C:\Users\<name>\.cargo\registry\src\...`, which leak the builder's
   Windows user name. `trim-paths` is still unstable in Cargo 1.99; use
   `--remap-path-prefix=<CARGO_HOME>\registry\src\<index dir>=crates.io`
   in the release `RUSTFLAGS` (tested, see the command in issue 2: no
   registry paths remain; the `/rustc/<hash>/library/...` std paths that
   remain hold no user data).
5. **No icon, version info or manifest (packaging).** Done: icon and
   version information via `build.rs` (`winresource`); no manifest (none
   needed).
6. **System requirements are not documented (docs).** Done in README and
   the user guide (Windows 10/11 x64, Vulkan or DirectX 12 driver; the
   static-CRT release needs no VC++ redistributable).
7. **README is behind (docs).** Done: features are current and
   `C:\games\median-xl` is no longer presented as a standard location.
8. **Interrupted index builds start over (source, minor).** Quitting while
   the index builds leaves a large `index.db-wal` (12-57 MB here); the next
   start rebuilds from scratch. A full vanilla build takes only 16 s, so
   this is cosmetic, but the WAL could be checkpointed or truncated on
   start.
9. **Release CLI output can trail the prompt (known, documented).** See
   `docs/cli.md`. Decided (user, 2026-10-19): one exe only, no console
   `capy.exe`.
