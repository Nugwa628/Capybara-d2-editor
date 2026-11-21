#!/usr/bin/env bash
# Builds, checks and packages the release of Capybara's D2 Editor on Linux.
#
# See docs/release-checklist.md, "Linux". Steps:
#
#   1. THIRD-PARTY-LICENSES.md must be up to date
#      (python3 -I scripts/third_party_licenses.py --check).
#   2. The files to package must exist.
#   3. cargo build --release -p capy-app --target <triple> with remapped
#      source paths, into target/release-dist (dev builds stay valid).
#   4. Checks on the binary. Linux: ELF x86-64, stripped, only glibc and
#      libgcc_s as NEEDED libraries, the glibc symbol-version floor (at most
#      --max-glibc), no builder paths, `--help` runs. Windows: PE32+ x86-64,
#      GUI subsystem, only system DLLs (no VC++ runtime, libgcc or
#      winpthread DLLs), icon and version resource, no builder paths.
#   5. dist/CapybarasD2Editor-<version>-linux-x86_64.tar.gz and, with
#      --platform windows|both, dist/CapybarasD2Editor-<version>-win64.zip.
#      Both have one top folder; the archives are reproducible (sorted,
#      fixed owner and time). Each is re-read and its SHA-256 printed.
#   6. With --smoke: the Linux archive is unpacked into a scratch folder and
#      run there: `profiles list`, then the GUI under xvfb-run (Mesa's
#      lavapipe Vulkan driver), never on your own display.
#
# The Windows exe is cross-built for x86_64-pc-windows-gnu, with mingw-w64
# (sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64) or, with
# no root, zig through cargo-zigbuild. See the checklist for the trade-offs.
#
# Game folders are never touched. Nothing is pushed or uploaded.
#
# Usage:
#   scripts/build-release.sh                       build, check, pack Linux
#   scripts/build-release.sh --platform both       also the Windows exe
#   scripts/build-release.sh --skip-build          check + pack the last build
#   scripts/build-release.sh --dry-run             plan + read-only checks only
#   scripts/build-release.sh --smoke               also run the packed binary
#
# Options:
#   --platform linux|windows|both   what to build (default linux)
#   --skip-build                    do not compile; use the last build
#   --dry-run                       build and write nothing
#   --bin PATH                      check and pack this Linux binary (implies
#                                   --skip-build for Linux)
#   --exe PATH                      check and pack this Windows exe (implies
#                                   --skip-build for Windows)
#   --windows-linker auto|mingw|zig (default auto: mingw-w64 when installed)
#   --max-glibc X.Y                 highest glibc version the binary may
#                                   require (default 2.35)
#   --version V                     version in the archive names (default:
#                                   [workspace.package] version)
#   --out-dir DIR                   where the archives go (default dist/)
#   --smoke                         run the packed Linux binary headless
set -uo pipefail

LINUX_TARGET=x86_64-unknown-linux-gnu
WINDOWS_TARGET=x86_64-pc-windows-gnu
BIN_NAME=CapybarasD2Editor
EXE_NAME=CapybarasD2Editor.exe
# Libraries every glibc system has; anything else must be dlopen'ed.
ALLOWED_NEEDED='^(libc\.so\.6|libm\.so\.6|libgcc_s\.so\.1|ld-linux-x86-64\.so\.2|libpthread\.so\.0|libdl\.so\.2|librt\.so\.1)$'
# DLLs a clean Windows 10/11 does not have.
FORBIDDEN_DLLS='^(vcruntime[0-9].*|msvcp[0-9].*|libgcc.*|libwinpthread.*|libstdc\+\+.*|lib.*)\.dll$'

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
TARGET_DIR="$REPO/target/release-dist"

platform=linux
skip_build=0
dry_run=0
bin_path=""
exe_path=""
win_linker=auto
max_glibc=2.35
version=""
out_dir="$REPO/dist"
smoke=0

die_usage() { echo "build-release: $*" >&2; echo "see the header of $0 for usage" >&2; exit 2; }
while [[ $# -gt 0 ]]; do
    case "$1" in
        --platform) platform="${2:-}"; shift 2 ;;
        --skip-build) skip_build=1; shift ;;
        --dry-run) dry_run=1; shift ;;
        --bin) bin_path="${2:-}"; shift 2 ;;
        --exe) exe_path="${2:-}"; shift 2 ;;
        --windows-linker) win_linker="${2:-}"; shift 2 ;;
        --max-glibc) max_glibc="${2:-}"; shift 2 ;;
        --version) version="${2:-}"; shift 2 ;;
        --out-dir) out_dir="${2:-}"; shift 2 ;;
        --smoke) smoke=1; shift ;;
        -h|--help) sed -n '2,/^set -uo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'; exit 0 ;;
        *) die_usage "unknown option $1" ;;
    esac
done
case "$platform" in linux|windows|both) ;; *) die_usage "--platform must be linux, windows or both" ;; esac
case "$win_linker" in auto|mingw|zig) ;; *) die_usage "--windows-linker must be auto, mingw or zig" ;; esac
do_linux=0; do_windows=0
[[ "$platform" == linux || "$platform" == both ]] && do_linux=1
[[ "$platform" == windows || "$platform" == both ]] && do_windows=1
[[ -n "$bin_path" && $do_linux -eq 0 ]] && die_usage "--bin needs --platform linux or both"
[[ -n "$exe_path" && $do_windows -eq 0 ]] && die_usage "--exe needs --platform windows or both"
[[ $smoke -eq 1 && $do_linux -eq 0 ]] && die_usage "--smoke runs the Linux binary; use --platform linux or both"

if [[ -t 1 ]]; then C_STEP=$'\e[36m'; C_OK=$'\e[32m'; C_FAIL=$'\e[31m'; C_OFF=$'\e[0m'
else C_STEP=""; C_OK=""; C_FAIL=""; C_OFF=""; fi
failures=()
step() { echo; echo "${C_STEP}== $*${C_OFF}"; }
info() { echo "   $*"; }
ok() { echo "${C_OK}   ok: $*${C_OFF}"; }
fail() { echo "${C_FAIL}   FAIL: $*${C_OFF}"; failures+=("$*"); }
stop_if_failed() {
    if [[ ${#failures[@]} -gt 0 ]]; then
        echo
        echo "${C_FAIL}build-release: ${#failures[@]} check(s) failed:${C_OFF}"
        for f in "${failures[@]}"; do echo "${C_FAIL}  - $f${C_OFF}"; done
        exit 1
    fi
}
need() { command -v "$1" >/dev/null 2>&1; }

# --- paths that must not end up in the binaries --------------------------

cargo_home="${CARGO_HOME:-$HOME/.cargo}"; cargo_home="${cargo_home%/}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"; rustup_home="${rustup_home%/}"
# rustc applies the last matching remap; these folders do not nest.
rust_flags="--remap-path-prefix=$REPO=. --remap-path-prefix=$cargo_home=cargo --remap-path-prefix=$rustup_home=rustup"

# Occurrences of each needle in a file, as UTF-8 and as UTF-16LE, ignoring
# ASCII case. Prints "count needle" lines for needles that occur.
count_needles() {
    python3 -I - "$@" <<'PY'
import sys
data = open(sys.argv[1], "rb").read().lower()
for needle in sys.argv[2:]:
    n = needle.lower()
    c = data.count(n.encode()) + data.count(n.encode("utf-16-le"))
    if c:
        print(c, needle)
PY
}

check_no_builder_paths() {
    local file="$1" hits
    local needles=("$HOME" "$REPO" "$cargo_home" "$rustup_home")
    # Windows tools may store the paths with backslashes too.
    local p; for p in "${needles[@]}"; do needles+=("${p//\//\\}"); done
    hits="$(count_needles "$file" "${needles[@]}")"
    if [[ -z "$hits" ]]; then
        ok "no builder paths (home, repository, cargo, rustup)"
    else
        while read -r line; do info "found $line"; done <<<"$hits"
        fail "$(basename "$file") contains the builder's paths (counts above; patterns overlap)"
    fi
}

# --- version -----------------------------------------------------------

workspace_version() {
    awk '
        /^\[/ { section = $0; next }
        section == "[workspace.package]" && /^version[ \t]*=/ {
            gsub(/^[^"]*"|".*$/, ""); print; exit
        }' "$REPO/Cargo.toml"
}

# --- Linux binary checks -----------------------------------------------

check_elf() {
    local bin="$1"
    step "Checking $bin"
    local size; size=$(stat -c %s "$bin")
    info "size: $(numfmt --grouping "$size") bytes ($(awk -v s="$size" 'BEGIN { printf "%.1f", s / 1048576 }') MiB)"
    if ! need readelf; then fail "readelf not found (sudo apt install binutils)"; return; fi

    local header; header=$(readelf -h "$bin" 2>/dev/null)
    if grep -q 'Class:.*ELF64' <<<"$header" && grep -q 'Machine:.*X86-64' <<<"$header"; then
        ok "ELF64 x86-64"
    else
        fail "not an ELF64 x86-64 binary"; return
    fi
    if readelf -S "$bin" | grep -q '\.symtab'; then
        fail "the binary has a symbol table (expected strip = true from the release profile)"
    else
        ok "stripped"
    fi

    local needed; needed=$(readelf -d "$bin" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | sort)
    info "NEEDED: $(tr '\n' ' ' <<<"$needed")"
    local bad; bad=$(grep -Ev "$ALLOWED_NEEDED" <<<"$needed" || true)
    if [[ -z "$bad" ]]; then ok "links only glibc and libgcc_s (graphics, X11, Wayland, D-Bus are opened at run time)"
    else fail "links libraries a user may not have: $(tr '\n' ' ' <<<"$bad")"; fi

    # Symbol versions the binary requires (weak ones are optional).
    local floor
    floor=$(readelf -V "$bin" | awk '
        /File: / { file = $0; sub(/.*File: /, "", file); sub(/ .*/, "", file) }
        /Name: GLIBC_[0-9]/ && /Flags: none/ && file ~ /^lib(c|m|pthread|dl|rt)\.so/ {
            v = $0; sub(/.*Name: GLIBC_/, "", v); sub(/ .*/, "", v); print v
        }' | sort -V | tail -n 1)
    if [[ -z "$floor" ]]; then
        fail "no GLIBC symbol versions found (readelf -V)"
    elif [[ "$(printf '%s\n%s\n' "$floor" "$max_glibc" | sort -V | tail -n 1)" == "$max_glibc" ]]; then
        ok "needs glibc $floor or newer (limit $max_glibc)"
    else
        fail "needs glibc $floor, newer than the limit $max_glibc (pass --max-glibc to accept)"
    fi

    check_no_builder_paths "$bin"

    local scratch; scratch=$(mktemp -d)
    local out code
    out=$(cd "$scratch" && env -u DISPLAY -u WAYLAND_DISPLAY CAPY_HOME="$scratch/home" "$bin" --help 2>&1)
    code=$?
    rm -rf -- "$scratch"
    if [[ $code -eq 0 && "$out" == *"Capybara's D2 Editor"* ]]; then ok "--help runs (exit 0)"
    else fail "--help: exit $code, output: ${out:0:200}"; fi
}

# --- Windows exe checks ------------------------------------------------

check_pe() {
    local exe="$1"
    step "Checking $exe"
    local size; size=$(stat -c %s "$exe")
    info "size: $(numfmt --grouping "$size") bytes ($(awk -v s="$size" 'BEGIN { printf "%.1f", s / 1048576 }') MiB)"
    if ! need objdump; then fail "objdump not found (sudo apt install binutils)"; return; fi
    local headers; headers=$(objdump -p "$exe" 2>/dev/null)
    if grep -q 'file format pei-x86-64' <<<"$headers" && grep -Eq '^Magic[[:space:]]+020b' <<<"$headers"; then
        ok "PE32+ x86-64"
    else
        fail "not a PE32+ x86-64 exe"; return
    fi
    if grep -Eq '^Subsystem[[:space:]]+0+2[[:space:]]' <<<"$headers"; then ok "subsystem 2 (Windows GUI)"
    else fail "not the Windows GUI subsystem ($(grep -E '^Subsystem' <<<"$headers"))"; fi

    local dlls; dlls=$(sed -n 's/^[[:space:]]*DLL Name: //p' <<<"$headers" | tr 'A-Z' 'a-z' | sort -u)
    info "imports: $(tr '\n' ' ' <<<"$dlls")"
    local bad; bad=$(grep -Ei "$FORBIDDEN_DLLS" <<<"$dlls" || true)
    if [[ -z "$bad" ]]; then ok "only system DLLs (no VC++ runtime, libgcc, winpthread)"
    else fail "imports DLLs Windows does not ship: $(tr '\n' ' ' <<<"$bad")"; fi

    if objdump -h "$exe" | grep -q '\.rsrc'; then
        # Version resource strings are UTF-16LE.
        if python3 -I -c '
import sys
d = open(sys.argv[1], "rb").read()
sys.exit(0 if all(s.encode("utf-16-le") in d for s in sys.argv[2:]) else 1)
' "$exe" "Capybara's D2 Editor" "$EXE_NAME" "$version"; then
            ok "icon and version resource (ProductName, OriginalFilename, version $version)"
        else
            fail ".rsrc without the expected version strings (ProductName, $EXE_NAME, $version)"
        fi
    else
        fail "no .rsrc section: no icon or version info (see build.rs; set CAPY_RC or install windres)"
    fi

    check_no_builder_paths "$exe"
}

# --- Windows toolchain -------------------------------------------------

zig_cmd=""
find_zig() {
    if need zig; then zig_cmd="zig"
    elif python3 -c 'import ziglang' >/dev/null 2>&1; then zig_cmd="python3 -m ziglang"
    fi
}

pick_windows_linker() {
    local have_mingw=0 have_zig=0
    need x86_64-w64-mingw32-gcc && have_mingw=1
    find_zig
    [[ -n "$zig_cmd" ]] && need cargo-zigbuild && have_zig=1
    case "$win_linker" in
        auto) if [[ $have_mingw -eq 1 ]]; then win_linker=mingw; elif [[ $have_zig -eq 1 ]]; then win_linker=zig; fi ;;
    esac
    case "$win_linker" in
        mingw) [[ $have_mingw -eq 1 ]] || { fail "x86_64-w64-mingw32-gcc not found: sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64"; return; } ;;
        zig) [[ $have_zig -eq 1 ]] || { fail "zig or cargo-zigbuild not found: pip install --user ziglang (or put zig on PATH) and cargo install --locked cargo-zigbuild"; return; } ;;
        *) fail "no Windows cross linker: sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 (or zig + cargo-zigbuild, see docs/release-checklist.md)"; return ;;
    esac
    if ! rustup target list --installed 2>/dev/null | grep -qx "$WINDOWS_TARGET"; then
        fail "Rust target $WINDOWS_TARGET is not installed: rustup target add $WINDOWS_TARGET"
    fi
}

# --- build ---------------------------------------------------------------

cargo_build() {
    local triple="$1"; shift
    local started=$SECONDS code
    info "RUSTFLAGS=$rust_flags"
    info "$* --release -p capy-app --locked --target $triple --target-dir $TARGET_DIR"
    # CARGO_ENCODED_RUSTFLAGS would win over RUSTFLAGS. --target keeps the
    # flags away from build scripts and proc macros.
    env -u CARGO_ENCODED_RUSTFLAGS RUSTFLAGS="$rust_flags" "$@" \
        --release -p capy-app --locked --target "$triple" --target-dir "$TARGET_DIR"
    code=$?
    if [[ $code -ne 0 ]]; then fail "cargo build for $triple failed (exit $code)"; stop_if_failed; fi
    ok "built $triple in $(( (SECONDS - started) / 60 )) min $(( (SECONDS - started) % 60 )) s"
}

# --- packaging -------------------------------------------------------------

source_epoch() {
    if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then echo "$SOURCE_DATE_EPOCH"
    else git -C "$REPO" log -1 --format=%ct 2>/dev/null || date +%s; fi
}

# Common documents: "source|entry" lines.
doc_files=()
collect_docs() {
    local rel
    local licenses=()
    for rel in "$REPO"/LICENSE*; do [[ -f "$rel" ]] && licenses+=("$(basename "$rel")"); done
    [[ ${#licenses[@]} -gt 0 ]] || fail "no LICENSE* file in the repository root"
    for rel in README.md "${licenses[@]}" NOTICE THIRD-PARTY-LICENSES.md docs/user-guide.md docs/cli.md; do
        if [[ -f "$REPO/$rel" ]]; then doc_files+=("$REPO/$rel|$rel"); info "$rel"
        else fail "missing $rel"; fi
    done
}
linux_extra=(
    "$REPO/assets/branding/capybara-256.png|capybara.png"
    "$REPO/packaging/linux/capybaras-d2-editor.desktop|capybaras-d2-editor.desktop"
    "$REPO/packaging/linux/install-desktop-entry.sh|install-desktop-entry.sh"
)

stage_package() {
    # stage_package <dir> <binary> <binary entry> <extra "src|entry"...>
    local dir="$1" bin="$2" entry="$3"; shift 3
    local item src rel
    mkdir -p -- "$dir"
    install -m 0755 -- "$bin" "$dir/$entry"
    for item in "${doc_files[@]}" "$@"; do
        src="${item%%|*}"; rel="${item#*|}"
        mkdir -p -- "$dir/$(dirname "$rel")"
        if [[ "$rel" == *.sh ]]; then install -m 0755 -- "$src" "$dir/$rel"
        else install -m 0644 -- "$src" "$dir/$rel"; fi
    done
    find "$dir" -type d -exec chmod 0755 {} +
}

pack_tar() {
    local stage="$1" name="$2" out="$3" epoch="$4"
    tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
        --format=gnu -C "$stage" -cf - "$name" | gzip -n -9 > "$out.tmp" && mv -f -- "$out.tmp" "$out"
}

pack_zip() {
    python3 -I - "$1" "$2" "$3" "$4" <<'PY'
import os, sys, time, zipfile
stage, name, out, epoch = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
# ZIP stores local time with 2-second steps from 1980.
stamp = time.gmtime(max(epoch, 315532800))[:6]
root = os.path.join(stage, name)
entries = []
for dirpath, dirnames, filenames in os.walk(root):
    dirnames.sort()
    for f in sorted(filenames):
        path = os.path.join(dirpath, f)
        entries.append((path, os.path.relpath(path, stage).replace(os.sep, "/")))
tmp = out + ".tmp"
with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for path, arc in sorted(entries, key=lambda e: e[1]):
        info = zipfile.ZipInfo(arc, stamp)
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = (0o100644 & 0xFFFF) << 16
        with open(path, "rb") as fh:
            z.writestr(info, fh.read(), compresslevel=9)
os.replace(tmp, out)
PY
}

report_archive() {
    local out="$1" expected="$2" count
    case "$out" in
        *.tar.gz) count=$(tar -tzf "$out" | grep -vc '/$') ;;
        *.zip) count=$(python3 -I -c 'import sys, zipfile; print(len(zipfile.ZipFile(sys.argv[1]).namelist()))' "$out") ;;
    esac
    if [[ "$count" -ne "$expected" ]]; then fail "$out holds $count files, expected $expected"; return; fi
    ok "$out ($(numfmt --grouping "$(stat -c %s "$out")") bytes, $count files)"
    info "sha256: $(sha256sum "$out" | cut -d' ' -f1)"
}

# --- smoke test ------------------------------------------------------------

smoke_test() {
    local archive="$1" name="$2"
    step "Smoke test (headless, xvfb-run + lavapipe)"
    local icd=/usr/share/vulkan/icd.d/lvp_icd.json
    need xvfb-run || { fail "xvfb-run not found (sudo apt install xvfb)"; return; }
    [[ -f "$icd" ]] || { fail "$icd not found (sudo apt install mesa-vulkan-drivers)"; return; }
    local scratch; scratch=$(mktemp -d)
    tar -xzf "$archive" -C "$scratch"
    local dir="$scratch/$name" code
    info "unpacked into $dir (CAPY_HOME unset: data goes to $dir/capybara)"
    # Never the user's own display: xvfb-run sets DISPLAY to a new server.
    local -a env_clean=(env -u DISPLAY -u WAYLAND_DISPLAY -u CAPY_HOME -u CAPY_SCREENSHOT -u CAPY_PROFILE -u CAPY_OPEN_MAP)

    local plist
    plist=$(cd "$dir" && "${env_clean[@]}" "./$BIN_NAME" profiles list 2>&1); code=$?
    sed 's/^/     /' <<<"$plist"
    if [[ $code -eq 0 ]]; then ok "profiles list (exit 0)"; else fail "profiles list: exit $code"; fi

    local -a hooks=(CAPY_SCREENSHOT="$scratch/gui.png")
    if grep -q '^vanilla[[:space:]]' <<<"$plist"; then
        hooks+=(CAPY_PROFILE=vanilla CAPY_OPEN_MAP='data\global\tiles\act1\town\towne1.ds1')
        info "opening towne1.ds1 in the vanilla profile"
    else
        info "no vanilla profile: screenshot of the empty editor"
    fi
    (cd "$dir" && "${env_clean[@]}" "${hooks[@]}" WGPU_BACKEND=vulkan VK_ICD_FILENAMES="$icd" \
        timeout 300 xvfb-run -a -s "-screen 0 1920x1080x24" "./$BIN_NAME" \
        > "$scratch/gui.log" 2>&1); code=$?
    if [[ $code -eq 0 && -s "$scratch/gui.png" && -f "$dir/capybara/config.toml" ]]; then
        ok "GUI started, wrote capybara/ and a screenshot (exit 0)"
        grep -aE 'Drew |Map catalog ready' "$scratch/gui.log" | sed -e 's/\x1b\[[0-9;]*m//g' -e 's/^.*state: /     /' || true
        local keep="$out_dir/smoke-$name.png"
        cp -f -- "$scratch/gui.png" "$keep" && info "screenshot: $keep"
    else
        fail "GUI smoke test: exit $code (log: $scratch/gui.log)"
        return
    fi
    rm -rf -- "$scratch"
}

# --- main ------------------------------------------------------------------

[[ -n "$version" ]] || version="$(workspace_version)"
[[ -n "$version" ]] || die_usage "no [workspace.package] version in Cargo.toml; pass --version"
linux_name="CapybarasD2Editor-$version-linux-x86_64"
windows_name="CapybarasD2Editor-$version-win64"
linux_out="$out_dir/$linux_name.tar.gz"
windows_out="$out_dir/$windows_name.zip"
if [[ $dry_run -eq 1 ]]; then mode="dry run"; elif [[ $skip_build -eq 1 ]]; then mode="skip build"; else mode="full build"; fi

echo "Capybara's D2 Editor release $version ($mode, $platform)"
info "repository: $REPO"
[[ $do_linux -eq 1 ]] && info "linux:      $linux_out"
[[ $do_windows -eq 1 ]] && info "windows:    $windows_out"

step "Third-party license notice"
if ! need python3; then
    fail "python3 not found (needed for scripts/third_party_licenses.py --check)"
elif (cd "$REPO" && python3 -I scripts/third_party_licenses.py --check); then
    ok "THIRD-PARTY-LICENSES.md is up to date"
else
    fail "THIRD-PARTY-LICENSES.md is stale: run python3 -I scripts/third_party_licenses.py and commit it"
fi

step "Files to package"
collect_docs
if [[ $do_linux -eq 1 ]]; then
    for item in "${linux_extra[@]}"; do
        src="${item%%|*}"
        if [[ -f "$src" ]]; then info "${item#*|}  <-  ${src#"$REPO"/}"
        else fail "missing $src"; fi
    done
fi
for tool in tar gzip sha256sum numfmt; do need "$tool" || fail "$tool not found"; done
stop_if_failed

linux_bin="${bin_path:-$TARGET_DIR/$LINUX_TARGET/release/$BIN_NAME}"
windows_exe="${exe_path:-$TARGET_DIR/$WINDOWS_TARGET/release/$EXE_NAME}"

if [[ $do_linux -eq 1 ]]; then
    step "Build Linux ($LINUX_TARGET)"
    if [[ $skip_build -eq 1 || $dry_run -eq 1 || -n "$bin_path" ]]; then
        info "skipped; would run with RUSTFLAGS=$rust_flags"
        info "cargo build --release -p capy-app --locked --target $LINUX_TARGET --target-dir $TARGET_DIR"
    else
        cargo_build "$LINUX_TARGET" cargo build
    fi
fi

if [[ $do_windows -eq 1 ]]; then
    step "Build Windows ($WINDOWS_TARGET)"
    if [[ $skip_build -eq 1 || $dry_run -eq 1 || -n "$exe_path" ]]; then
        info "skipped; would cross-build with --windows-linker $win_linker"
    else
        pick_windows_linker
        stop_if_failed
        if [[ "$win_linker" == mingw ]]; then
            info "linker: mingw-w64 (x86_64-w64-mingw32-gcc), resources: x86_64-w64-mingw32-windres"
            cargo_build "$WINDOWS_TARGET" cargo build
        else
            info "linker: zig ($zig_cmd) through cargo-zigbuild, resources: $zig_cmd rc"
            export CAPY_RC="$zig_cmd rc"
            cargo_build "$WINDOWS_TARGET" cargo zigbuild
            unset CAPY_RC
        fi
    fi
fi

if [[ $do_linux -eq 1 ]]; then
    if [[ -f "$linux_bin" ]]; then check_elf "$linux_bin"
    elif [[ $dry_run -eq 1 ]]; then info "no $linux_bin yet; the checks run after a build"
    else fail "no $linux_bin (build first, or pass --bin)"; fi
fi
if [[ $do_windows -eq 1 ]]; then
    if [[ -f "$windows_exe" ]]; then check_pe "$windows_exe"
    elif [[ $dry_run -eq 1 ]]; then info "no $windows_exe yet; the checks run after a build"
    else fail "no $windows_exe (build first, or pass --exe)"; fi
fi
stop_if_failed

step "Package"
epoch=$(source_epoch)
info "file times: $(date -u -d "@$epoch" '+%Y-%m-%d %H:%M:%S UTC') (last commit; SOURCE_DATE_EPOCH overrides)"
n_docs=${#doc_files[@]}
if [[ $dry_run -eq 1 ]]; then
    [[ $do_linux -eq 1 ]] && info "$linux_name/: $BIN_NAME, $n_docs documents, ${#linux_extra[@]} desktop files; $linux_out not written"
    [[ $do_windows -eq 1 ]] && info "$windows_name/: $EXE_NAME, $n_docs documents; $windows_out not written"
else
    mkdir -p -- "$out_dir"
    stage=$(mktemp -d)
    trap 'rm -rf -- "$stage"' EXIT
    if [[ $do_linux -eq 1 ]]; then
        stage_package "$stage/$linux_name" "$linux_bin" "$BIN_NAME" "${linux_extra[@]}"
        (cd "$stage" && find "$linux_name" -type f | sort | sed 's/^/   /')
        pack_tar "$stage" "$linux_name" "$linux_out" "$epoch" || fail "tar failed"
        report_archive "$linux_out" $((1 + n_docs + ${#linux_extra[@]}))
    fi
    if [[ $do_windows -eq 1 ]]; then
        stage_package "$stage/$windows_name" "$windows_exe" "$EXE_NAME"
        (cd "$stage" && find "$windows_name" -type f | sort | sed 's/^/   /')
        pack_zip "$stage" "$windows_name" "$windows_out" "$epoch" || fail "zip failed"
        report_archive "$windows_out" $((1 + n_docs))
    fi
    stop_if_failed
fi

if [[ $smoke -eq 1 ]]; then
    if [[ $dry_run -eq 1 ]]; then info "dry run: no smoke test"
    else smoke_test "$linux_out" "$linux_name"; fi
    stop_if_failed
fi

echo
echo "${C_OK}build-release: done ($mode)${C_OFF}"
exit 0
