#!/usr/bin/env python3
"""Generate THIRD-PARTY-LICENSES.md for the release exe.

Usage:
    python -I scripts/third_party_licenses.py            # write the notice
    python -I scripts/third_party_licenses.py --check    # exit 1 if it is stale
    python -I scripts/third_party_licenses.py --out FILE

The release exe is `cargo build --release -p capy-app` for
x86_64-pc-windows-msvc, and it links every Rust dependency statically. This
script finds the packages that end up in it and collects their licenses:

1. `cargo metadata --format-version 1 --locked --filter-platform <target>`
   gives the packages, their manifests and the resolve graph.
2. Starting at capy-app, it follows normal dependencies only. Dev-dependencies
   are dropped. Build-dependencies and proc-macros (and everything under them)
   run on the build machine and do not ship; they are listed separately.
3. `cargo metadata` resolves features for the whole workspace, so it can
   include optional packages that another member turns on.
   `cargo tree -p capy-app -e normal,no-proc-macro` resolves features the way
   the release build does, and its package set prunes the walk. Pass
   `--no-tree` to skip this and keep the larger, workspace-wide set.
4. License texts come from each package's source directory (the folder of
   `manifest_path` in the cargo registry): files named LICENSE*, LICENCE*,
   COPYING*, COPYRIGHT*, UNLICENSE* and NOTICE*, plus `license-file`.

Data files, fonts and bundled C code that are not Rust packages are described
by the curated tables near the top of this file. The script checks that they
still match the tree (embedded files, font files, native build scripts) and
flags anything it does not recognise.

Neither command compiles anything. Standard library only. The output is
deterministic: sorted by name and version, LF line endings, no local paths.
"""

import argparse
import hashlib
import json
import re
import struct
import subprocess
import sys
from pathlib import Path

TARGET = "x86_64-pc-windows-msvc"
ROOT_CRATE = "capy-app"
REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "THIRD-PARTY-LICENSES.md"

# --------------------------------------------------------------------------
# License policy
# --------------------------------------------------------------------------

# Rank 0: permissive. Attribution only (keep the copyright and license text).
PERMISSIVE = {
    "0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "BSL-1.0",
    "CC0-1.0", "ISC", "MIT", "MIT-0", "NCSA", "Unicode-3.0",
    "Unicode-DFS-2016", "Unlicense", "Zlib", "bzip2-1.0.6",
}
# Exceptions that only add permissions.
PERMISSIVE_EXCEPTIONS = {"LLVM-exception"}
# Rank 1: allowed, with conditions beyond attribution. Each has a note.
CONDITIONS = {
    "MPL-2.0": (
        "MPL-2.0 is file-level (weak) copyleft. The exe may stay MIT, but the "
        "MPL-covered source files must remain under MPL-2.0: anyone who gets "
        "the exe must be told how to get the source of those files (a link to "
        "the crate's published source is enough), including any changes we "
        "make to them. Unmodified use from crates.io needs only this notice "
        "and the link."
    ),
    "OFL-1.1": (
        "SIL Open Font License 1.1. Embedding the font in a program is "
        "allowed. The copyright notice and the license must accompany it, the "
        "font may not be sold on its own, and a modified font may not use the "
        "Reserved Font Names."
    ),
    "Ubuntu-font-1.0": (
        "Ubuntu Font Licence 1.0. Embedding the font is allowed. The copyright "
        "notice and the licence must accompany it, and a modified font must "
        "be renamed and stay under the same licence."
    ),
}
# Rank 3: strong or weak copyleft families that need a decision.
COPYLEFT_PREFIXES = ("GPL-", "LGPL-", "AGPL-", "EUPL-", "CDDL-", "EPL-", "OSL-",
                     "CPL-", "SSPL-")

RANK_OK, RANK_NOTE, RANK_UNKNOWN, RANK_COPYLEFT = 0, 1, 2, 3


def id_rank(ident):
    if ident in PERMISSIVE:
        return RANK_OK
    if ident in CONDITIONS:
        return RANK_NOTE
    if ident.startswith(COPYLEFT_PREFIXES):
        return RANK_COPYLEFT
    return RANK_UNKNOWN


class Expr:
    """A parsed SPDX expression: ("id", name, exception) / ("or"|"and", [..])."""

    TOKEN = re.compile(r"\s*(\(|\)|[A-Za-z0-9.+:\-]+)")

    def __init__(self, text):
        self.text = text
        norm = text.replace("/", " OR ")
        self.tokens = []
        pos = 0
        while pos < len(norm):
            m = self.TOKEN.match(norm, pos)
            if not m:
                if norm[pos:].strip() == "":
                    break
                raise ValueError(f"cannot parse license expression {text!r}")
            self.tokens.append(m.group(1))
            pos = m.end()
        self.pos = 0
        self.tree = self._or()
        if self.pos != len(self.tokens):
            raise ValueError(f"cannot parse license expression {text!r}")

    def _peek(self):
        return self.tokens[self.pos] if self.pos < len(self.tokens) else None

    def _or(self):
        items = [self._and()]
        while (self._peek() or "").upper() == "OR":
            self.pos += 1
            items.append(self._and())
        return items[0] if len(items) == 1 else ("or", items)

    def _and(self):
        items = [self._atom()]
        while (self._peek() or "").upper() == "AND":
            self.pos += 1
            items.append(self._atom())
        return items[0] if len(items) == 1 else ("and", items)

    def _atom(self):
        tok = self._peek()
        if tok is None:
            raise ValueError(f"cannot parse license expression {self.text!r}")
        self.pos += 1
        if tok == "(":
            node = self._or()
            if self._peek() != ")":
                raise ValueError(f"unbalanced parentheses in {self.text!r}")
            self.pos += 1
            return node
        exc = None
        if (self._peek() or "").upper() == "WITH":
            self.pos += 1
            exc = self._peek()
            self.pos += 1
        return ("id", tok, exc)

    def ids(self, node=None):
        node = self.tree if node is None else node
        if node[0] == "id":
            return [node[1]] + ([node[2]] if node[2] else [])
        return [i for child in node[1] for i in self.ids(child)]

    def rank(self, node=None):
        """Best rank reachable by electing OR branches."""
        node = self.tree if node is None else node
        if node[0] == "id":
            r = id_rank(node[1])
            if node[2] and node[2] not in PERMISSIVE_EXCEPTIONS:
                r = max(r, RANK_UNKNOWN)
            return r
        ranks = [self.rank(child) for child in node[1]]
        return min(ranks) if node[0] == "or" else max(ranks)


# --------------------------------------------------------------------------
# Curated: data and code in the exe that are not cargo packages
# --------------------------------------------------------------------------

DS1EDIT_MIT = """\
MIT License

Copyright (c) 2025-2026 Ben Ethington

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""

# Repo-relative files that a shipped crate embeds with include_str!/include_bytes!.
ASSETS = {
    "assets/editor/obj.txt": {
        "title": "DS1 object table (`assets/editor/obj.txt`)",
        "license": "MIT (DS1Edit), over an unlicensed original by Paul Siramy",
        "body": (
            "It maps DS1 object and monster ids to the COF token, mode and class the preview "
            "needs. It comes from DS1Edit by Ben Ethington "
            "(https://github.com/bethington/d2-ds1-edit), which is MIT "
            "licensed; its license is reproduced below.\n\n"
            "DS1Edit's own NOTICE says this table is Paul Siramy's, from "
            "win_ds1edit (http://paul.siramy.free.fr/_divers/ds1/dl_ds1edit.html), "
            "and that DS1Edit's MIT license covers only DS1Edit's changes and "
            "is not a relicensing of Paul's work. win_ds1edit was published "
            "with its source from 2002 on, with no license text and no "
            "copyright notice. The table is used here on the same basis as "
            "DS1Edit uses it: with attribution to Paul Siramy."
        ),
        "text": DS1EDIT_MIT,
    },
    "assets/known-names/seed.txt": {
        "title": "Known-names seed (`assets/known-names/seed.txt`)",
        "license": "list of game file names; sources credited",
        "body": (
            "A list of about 200 Diablo II game file paths that the game or "
            "the editor loads by hard-coded name. "
            "It holds names only, no game data. The names were collected from "
            "DS1Edit's format guides and source string literals (MIT, license "
            "above) and from common Diablo II conventions; see "
            "`assets/known-names/README.md`. The files they name belong to "
            "Blizzard Entertainment and are not distributed with this program."
        ),
        "text": None,
    },
}
# Listed in NOTICE as "if present"; described only when the file exists.
OPTIONAL_ASSETS = {
    "assets/editor/ds1edit.dt1": (
        "DS1Edit editor marker tiles (`assets/editor/ds1edit.dt1`). Drawn by "
        "Paul Siramy for win_ds1edit and kept in DS1Edit; same terms as "
        "`obj.txt` above."
    ),
}

# Fonts compiled in by egui's `default_fonts` feature. ttf -> license file and
# SPDX id. The script fails if the crate's fonts/ folder no longer matches.
FONT_CRATE = "epaint_default_fonts"
FONTS = {
    "Hack-Regular.ttf": ("Hack-Regular.txt", "MIT AND Bitstream-Vera",
                         "Hack (Source Foundry), monospace text"),
    "NotoEmoji-Regular.ttf": ("OFL.txt", "OFL-1.1",
                              "Noto Emoji (Google), emoji glyphs"),
    "Ubuntu-Light.ttf": ("UFL.txt", "Ubuntu-font-1.0",
                         "Ubuntu Light (Canonical), proportional text"),
    "emoji-icon-font.ttf": ("emoji-icon-font-mit-license.txt", "MIT",
                            "emoji-icon-font (John Slegers), icon glyphs"),
}

# Shipped crates whose build script can compile C or assembly into the exe.
# Keyed by crate name; the callable gets the resolved feature list.
NATIVE = {
    "libsqlite3-sys": lambda feats: (
        "SQLite, compiled from the amalgamation bundled in the crate "
        "(`bundled` feature). SQLite is in the public domain."
        if "bundled" in feats else
        "links a system SQLite (no `bundled` feature): check how it ships."
    ),
    "blake3": lambda feats: (
        "`pure` feature: no C or assembly is compiled; Rust only."
        if "pure" in feats else
        "compiles the crate's own C/assembly, under the crate's license."
    ),
}
NATIVE_BUILD_TOOLS = {"cc", "cmake", "nasm-rs", "bindgen"}

# Shipped crates that embed third-party data under terms of its own.
CRATE_DATA = {
    "jiff-tzdb": (
        "embeds the IANA Time Zone Database (https://www.iana.org/time-zones), "
        "which is in the public domain. The crate's code is `Unlicense OR MIT`."
    ),
}

# --------------------------------------------------------------------------
# Helpers
# --------------------------------------------------------------------------

LICENSE_FILE_RE = re.compile(r"^(licen[cs]e|copying|copyright|unlicense|notice)",
                             re.IGNORECASE)


def version_key(version):
    parts = []
    for piece in re.split(r"[.+\-]", version):
        parts.append((0, int(piece), "") if piece.isdigit() else (1, 0, piece))
    return tuple(parts)


def pkg_key(pkg):
    return (pkg["name"].lower(), version_key(pkg["version"]))


def label(pkg):
    return f"{pkg['name']} {pkg['version']}"


def run(cmd):
    proc = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True,
                          encoding="utf-8", errors="replace")
    if proc.returncode != 0:
        sys.exit(f"error: {' '.join(cmd)} failed:\n{proc.stderr}")
    return proc.stdout


def read_text(path):
    raw = path.read_bytes()
    try:
        text = raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        text = raw.decode("latin-1")
    lines = [line.rstrip() for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n")]
    return "\n".join(lines).strip("\n")


def text_key(text):
    return hashlib.sha256(" ".join(text.split()).encode("utf-8")).hexdigest()


def fence(text, lang="text"):
    longest = max((len(m) for m in re.findall(r"`+", text)), default=0)
    ticks = "`" * max(3, longest + 1)
    return f"{ticks}{lang}\n{text}\n{ticks}"


def text_family(text):
    t = " ".join(text.split()).lower()
    found = []
    if "apache license" in t and "version 2.0" in t:
        found.append("Apache-2.0")
    # BSL-1.0 and the Unicode license open with the same words as MIT.
    if "permission is hereby granted, free of charge" in t             and "boost software license" not in t             and not ("unicode" in t and "data files" in t):
        found.append("MIT")
    if "redistribution and use in source and binary forms" in t:
        found.append("BSD")
    if "permission to use, copy, modify, and/or distribute this software" in t \
            or "permission to use, copy, modify, and distribute this software" in t:
        found.append("ISC")
    if "altered source versions must be plainly marked" in t:
        found.append("Zlib")
    if "boost software license" in t:
        found.append("BSL-1.0")
    if "mozilla public license" in t:
        found.append("MPL-2.0")
    if "unicode" in t and "data files" in t:
        found.append("Unicode")
    if "free and unencumbered software released into the public domain" in t:
        found.append("Unlicense")
    if "cc0" in t or "creative commons" in t:
        found.append("CC0-1.0")
    # The MPL-2.0 text names the GPL family as "Secondary Licenses".
    if ("gnu general public license" in t or "gnu lesser general public" in t)             and "mozilla public license" not in t:
        found.append("GPL family")
    if not found:
        return "Other"
    return found[0] if len(found) == 1 else " + ".join(found)


FAMILY_ORDER = ["Apache-2.0", "MIT", "BSD", "ISC", "Zlib", "BSL-1.0", "Unicode",
                "Unlicense", "CC0-1.0", "MPL-2.0", "GPL family"]


def family_sort(fam):
    return FAMILY_ORDER.index(fam) if fam in FAMILY_ORDER else len(FAMILY_ORDER)


def anchor(text):
    return re.sub(r"[^a-z0-9 \-]", "", text.lower()).replace(" ", "-")


def ttf_names(path):
    """Copyright (nameID 0) and version (nameID 5) from a TrueType name table."""
    data = path.read_bytes()
    (num_tables,) = struct.unpack(">H", data[4:6])
    for i in range(num_tables):
        tag, _cs, off, _ln = struct.unpack(">4sIII", data[12 + 16 * i:28 + 16 * i])
        if tag != b"name":
            continue
        _fmt, count, str_off = struct.unpack(">HHH", data[off:off + 6])
        best = {}
        for j in range(count):
            rec = data[off + 6 + 12 * j:off + 18 + 12 * j]
            pid, eid, lid, nid, length, noff = struct.unpack(">6H", rec)
            if nid not in (0, 5):
                continue
            raw = data[off + str_off + noff:off + str_off + noff + length]
            if pid in (0, 3):
                value, prio = raw.decode("utf-16-be", "replace"), (0 if lid == 0x409 else 1)
            elif pid == 1:
                value, prio = raw.decode("mac_roman", "replace"), 2
            else:
                continue
            if nid not in best or prio < best[nid][0]:
                best[nid] = (prio, " ".join(value.split()))
        version = best.get(5, (0, ""))[1].split(";")[0].strip()
        return best.get(0, (0, ""))[1], version
    return "", ""


# --------------------------------------------------------------------------
# Collect
# --------------------------------------------------------------------------

def tree_set(edges):
    out = run(["cargo", "tree", "--locked", "-p", ROOT_CRATE, "-e", edges,
               "--target", TARGET, "--prefix", "none", "-f", "{p}"])
    found = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            found.add((parts[0], parts[1][1:]))
    return found


def collect(meta, use_tree):
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    roots = [i for i in members if pkgs[i]["name"] == ROOT_CRATE]
    if len(roots) != 1:
        sys.exit(f"error: workspace member {ROOT_CRATE} not found")

    def is_proc_macro(pid):
        return any("proc-macro" in t["kind"] for t in pkgs[pid]["targets"])

    shipped, host_roots, stack = set(), [], [roots[0]]
    while stack:
        pid = stack.pop()
        if pid in shipped:
            continue
        shipped.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {k["kind"] for k in dep["dep_kinds"]}
            if None in kinds:
                (host_roots if is_proc_macro(dep["pkg"]) else stack).append(dep["pkg"])
            if "build" in kinds:
                host_roots.append(dep["pkg"])
    host, stack = set(), list(host_roots)
    while stack:
        pid = stack.pop()
        if pid in host:
            continue
        host.add(pid)
        for dep in nodes[pid]["deps"]:
            if {k["kind"] for k in dep["dep_kinds"]} & {None, "build"}:
                stack.append(dep["pkg"])
    host -= shipped

    pruned = set()
    if use_tree:
        ship_tree = tree_set("normal,no-proc-macro")
        build_tree = tree_set("normal,build")
        key = lambda pid: (pkgs[pid]["name"], pkgs[pid]["version"])
        pruned = {pid for pid in shipped if key(pid) not in ship_tree}
        shipped -= pruned
        missing = ship_tree - {key(pid) for pid in shipped}
        if missing:
            sys.exit(f"error: cargo tree lists packages the metadata walk missed: {sorted(missing)}")
        host = {pid for pid in host if key(pid) in build_tree}
        pruned -= host

    return pkgs, nodes, members, shipped, host, pruned


def license_files(pkg):
    root = Path(pkg["manifest_path"]).parent
    found = {}
    for entry in sorted(root.iterdir(), key=lambda p: p.name.lower()):
        if not LICENSE_FILE_RE.match(entry.name):
            continue
        if entry.is_file():
            found[entry.name] = entry
        elif entry.is_dir():  # e.g. LICENSES/ (REUSE layout)
            for sub in sorted(entry.rglob("*"), key=lambda p: str(p).lower()):
                if sub.is_file():
                    found[sub.relative_to(root).as_posix()] = sub
    if pkg.get("license_file"):
        lf = (root / pkg["license_file"]).resolve()
        if lf.is_file():
            rel = lf.relative_to(root).as_posix() if lf.is_relative_to(root) else lf.name
            found.setdefault(rel, lf)
    return [(name, read_text(path)) for name, path in sorted(found.items(), key=lambda kv: kv[0].lower())]


def classify(pkg, files):
    """(rank, reason) for the flagged list, or None if nothing to say."""
    lic = pkg.get("license")
    if not lic:
        if pkg.get("license_file"):
            return RANK_UNKNOWN, f"no SPDX expression; license file `{pkg['license_file']}` must be read by hand"
        return RANK_UNKNOWN, "no license declared"
    try:
        expr = Expr(lic)
    except ValueError as err:
        return RANK_UNKNOWN, str(err)
    rank = expr.rank()
    ids = sorted(set(expr.ids()))
    notes = []
    if rank == RANK_COPYLEFT:
        bad = [i for i in ids if id_rank(i) == RANK_COPYLEFT]
        notes.append(f"copyleft: {', '.join(bad)}. Needs a decision before release")
    elif rank == RANK_UNKNOWN:
        bad = [i for i in ids if id_rank(i) == RANK_UNKNOWN and i not in PERMISSIVE_EXCEPTIONS]
        notes.append(f"unrecognised identifier(s): {', '.join(bad)}. Review by hand")
    elif rank == RANK_NOTE:
        cond = [i for i in ids if i in CONDITIONS]
        notes.append(f"conditions apply: {', '.join(cond)} (see notes below)")
        if "MPL-2.0" in cond:
            notes.append("source of the MPL files: "
                         f"https://crates.io/crates/{pkg['name']}/{pkg['version']}"
                         + (f" and {pkg['repository']}" if pkg.get("repository") else ""))
    if not [f for f in files if not f[0].upper().startswith("NOTICE")]:
        notes.append("the crate ships no license text; standard terms and "
                     "authors are listed at the end of section 5")
        rank = max(rank, RANK_NOTE)
    if not notes:
        return None
    return rank, "; ".join(notes)


def copyleft_options(pkg):
    lic = pkg.get("license")
    if not lic:
        return []
    try:
        expr = Expr(lic)
    except ValueError:
        return []
    if expr.rank() != RANK_OK:
        return []
    return sorted({i for i in expr.ids() if id_rank(i) in (RANK_COPYLEFT, RANK_NOTE)})


def embedded_files(meta, pkgs, shipped, members):
    """Repo files outside their crate that shipped workspace crates embed."""
    inc = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')
    found = {}
    for pid in shipped & members:
        crate_dir = Path(pkgs[pid]["manifest_path"]).parent
        src = crate_dir / "src"
        if not src.is_dir():
            continue
        for rs in sorted(src.rglob("*.rs")):
            for m in inc.finditer(rs.read_text(encoding="utf-8", errors="replace")):
                target = (rs.parent / m.group(1)).resolve()
                if target.is_relative_to(crate_dir):
                    continue
                rel = target.relative_to(REPO).as_posix() if target.is_relative_to(REPO) else target.name
                found.setdefault(rel, set()).add(pkgs[pid]["name"])
    return found


# --------------------------------------------------------------------------
# Render
# --------------------------------------------------------------------------

def render(meta, use_tree):
    pkgs, nodes, members, shipped, host, pruned = collect(meta, use_tree)
    third = sorted((pkgs[i] for i in shipped - members), key=pkg_key)
    host_pkgs = sorted((pkgs[i] for i in host - members), key=pkg_key)
    pruned_pkgs = sorted((pkgs[i] for i in pruned - members), key=pkg_key)
    first_party = sorted(pkgs[i]["name"] for i in shipped & members)
    by_name = {p["name"]: p for p in third}
    features = {pkgs[n["id"]]["name"]: n.get("features", []) for n in meta["resolve"]["nodes"]
                if n["id"] in shipped}

    files = {p["id"]: license_files(p) for p in third}
    flags = []
    for p in third:
        c = classify(p, files[p["id"]])
        if c:
            flags.append((p, c[0], c[1]))

    # Bundled native code.
    native = []
    for p in third:
        node = nodes[p["id"]]
        tools = sorted({pkgs[d["pkg"]]["name"] for d in node["deps"]
                        if any(k["kind"] == "build" for k in d["dep_kinds"])
                        and pkgs[d["pkg"]]["name"] in NATIVE_BUILD_TOOLS})
        if not tools and p["name"] not in NATIVE:
            continue
        if p["name"] in NATIVE:
            native.append((p, NATIVE[p["name"]](features.get(p["name"], [])), tools))
        else:
            native.append((p, None, tools))
            flags.append((p, RANK_UNKNOWN, f"build script uses {', '.join(tools)} and may "
                          "compile native code; add it to NATIVE in the script after review"))

    # Embedded data files.
    embedded = embedded_files(meta, pkgs, shipped, members)
    unknown_assets = sorted(set(embedded) - set(ASSETS))
    stale_assets = sorted(set(ASSETS) - set(embedded))
    if stale_assets:
        sys.exit(f"error: ASSETS lists files no shipped crate embeds: {stale_assets}")

    # Fonts.
    fonts = []
    font_pkg = by_name.get(FONT_CRATE)
    if font_pkg:
        font_dir = Path(font_pkg["manifest_path"]).parent / "fonts"
        ttfs = sorted(p.name for p in font_dir.glob("*.ttf"))
        if ttfs != sorted(FONTS):
            sys.exit(f"error: {FONT_CRATE} fonts changed: {ttfs} vs {sorted(FONTS)}; update FONTS")
        for ttf in ttfs:
            lic_name, spdx, desc = FONTS[ttf]
            copyright_, version = ttf_names(font_dir / ttf)
            fonts.append((ttf, desc, spdx, copyright_, version, lic_name,
                          read_text(font_dir / lic_name)))

    flags.sort(key=lambda f: (-f[1], pkg_key(f[0])))
    out = []
    w = out.append

    w("# Third-party licenses")
    w("")
    w("Capybara Map Editor is MIT licensed (see [LICENSE](LICENSE)). Its release "
      "exe, built with `cargo build --release -p capy-app` for "
      f"`{TARGET}`, statically links the third-party Rust packages listed "
      "here and embeds the data files and fonts described in section 3. This "
      "file reproduces their licenses and notices, as those licenses require.")
    w("")
    w("This file is generated by `python -I scripts/third_party_licenses.py` "
      "from `Cargo.lock`. Do not edit it by hand: re-run the script after any "
      "dependency change (`--check` exits 1 when this file is stale). For "
      "credits and the history of the file formats, see [NOTICE](NOTICE).")
    w("")
    w(f"- Third-party packages in the exe: **{len(third)}**")
    w(f"- First-party workspace crates in the exe (MIT, not listed): {', '.join(first_party)}")
    w(f"- Build-time only, not shipped: {len(host_pkgs)} (section 7)")
    w(f"- Flagged for review: {len(flags)} (section 2)")
    w("")
    w("Contents:")
    w("")
    sections = ["1. Summary by license", "2. Flagged packages",
                "3. Data, fonts and native code", "4. Package list",
                "5. License texts", "6. NOTICE files",
                "7. Build-time packages (not shipped)"]
    for s in sections:
        w(f"- [{s}](#{anchor(s)})")
    w("")

    # 1. Summary
    groups = {}
    for p in third:
        groups.setdefault(p.get("license") or "(none)", []).append(p)
    w("## 1. Summary by license")
    w("")
    w("| License expression | Packages |")
    w("|---|---:|")
    for lic in sorted(groups, key=lambda l: (-len(groups[l]), l)):
        w(f"| [`{lic}`](#{anchor('license ' + lic)}) | {len(groups[lic])} |")
    w(f"| **Total** | **{len(third)}** |")
    w("")
    for lic in sorted(groups, key=lambda l: (-len(groups[l]), l)):
        w(f"### License: {lic}")
        w("")
        w(", ".join(f"`{label(p)}`" for p in groups[lic]))
        w("")

    # 2. Flagged
    w("## 2. Flagged packages")
    w("")
    w("Allowed without review: " + ", ".join(sorted(PERMISSIVE)) + ", and "
      "`Apache-2.0 WITH LLVM-exception`. For an `OR` expression, the most "
      "permissive branch is elected. Everything else is listed here: copyleft "
      "(GPL, LGPL, AGPL and the like), unknown or missing licenses, licenses "
      "with conditions beyond attribution, and crates that ship no license text.")
    w("")
    severity = {RANK_COPYLEFT: "copyleft", RANK_UNKNOWN: "review",
                RANK_NOTE: "note"}
    if flags:
        w("| Package | License | Level | Why |")
        w("|---|---|---|---|")
        for p, rank, why in flags:
            w(f"| `{label(p)}` | `{p.get('license') or '(none)'}` | {severity[rank]} | {why} |")
    else:
        w("Nothing is flagged.")
    w("")
    copyleft_found = any(r == RANK_COPYLEFT for _, r, _ in flags)
    w(f"No package requires a copyleft license: **{'no, see above' if copyleft_found else 'yes'}**.")
    w("")
    used_conditions = sorted({i for p, _, _ in flags if p.get("license")
                              for i in Expr(p["license"]).ids() if i in CONDITIONS}
                             | {i for f in fonts for i in Expr(f[2]).ids() if i in CONDITIONS})
    if used_conditions:
        w("What the conditions require:")
        w("")
        for i in used_conditions:
            w(f"- **{i}**: {CONDITIONS[i]}")
        w("")
    options = [(p, copyleft_options(p)) for p in third]
    options = [(p, o) for p, o in options if o]
    if options:
        w("Dual-licensed packages that also offer a copyleft or conditional "
          "license; the permissive branch is elected:")
        w("")
        for p, o in options:
            w(f"- `{label(p)}`: `{p['license']}` (not elected: {', '.join(o)}; "
              "the text of the license not elected is left out of section 5)")
        w("")
    if unknown_assets:
        w("Embedded files with no entry in the script's ASSETS table (review them):")
        w("")
        for a in unknown_assets:
            w(f"- `{a}` (embedded by {', '.join(sorted(embedded[a]))})")
        w("")

    # 3. Data, fonts, native code
    w("## 3. Data, fonts and native code")
    w("")
    w("These are compiled into the exe but are not cargo packages, or are "
      "inside a package but carry their own license.")
    w("")
    for rel in sorted(ASSETS):
        a = ASSETS[rel]
        w(f"### {a['title']}")
        w("")
        w(f"License: {a['license']}. Embedded by: {', '.join(sorted(embedded[rel]))}.")
        w("")
        w(a["body"])
        w("")
        if a["text"]:
            w(fence(a["text"]))
            w("")
    for rel in sorted(OPTIONAL_ASSETS):
        if (REPO / rel).is_file():
            w(f"### `{rel}`")
            w("")
            w(OPTIONAL_ASSETS[rel])
            w("")
    if fonts:
        w(f"### Fonts embedded by egui (`{label(font_pkg)}`)")
        w("")
        w("`bevy_egui`'s `default_fonts` feature turns on egui's built-in fonts. "
          f"The `{FONT_CRATE}` crate (`{font_pkg['license']}`) compiles these "
          "font files into the exe with `include_bytes!`. Copyright and "
          "version below are read from each font's own `name` table. Bevy's "
          "text stack (`bevy_text`) is not enabled, so Bevy embeds no font.")
        w("")
        w("| Font file | Use | License | Copyright (from the font) | Version | License file |")
        w("|---|---|---|---|---|---|")
        for ttf, desc, spdx, cr, ver, lic_name, _ in fonts:
            w(f"| `{ttf}` | {desc} | `{spdx}` | {cr or '(none in font)'} | {ver} | `fonts/{lic_name}` |")
        w("")
        seen = set()
        for ttf, desc, spdx, cr, ver, lic_name, text in fonts:
            if lic_name in seen:
                continue
            seen.add(lic_name)
            users = [f[0] for f in fonts if f[5] == lic_name]
            w(f"#### `fonts/{lic_name}` (for {', '.join(f'`{u}`' for u in users)})")
            w("")
            w(fence(text))
            w("")
    crate_data = [(by_name[n], CRATE_DATA[n]) for n in sorted(CRATE_DATA) if n in by_name]
    if crate_data:
        w("### Data embedded by Rust packages")
        w("")
        for p, what in crate_data:
            w(f"- `{label(p)}` {what}")
        w("")
    if native:
        w("### Native code compiled into the exe")
        w("")
        w("| Package | Build tools | What is compiled |")
        w("|---|---|---|")
        for p, what, tools in native:
            w(f"| `{label(p)}` | {', '.join(tools) or '-'} | {what or 'unreviewed'} |")
        w("")
        sqlite = by_name.get("libsqlite3-sys")
        if sqlite and "bundled" in features.get("libsqlite3-sys", []):
            amalg = Path(sqlite["manifest_path"]).parent / "sqlite3" / "sqlite3.c"
            if amalg.is_file():
                head = amalg.read_text(encoding="utf-8", errors="replace")[:20000]
                ver = re.search(r"version (\d+\.\d+\.\d+)", head)
                bless = re.search(r"\*\* (The author disclaims copyright.*?)\n\*\*\n\*{5}", head, re.S)
                w(f"SQLite {ver.group(1) if ver else ''} (https://sqlite.org/), "
                  "bundled by `libsqlite3-sys`, carries this dedication instead of a license:")
                w("")
                if bless:
                    lines = [re.sub(r"^\*\* ?", "", l) for l in bless.group(1).split("\n")]
                    w(fence("\n".join(l.rstrip() for l in lines).strip()))
                    w("")

    # 4. Package list
    w("## 4. Package list")
    w("")
    w("| Package | Version | License | Repository | License files in the crate |")
    w("|---|---|---|---|---|")
    for p in third:
        repo = p.get("repository") or p.get("homepage") or ""
        names = ", ".join(f"`{n}`" for n, _ in files[p["id"]]) or "**none**"
        w(f"| {p['name']} | {p['version']} | `{p.get('license') or '(none)'}` | {repo} | {names} |")
    w("")

    # 5. License texts
    texts = {}
    for p in third:
        for name, text in files[p["id"]]:
            if name.upper().startswith("NOTICE") or not text.strip():
                continue
            # A copyleft alternative that is not elected (see section 2).
            if text_family(text) == "GPL family" and p.get("license")                     and Expr(p["license"]).rank() == RANK_OK:
                continue
            k = text_key(text)
            entry = texts.setdefault(k, {"text": text, "users": []})
            entry["users"].append((p, name))
    ordered = sorted(texts.values(), key=lambda e: (family_sort(text_family(e["text"])),
                                                   text_family(e["text"]),
                                                   pkg_key(e["users"][0][0]),
                                                   e["users"][0][1].lower()))
    w("## 5. License texts")
    w("")
    w(f"Each distinct license text appears once ({len(ordered)} texts), "
      "followed by the packages that ship it and the file name in each "
      "package. Texts that differ only in whitespace are merged. Dual-licensed "
      "packages appear under each of their texts.")
    w("")
    for n, e in enumerate(ordered, 1):
        fam = text_family(e["text"])
        w(f"### Text {n}: {fam}")
        w("")
        users = sorted(e["users"], key=lambda u: (pkg_key(u[0]), u[1].lower()))
        w("Used by: " + ", ".join(f"`{label(p)}` ({name})" for p, name in users))
        w("")
        w(fence(e["text"]))
        w("")

    bare = [p for p in third
            if not [f for f in files[p["id"]] if not f[0].upper().startswith("NOTICE")]]
    if bare:
        first_of = {}
        for n, e in enumerate(ordered, 1):
            first_of.setdefault(text_family(e["text"]), n)
        spdx_family = {"MIT": "MIT", "Apache-2.0": "Apache-2.0", "BSL-1.0": "BSL-1.0",
                       "ISC": "ISC", "Zlib": "Zlib", "Unicode-3.0": "Unicode",
                       "Unlicense": "Unlicense", "CC0-1.0": "CC0-1.0", "MPL-2.0": "MPL-2.0",
                       "BSD-2-Clause": "BSD", "BSD-3-Clause": "BSD"}
        w("### Packages that ship no license text")
        w("")
        w("These crates declare a license in `Cargo.toml` but their published "
          "package has no license file. The standard terms of each license "
          "apply; the column on the right points to a copy of those terms "
          "above. Copyright is held by the authors named in the crate's "
          "`Cargo.toml` (shown here) or, when none are named, by the "
          "contributors of its repository. Section 3 has the font licenses for "
          f"`{FONT_CRATE}`.")
        w("")
        w("| Package | License | Authors (Cargo.toml) | Repository | Standard terms |")
        w("|---|---|---|---|---|")
        for p in bare:
            ids = sorted(set(Expr(p["license"]).ids())) if p.get("license") else []
            refs = [f"{i}: Text {first_of[spdx_family[i]]}" for i in ids
                    if i in spdx_family and spdx_family[i] in first_of]
            terms = "; ".join(refs) or "standard SPDX text"
            if p["name"] == FONT_CRATE:
                terms += "; fonts: section 3"
            authors = ", ".join(re.sub(r"\s*<[^>]*>", "", a) for a in p.get("authors") or []) \
                or "(none named)"
            w(f"| `{label(p)}` | `{p.get('license') or '(none)'}` | {authors} | "
              f"{p.get('repository') or ''} | {terms} |")
        w("")

    # 6. NOTICE files
    notices = {}
    for p in third:
        for name, text in files[p["id"]]:
            if name.upper().startswith("NOTICE") and text.strip():
                notices.setdefault(text_key(text), {"text": text, "users": []})["users"].append((p, name))
    w("## 6. NOTICE files")
    w("")
    w("Apache-2.0 section 4(d) requires that a NOTICE file shipped with a "
      "work be reproduced with it. These are all the NOTICE files found in "
      "shipped packages.")
    w("")
    if not notices:
        w("No shipped package has a NOTICE file.")
        w("")
    for e in sorted(notices.values(), key=lambda e: pkg_key(e["users"][0][0])):
        users = sorted(e["users"], key=lambda u: pkg_key(u[0]))
        w(f"### NOTICE: {', '.join(label(p) for p, _ in users)}")
        w("")
        w(fence(e["text"]))
        w("")

    # 7. Build-only
    w("## 7. Build-time packages (not shipped)")
    w("")
    w("Proc-macros, build scripts and their dependencies run on the build "
      "machine only. Their code is not in the exe, so their licenses impose "
      "nothing on it. Listed for completeness.")
    w("")
    w("| Package | Version | License |")
    w("|---|---|---|")
    for p in host_pkgs:
        w(f"| {p['name']} | {p['version']} | `{p.get('license') or '(none)'}` |")
    w("")
    if pruned_pkgs:
        w("These packages are in the workspace-wide resolve but are not built "
          "for `-p capy-app` (another workspace member or a dev-dependency "
          "enables them): " + ", ".join(f"`{label(p)}`" for p in pruned_pkgs) + ".")
        w("")

    return "\n".join(out).rstrip("\n") + "\n", third, groups, flags, fonts


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT, help="output file")
    ap.add_argument("--check", action="store_true",
                    help="do not write; exit 1 if the output file is stale")
    ap.add_argument("--metadata", type=Path,
                    help="read cargo metadata JSON from this file instead of running cargo")
    ap.add_argument("--no-tree", action="store_true",
                    help="skip the cargo tree feature check (keeps the workspace-wide set)")
    args = ap.parse_args()

    if args.metadata:
        meta = json.loads(args.metadata.read_text(encoding="utf-8"))
    else:
        meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked",
                               "--filter-platform", TARGET]))
    text, third, groups, flags, _fonts = render(meta, not args.no_tree)

    if args.check:
        current = args.out.read_bytes().decode("utf-8") if args.out.is_file() else ""
        if current != text:
            print(f"{args.out.name} is stale; run: python -I scripts/third_party_licenses.py",
                  file=sys.stderr)
            return 1
        print(f"{args.out.name} is up to date ({len(third)} packages).")
        return 0

    args.out.write_bytes(text.encode("utf-8"))
    print(f"wrote {args.out} ({len(third)} packages, {len(flags)} flagged)")
    for lic in sorted(groups, key=lambda l: (-len(groups[l]), l)):
        print(f"  {len(groups[lic]):4d}  {lic}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
