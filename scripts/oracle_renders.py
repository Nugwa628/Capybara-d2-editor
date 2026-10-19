#!/usr/bin/env python3
"""Render oracle reference PNGs of vanilla DS1 maps with the reference editor.

Usage:
    python -I scripts/oracle_renders.py [--count N] [--maps act1_town,act2_town,...]

Each selected entry is one DS1 file listed in an oracle ``assets\\*.ini``.
Maps are named ``<ini_stem>__<ds1_stem>``. Without ``--maps`` the default set
is the first DS1 line of each ini in DEFAULT_MAPS. ``--maps`` also accepts
``<ini_stem>/<ds1_stem>`` to pick a specific DS1 inside an ini.

For every map the oracle is run twice:

    ds1edit.exe <file.ds1> <LvlTypesId> <LvlPrestDef> --zoom=1:4 --headless <out.png>

with the oracle folder as working directory. The first render is kept as
``<name>.png``; the second render is compared against it (bytes first, then
decoded pixels) to record determinism. Results go to ``manifest.json``.

Terrain-only renders are not produced: the reference editor has no CLI flag or
ini key that hides objects, NPC paths, walkable info, grid or specials. Object
shadows and objects are gated by the per-map ``animations_layer_mask``
(ds1.c:446 default 1, preview.c:3629-3695), which only the F3 key changes at
runtime (interface.c:1066-1071). So ``terrain_png`` is always null.

Standard library only.
"""

import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

# ---------------------------------------------------------------------------
# Fixed locations (absolute, per the oracle setup)
# ---------------------------------------------------------------------------
ORACLE_DIR = Path(r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor\testdata\private\oracle")
EXE = ORACLE_DIR / "ds1edit.exe"
ASSETS_DIR = ORACLE_DIR / "assets"
CORPUS_TILES = Path(
    r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor\testdata\private\extracted"
    r"\vanilla\data\global\tiles"
)
OUT_DIR = Path(
    r"C:\Users\ZF\Desktop\sugma\capybara-mapeditor\testdata\private\oracle-renders"
)

ZOOM = "1:4"
RENDER_TIMEOUT_S = 120
STDOUT_TAIL_LINES = 20
STDERR_TAIL_LINES = 20
SAVED_MARKER = "headless mode : saved"

# Varied default set: towns of acts 1-5, outdoor, cave, crypt, desert, jungle,
# Act 4 maps, the Act 5 siege, and the LvlTypes 35 palette case (act5_lava).
DEFAULT_MAPS = [
    "act1_town",
    "act1_wilderness",
    "act1_cave",
    "act1_crypt",
    "act1_catacombs",
    "act1_barracks",
    "act2_town",
    "act2_desert",
    "act2_tomb",
    "act3_town",
    "act3_jungle",
    "act3_kurast",
    "act4_fortress",
    "act4_mesa",
    "act5_town",
    "act5_siege",
    "act5_lava",
    "act5_ice_caves",
]

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


class UnsupportedPng(Exception):
    pass


# ---------------------------------------------------------------------------
# INI and corpus mapping
# ---------------------------------------------------------------------------
def parse_ini(ini_path):
    """Return list of entries: dicts with lvltypes_id, lvlprest_def, rel (lowercase, after assets/tiles/)."""
    entries = []
    with open(ini_path, "r", encoding="utf-8", errors="replace") as fh:
        for raw in fh:
            line = raw.strip()
            if not line or line.startswith((";", "#")):
                continue
            parts = line.split(maxsplit=2)
            if len(parts) != 3:
                raise ValueError(f"{ini_path.name}: bad line {line!r}")
            lvltypes_id, lvlprest_def, virtual = parts
            norm = virtual.replace("\\", "/").lower()
            marker = "assets/tiles/"
            idx = norm.find(marker)
            if idx < 0:
                raise ValueError(f"{ini_path.name}: no assets/tiles/ in {virtual!r}")
            rel = norm[idx + len(marker):]
            entries.append({
                "lvltypes_id": int(lvltypes_id),
                "lvlprest_def": int(lvlprest_def),
                "rel": rel,
            })
    return entries


def load_catalog():
    """Return dict ini_stem -> list of entries (each with ds1 info), in file order."""
    catalog = {}
    for ini_path in sorted(ASSETS_DIR.glob("*.ini")):
        stem = ini_path.stem
        entries = parse_ini(ini_path)
        for e in entries:
            ds1_stem = Path(e["rel"]).stem
            e["ds1_stem"] = ds1_stem
            e["ini_stem"] = stem
            e["ini_rel"] = f"assets\\{ini_path.name}"
            e["corpus_path"] = CORPUS_TILES.joinpath(*e["rel"].split("/"))
            e["game_path"] = "data\\global\\tiles\\" + e["rel"].replace("/", "\\")
        catalog[stem] = entries
    return catalog


def select_maps(catalog, maps_arg, count):
    """Return list of (name, entry) to render."""
    selected = []
    if maps_arg:
        tokens = [t.strip() for t in maps_arg.split(",") if t.strip()]
    else:
        tokens = list(DEFAULT_MAPS)
    for token in tokens:
        if "/" in token:
            stem, ds1 = token.split("/", 1)
            ds1 = Path(ds1).stem.lower()
            if stem not in catalog:
                raise SystemExit(f"unknown ini stem {stem!r}")
            matches = [e for e in catalog[stem] if e["ds1_stem"] == ds1]
            if not matches:
                raise SystemExit(f"no ds1 {ds1!r} in {stem}.ini")
            entry = matches[0]
        else:
            if token not in catalog:
                raise SystemExit(f"unknown ini stem {token!r}")
            if not catalog[token]:
                raise SystemExit(f"{token}.ini has no entries")
            entry = catalog[token][0]
        name = f"{entry['ini_stem']}__{entry['ds1_stem']}"
        selected.append((name, entry))
    if count is not None:
        selected = selected[:count]
    return selected


# ---------------------------------------------------------------------------
# PNG helpers (stdlib only)
# ---------------------------------------------------------------------------
def png_size(data):
    if data[:8] != PNG_SIGNATURE or data[12:16] != b"IHDR":
        raise ValueError("not a PNG with IHDR first")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def _png_chunks(data):
    pos = 8
    while pos + 8 <= len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        ctype = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        yield ctype, body
        pos += 12 + length
        if ctype == b"IEND":
            break


def _paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def png_pixels(data):
    """Decode an 8-bit, non-interlaced PNG to (width, height, color_type, raw_rows)."""
    if data[:8] != PNG_SIGNATURE:
        raise UnsupportedPng("bad signature")
    idat = []
    header = None
    for ctype, body in _png_chunks(data):
        if ctype == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif ctype == b"IDAT":
            idat.append(body)
    if header is None:
        raise UnsupportedPng("no IHDR")
    width, height, bit_depth, color_type, _, _, interlace = header
    channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(color_type)
    if bit_depth != 8 or channels is None or interlace != 0:
        raise UnsupportedPng(f"unsupported bit_depth={bit_depth} color_type={color_type} interlace={interlace}")
    bpp = channels
    stride = width * bpp
    raw = zlib.decompress(b"".join(idat))
    out = bytearray()
    prev = bytearray(stride)
    pos = 0
    for _ in range(height):
        ftype = raw[pos]
        line = raw[pos + 1:pos + 1 + stride]
        pos += 1 + stride
        recon = bytearray(stride)
        for i in range(stride):
            x = line[i]
            a = recon[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if ftype == 0:
                v = x
            elif ftype == 1:
                v = x + a
            elif ftype == 2:
                v = x + b
            elif ftype == 3:
                v = x + ((a + b) >> 1)
            elif ftype == 4:
                v = x + _paeth(a, b, c)
            else:
                raise UnsupportedPng(f"bad filter type {ftype}")
            recon[i] = v & 0xFF
        out += recon
        prev = recon
    return width, height, color_type, bytes(out)


def compare_renders(path_a, path_b):
    """Return (deterministic: bool, method: str)."""
    data_a = Path(path_a).read_bytes()
    data_b = Path(path_b).read_bytes()
    if data_a == data_b:
        return True, "bytes-identical"
    try:
        pa = png_pixels(data_a)
        pb = png_pixels(data_b)
    except (UnsupportedPng, zlib.error, struct.error, IndexError) as exc:
        return False, f"bytes-differ (pixel decode unsupported: {exc})"
    if pa[0] == pb[0] and pa[1] == pb[1] and pa[2] == pb[2] and pa[3] == pb[3]:
        return True, "pixels-identical (metadata differs)"
    return False, "pixels-differ"


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------
def _tail(blob, n):
    if blob is None:
        return ""
    if isinstance(blob, bytes):
        blob = blob.decode("utf-8", errors="replace")
    lines = blob.splitlines()
    return "\n".join(lines[-n:])


def run_render(ds1_path, entry, out_png):
    """Run the oracle once. Returns dict with ok, stdout, stderr, error."""
    if out_png.exists():
        out_png.unlink()
    cmd = [
        str(EXE),
        str(ds1_path),
        str(entry["lvltypes_id"]),
        str(entry["lvlprest_def"]),
        f"--zoom={ZOOM}",
        "--headless",
        str(out_png),
    ]
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(ORACLE_DIR),
            capture_output=True,
            timeout=RENDER_TIMEOUT_S,
        )
        stdout, stderr, rc = proc.stdout, proc.stderr, proc.returncode
        timed_out = False
    except subprocess.TimeoutExpired as exc:
        stdout, stderr, rc = exc.stdout, exc.stderr, None
        timed_out = True

    stdout_text = stdout.decode("utf-8", errors="replace") if isinstance(stdout, bytes) else (stdout or "")
    error = None
    if timed_out:
        error = f"timeout after {RENDER_TIMEOUT_S}s"
    elif rc != 0:
        error = f"exit code {rc}"
    elif SAVED_MARKER not in stdout_text:
        # The exe exits 0 even when headless rendering fails ("render failed").
        error = "no 'saved' line in stdout"
    elif not out_png.exists() or out_png.stat().st_size == 0:
        error = "output PNG missing or empty"
    else:
        try:
            png_size(out_png.read_bytes())
        except (ValueError, struct.error) as exc:
            error = f"output is not a valid PNG: {exc}"

    return {
        "ok": error is None,
        "stdout": stdout,
        "stderr": stderr,
        "error": error,
    }


def render_map(name, entry):
    ds1_path = entry["corpus_path"]
    png = OUT_DIR / f"{name}.png"
    record = {
        "name": name,
        "ini": entry["ini_rel"],
        "lvltypes_id": entry["lvltypes_id"],
        "lvlprest_def": entry["lvlprest_def"],
        "ds1": entry["game_path"],
        "png": str(png),
        "terrain_png": None,
        "width": None,
        "height": None,
        "deterministic": False,
        "determinism_check": None,
        "stdout_tail": "",
        "stderr_tail": "",
        "error": None,
        "ok": False,
    }

    if not ds1_path.is_file():
        record["error"] = f"corpus file missing: {ds1_path}"
        return record

    first = run_render(ds1_path, entry, png)
    record["stdout_tail"] = _tail(first["stdout"], STDOUT_TAIL_LINES)
    record["stderr_tail"] = _tail(first["stderr"], STDERR_TAIL_LINES)
    if not first["ok"]:
        record["error"] = f"render 1: {first['error']}"
        return record

    width, height = png_size(png.read_bytes())
    record["width"], record["height"] = width, height

    rerun_dir = Path(tempfile.mkdtemp(prefix="oracle_rerun_"))
    try:
        second_png = rerun_dir / f"{name}.png"
        second = run_render(ds1_path, entry, second_png)
        if not second["ok"]:
            record["error"] = f"render 2: {second['error']}"
            record["stderr_tail"] = _tail(second["stderr"], STDERR_TAIL_LINES) or record["stderr_tail"]
            return record
        deterministic, method = compare_renders(png, second_png)
    finally:
        shutil.rmtree(rerun_dir, ignore_errors=True)

    record["deterministic"] = deterministic
    record["determinism_check"] = method
    record["ok"] = True
    return record


def print_summary(records):
    header = f"{'map':<34} {'size':>11}  {'deterministic':<13} {'ok':<5} note"
    print(header)
    print("-" * len(header))
    for r in records:
        size = f"{r['width']}x{r['height']}" if r["width"] else "-"
        note = r["error"] or ""
        print(f"{r['name']:<34} {size:>11}  {str(r['deterministic']):<13} {str(r['ok']):<5} {note}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--count", type=int, default=None,
                        help="render only the first N selected maps")
    parser.add_argument("--maps", default=None,
                        help="comma list of ini stems (or stem/ds1 to pick one DS1)")
    args = parser.parse_args(argv)

    if not EXE.is_file():
        print(f"oracle exe not found: {EXE}", file=sys.stderr)
        return 2

    catalog = load_catalog()
    selected = select_maps(catalog, args.maps, args.count)
    if not selected:
        print("no maps selected", file=sys.stderr)
        return 2

    OUT_DIR.mkdir(parents=True, exist_ok=True)

    records = []
    for index, (name, entry) in enumerate(selected, 1):
        print(f"[{index}/{len(selected)}] {name}", flush=True)
        records.append(render_map(name, entry))

    manifest_path = OUT_DIR / "manifest.json"
    manifest_path.write_text(json.dumps(records, indent=2) + "\n", encoding="utf-8")

    print()
    print_summary(records)
    print()
    print(f"manifest: {manifest_path}")
    return 0 if all(r["ok"] for r in records) else 1


if __name__ == "__main__":
    sys.exit(main())
