#!/usr/bin/env python3
"""Extract Diablo II fixture files from MPQ archives into a private corpus.

Usage:
    python -I scripts/extract_fixtures.py --profile vanilla|mxl --out <dir>
        [--ext ds1,dt1,txt,bin,dat,pl2,tbl,cof,d2]

Game archives are only ever opened with open(path, "rb"). Standard library only.
Format notes follow the public MPQ layout (StormLib conventions). The PKWARE DCL
decoder is a port of zlib's contrib/blast/blast.c (Mark Adler, zlib license).
"""

import argparse
import bz2
import fnmatch
import os
import re
import struct
import sys
import zlib
from collections import Counter
from contextlib import ExitStack
from pathlib import Path

VANILLA_DIR = Path(r"C:\Program Files (x86)\Diablo II")
MXL_DIR = Path(r"C:\games\median-xl")
DEFAULT_EXT = "ds1,dt1,txt,bin,dat,pl2,tbl,cof,d2"

M32 = 0xFFFFFFFF
MPQ_MAGIC = b"MPQ\x1a"
HASH_EMPTY = 0xFFFFFFFF
HASH_DELETED = 0xFFFFFFFE

FLAG_IMPLODE = 0x00000100
FLAG_COMPRESS = 0x00000200
FLAG_ENCRYPTED = 0x00010000
FLAG_FIX_KEY = 0x00020000
FLAG_SINGLE_UNIT = 0x01000000
FLAG_EXISTS = 0x80000000

# Compression mask bits stored in the first byte of each compressed sector.
MASK_HUFFMAN = 0x01
MASK_ZLIB = 0x02
MASK_PKWARE = 0x08
MASK_BZIP2 = 0x10
MASK_ADPCM_MONO = 0x40
MASK_ADPCM_STEREO = 0x80
MASK_SUPPORTED = MASK_ZLIB | MASK_PKWARE | MASK_BZIP2

EXCEL = "data\\global\\excel\\"
EXCEL_EXTRA_STEMS = ["lvltypes", "lvlprest", "lvlwarp", "objects", "objpreset", "levels", "automap"]


class ExtractError(Exception):
    """One file (or archive structure) could not be read or decoded."""


class UnsupportedError(ExtractError):
    """A compression method this script does not implement."""


# ---------------------------------------------------------------------------
# Crypto: MPQ crypt table, name hashing, block decryption
# ---------------------------------------------------------------------------

def _build_crypt_table():
    table = [0] * 0x500
    seed = 0x00100001
    for index1 in range(0x100):
        index = index1
        for _ in range(5):
            seed = (seed * 125 + 3) % 0x2AAAAB
            high = (seed & 0xFFFF) << 16
            seed = (seed * 125 + 3) % 0x2AAAAB
            table[index] = high | (seed & 0xFFFF)
            index += 0x100
    return table


CRYPT = _build_crypt_table()


def hash_string(name, hash_type):
    """MPQ name hash. hash_type: 0 table offset, 1 name A, 2 name B, 3 file key."""
    seed1 = 0x7FED7FED
    seed2 = 0xEEEEEEEE
    text = name.upper().replace("/", "\\")
    for ch in text.encode("latin-1", "replace"):
        seed1 = CRYPT[(hash_type << 8) + ch] ^ ((seed1 + seed2) & M32)
        seed2 = (ch + seed1 + seed2 + (seed2 << 5) + 3) & M32
    return seed1


def name_hashes(name):
    return hash_string(name, 0), hash_string(name, 1), hash_string(name, 2)


def decrypt(data, key):
    """Decrypt whole dwords of data. A trailing partial dword is left unchanged.

    Decryption is a sequential stream, so decrypting only the first N dwords
    gives the same first N results as decrypting a longer buffer.
    """
    count = len(data) // 4
    if count == 0:
        return bytes(data)
    words = struct.unpack_from("<%dI" % count, data)
    out = [0] * count
    seed = 0xEEEEEEEE
    k = key & M32
    for i, enc in enumerate(words):
        seed = (seed + CRYPT[0x400 + (k & 0xFF)]) & M32
        value = enc ^ ((k + seed) & M32)
        k = ((((~k) << 0x15) + 0x11111111) & M32) | (k >> 0x0B)
        seed = (value + seed + (seed << 5) + 3) & M32
        out[i] = value
    return struct.pack("<%dI" % count, *out) + bytes(data[count * 4:])


# ---------------------------------------------------------------------------
# PKWARE DCL "explode" decoder (port of zlib contrib/blast/blast.c)
# ---------------------------------------------------------------------------

_BLAST_MAXBITS = 13
_LITLEN = [
    11, 124, 8, 7, 28, 7, 188, 13, 76, 4, 10, 8, 12, 10, 12, 10, 8, 23, 8,
    9, 7, 6, 7, 8, 7, 6, 55, 8, 23, 24, 12, 11, 7, 9, 11, 12, 6, 7, 22, 5,
    7, 24, 6, 11, 9, 6, 7, 22, 7, 11, 38, 7, 9, 8, 25, 11, 8, 11, 9, 12,
    8, 12, 5, 38, 5, 38, 5, 11, 7, 5, 6, 21, 6, 10, 53, 8, 7, 24, 10, 27,
    44, 253, 253, 253, 252, 252, 252, 13, 12, 45, 12, 45, 12, 61, 12, 45,
    44, 173]
_LENLEN = [2, 35, 36, 53, 38, 23]
_DISTLEN = [2, 20, 53, 230, 247, 151, 248]
_BASE = [3, 2, 4, 5, 6, 7, 8, 9, 10, 12, 16, 24, 40, 72, 136, 264]
_EXTRA = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]


def _blast_construct(rep):
    """Build canonical Huffman tables from run-length code-length reps.

    Each rep byte is (repeat - 1) in the high nibble and a code length in the
    low nibble. Returns (count[0..13], symbols sorted by length).
    """
    length = []
    for b in rep:
        length.extend([b & 15] * ((b >> 4) + 1))
    n = len(length)
    count = [0] * (_BLAST_MAXBITS + 1)
    for ln in length:
        count[ln] += 1
    symbol = [0] * n
    if count[0] == n:
        return count, symbol
    offs = [0] * (_BLAST_MAXBITS + 2)
    for ln in range(1, _BLAST_MAXBITS):
        offs[ln + 1] = offs[ln] + count[ln]
    for sym, ln in enumerate(length):
        if ln != 0:
            symbol[offs[ln]] = sym
            offs[ln] += 1
    return count, symbol


_LIT_COUNT, _LIT_SYM = _blast_construct(_LITLEN)
_LEN_COUNT, _LEN_SYM = _blast_construct(_LENLEN)
_DIST_COUNT, _DIST_SYM = _blast_construct(_DISTLEN)


class _BlastBits:
    """LSB-first bit reader over an in-memory buffer."""

    __slots__ = ("data", "pos", "buf", "cnt")

    def __init__(self, data):
        self.data = data
        self.pos = 0
        self.buf = 0
        self.cnt = 0

    def bits(self, need):
        while self.cnt < need:
            if self.pos >= len(self.data):
                raise ExtractError("blast: input ended early")
            self.buf |= self.data[self.pos] << self.cnt
            self.pos += 1
            self.cnt += 8
        val = self.buf & ((1 << need) - 1)
        self.buf >>= need
        self.cnt -= need
        return val

    def decode(self, count, symbol):
        code = first = index = 0
        for ln in range(1, _BLAST_MAXBITS + 1):
            code |= self.bits(1) ^ 1
            c = count[ln]
            if code < first + c:
                return symbol[index + code - first]
            index += c
            first = (first + c) << 1
            code <<= 1
        raise ExtractError("blast: invalid huffman code")


def blast(data):
    """Decompress one PKWARE DCL ("implode") stream held entirely in memory."""
    br = _BlastBits(data)
    lit = br.bits(8)
    if lit > 1:
        raise ExtractError("blast: bad literal flag")
    dict_bits = br.bits(8)
    if dict_bits < 4 or dict_bits > 6:
        raise ExtractError("blast: bad dictionary size")
    out = bytearray()
    while True:
        if br.bits(1):
            sym = br.decode(_LEN_COUNT, _LEN_SYM)
            length = _BASE[sym] + br.bits(_EXTRA[sym])
            if length == 519:
                break
            sh = 2 if length == 2 else dict_bits
            dist = ((br.decode(_DIST_COUNT, _DIST_SYM) << sh) + br.bits(sh)) + 1
            if dist > len(out):
                raise ExtractError("blast: distance too far back")
            start = len(out) - dist
            if length <= dist:
                out += out[start:start + length]
            else:
                for i in range(length):  # overlapping copy
                    out.append(out[start + i])
        else:
            sym = br.decode(_LIT_COUNT, _LIT_SYM) if lit else br.bits(8)
            out.append(sym)
    return bytes(out)


def _self_check():
    if hash_string("(hash table)", 3) != 0xC3AF3770:
        raise SystemExit("self-check failed: hash_string('(hash table)', 3)")
    if hash_string("(block table)", 3) != 0xEC83B3A3:
        raise SystemExit("self-check failed: hash_string('(block table)', 3)")
    if sum(_LIT_COUNT) != 256 or sum(_LEN_COUNT) != 16 or sum(_DIST_COUNT) != 64:
        raise SystemExit("self-check failed: blast table sizes")
    if blast(bytes.fromhex("00048224258f807f")) != b"AIAIAIAIAIAIA":
        raise SystemExit("self-check failed: blast test vector")


# ---------------------------------------------------------------------------
# Archive reading
# ---------------------------------------------------------------------------

def decompress_block(data, flags):
    """Undo COMPRESS/IMPLODE on one sector or single-unit block.

    Returns (decoded bytes, OR of compression mask bits used).
    """
    if flags & FLAG_COMPRESS:
        if not data:
            raise ExtractError("empty compressed sector")
        mask = data[0]
        payload = data[1:]
        if mask & ~MASK_SUPPORTED:
            raise UnsupportedError("compression mask 0x%02x not supported" % mask)
        if mask & MASK_BZIP2:
            payload = bz2.decompress(payload)
        if mask & MASK_PKWARE:
            payload = blast(payload)
        if mask & MASK_ZLIB:
            payload = zlib.decompress(payload)
        return payload, mask
    if flags & FLAG_IMPLODE:
        return blast(data), MASK_PKWARE
    raise ExtractError("compressed data without a compression flag")


class Archive:
    def __init__(self, path, fh):
        self.path = path
        self.name = path.name
        self.fh = fh
        size = os.fstat(fh.fileno()).st_size
        self.base = None
        for off in range(0, size, 512):
            fh.seek(off)
            if fh.read(4) == MPQ_MAGIC:
                self.base = off
                break
        if self.base is None:
            raise ExtractError("%s: no MPQ header found" % self.name)

        fh.seek(self.base)
        head = fh.read(44)
        (_magic, header_size, _archive_size, fmt, shift,
         hash_off, block_off, hash_count, block_count) = struct.unpack_from("<4sIIHHIIII", head, 0)
        ext_off = 0
        if fmt >= 1 and header_size >= 44:
            ext_off, hash_hi, block_hi = struct.unpack_from("<QHH", head, 32)
            hash_off |= hash_hi << 32
            block_off |= block_hi << 32
        self.sector_size = 512 << shift

        self.hashes = self._read_table(hash_off, hash_count, "(hash table)", "<IIHHI")
        self.blocks = self._read_table(block_off, block_count, "(block table)", "<IIII")
        if ext_off and self.blocks:
            hi = struct.unpack("<%dH" % len(self.blocks),
                               self.read_at(ext_off, 2 * len(self.blocks)))
            self.blocks = [(o | (h << 32), c, f, fl)
                           for (o, c, f, fl), h in zip(self.blocks, hi)]

    def read_at(self, offset, size):
        self.fh.seek(self.base + offset)
        data = self.fh.read(size)
        if len(data) != size:
            raise ExtractError("%s: truncated read at 0x%x" % (self.name, offset))
        return data

    def _read_table(self, offset, count, key_name, fmt):
        if count == 0:
            return []
        raw = self.read_at(offset, count * 16)
        raw = decrypt(raw, hash_string(key_name, 3))
        return list(struct.iter_unpack(fmt, raw))

    def find(self, hashes):
        """Return the block index for a name given its (table, A, B) hashes, or None."""
        n = len(self.hashes)
        if n == 0:
            return None
        h0, name_a, name_b = hashes
        idx = h0 % n
        found = None
        for _ in range(n):
            ha, hb, locale, _platform, block = self.hashes[idx]
            if block == HASH_EMPTY:
                break
            if (block != HASH_DELETED and block < len(self.blocks)
                    and ha == name_a and hb == name_b):
                if locale == 0:
                    return block
                if found is None:
                    found = block
            idx = (idx + 1) % n
        return found

    def read_block(self, index, name):
        """Return (decoded bytes, OR of compression mask bits) for a block."""
        offset, csize, fsize, flags = self.blocks[index]
        if not flags & FLAG_EXISTS:
            raise ExtractError("block is marked as not existing")
        if fsize == 0:
            return b"", 0

        key = None
        if flags & FLAG_ENCRYPTED:
            key = hash_string(name.rsplit("\\", 1)[-1], 3)
            if flags & FLAG_FIX_KEY:
                key = ((key + offset) & M32) ^ fsize

        raw = self.read_at(offset, csize)
        compressed = bool(flags & (FLAG_COMPRESS | FLAG_IMPLODE))

        if flags & FLAG_SINGLE_UNIT:
            data = decrypt(raw, key) if key is not None else raw
            if compressed and csize < fsize:
                return decompress_block(data, flags)
            return data, 0

        sector = self.sector_size
        nsect = (fsize + sector - 1) // sector
        offsets = None
        if compressed:
            # Only the first nsect+1 table entries are used; the optional CRC
            # table, if any, sits after the data and is not needed.
            tab_len = (nsect + 1) * 4
            if tab_len > csize:
                raise ExtractError("sector offset table larger than block")
            table = raw[:tab_len]
            if key is not None:
                table = decrypt(table, (key - 1) & M32)
            offsets = struct.unpack("<%dI" % (nsect + 1), table)

        parts = []
        masks = 0
        for i in range(nsect):
            expect = min(sector, fsize - i * sector)
            if offsets is not None:
                start, end = offsets[i], offsets[i + 1]
            else:
                start = i * sector
                end = min(start + sector, csize)
            if start > end or end > csize:
                raise ExtractError("sector %d lies outside its block" % i)
            chunk = raw[start:end]
            if key is not None:
                chunk = decrypt(chunk, (key + i) & M32)
            if len(chunk) != expect:  # stored size differs from size: compressed
                if not compressed:
                    raise ExtractError("sector %d has unexpected size" % i)
                chunk, used = decompress_block(chunk, flags)
                masks |= used
            parts.append(chunk)
        data = b"".join(parts)
        if len(data) != fsize:
            raise ExtractError("decoded %d bytes, expected %d" % (len(data), fsize))
        return data, masks


# ---------------------------------------------------------------------------
# Profiles, names, and extraction
# ---------------------------------------------------------------------------

def norm(name):
    return name.strip().replace("/", "\\").lower().lstrip("\\")


def ext_of(name):
    base = name.rsplit("\\", 1)[-1]
    return base.rsplit(".", 1)[1] if "." in base else ""


def safe_parts(name):
    parts = name.split("\\")
    for part in parts:
        if part in ("", ".", "..") or ":" in part or "/" in part:
            raise ExtractError("unsafe path %r" % name)
    return parts


def read_listfile(arch):
    bi = arch.find(name_hashes("(listfile)"))
    if bi is None:
        return []
    data, _ = arch.read_block(bi, "(listfile)")
    text = data.decode("latin-1")
    return [norm(part) for part in re.split(r"[;\r\n]", text) if part.strip()]


def extra_names():
    names = []
    for stem in EXCEL_EXTRA_STEMS:
        names.append(EXCEL + stem + ".bin")
        names.append(EXCEL + stem + ".txt")
    for act in range(1, 6):
        names.append("data\\global\\palette\\act%d\\pal.dat" % act)
        names.append("data\\global\\palette\\act%d\\pal.pl2" % act)
    names.append("data\\global\\animdata.d2")
    return names


def excel_pairs(names):
    pairs = set()
    for n in names:
        if n.startswith(EXCEL):
            if n.endswith(".txt"):
                pairs.add(n[:-4] + ".bin")
            elif n.endswith(".bin"):
                pairs.add(n[:-4] + ".txt")
    return pairs


def find_file(folder, name):
    for p in folder.iterdir():
        if p.is_file() and p.name.lower() == name.lower():
            return p
    return None


def resolve_archives(profile):
    """Archive paths in priority order (first match wins)."""
    if profile == "vanilla":
        folder = VANILLA_DIR
        head = []
        base_names = ["Patch_D2.mpq", "d2exp.mpq", "d2data.mpq", "d2char.mpq"]
    else:
        folder = MXL_DIR
        head = sorted((p for p in folder.iterdir()
                       if p.is_file() and fnmatch.fnmatchcase(p.name.lower(), "medianxl-*.mpq")),
                      key=lambda p: p.name.lower())
        if not head:
            raise SystemExit("error: no medianxl-*.mpq in %s" % folder)
        base_names = ["patch_d2.mpq", "d2exp.mpq", "d2data.mpq", "d2char.mpq"]
    paths = list(head)
    for name in base_names:
        p = find_file(folder, name)
        if p is None:
            raise SystemExit("error: %s not found in %s" % (name, folder))
        paths.append(p)
    return paths


def is_inside(child, parent):
    try:
        return os.path.commonpath([child, parent]) == parent
    except ValueError:
        return False


def run(profile, out_root, exts):
    paths = resolve_archives(profile)
    print("profile: %s" % profile)
    print("archive order:")
    for p in paths:
        print("  %s" % p)

    failures = []
    written = []  # (name, archive, size, flags, masks)
    with ExitStack() as stack:
        archives = []
        for p in paths:
            fh = stack.enter_context(open(p, "rb"))
            try:
                archives.append(Archive(p, fh))
            except (ExtractError, struct.error, OSError) as exc:
                raise SystemExit("error: cannot open %s: %s" % (p.name, exc))

        listed = set()
        for arch in archives:
            try:
                listed.update(read_listfile(arch))
            except Exception as exc:  # report and keep going with other archives
                failures.append(("(listfile)", arch.name, "%s: %s" % (type(exc).__name__, exc)))

        extras = set(extra_names())
        names = listed | extras | excel_pairs(listed | extras)

        not_found_extra = []
        missing_listed = 0
        for name in sorted(names):
            if ext_of(name) not in exts:
                continue
            hashes = name_hashes(name)
            hit = None
            for arch in archives:
                bi = arch.find(hashes)
                if bi is not None:
                    hit = (arch, bi)
                    break
            if hit is None:
                if name in extras:
                    not_found_extra.append(name)
                else:
                    missing_listed += 1
                continue
            arch, bi = hit
            try:
                parts = safe_parts(name)
                data, masks = arch.read_block(bi, name)
                size_expected = arch.blocks[bi][2]
                if len(data) != size_expected:
                    failures.append((name, arch.name,
                                     "size mismatch: %d != %d" % (len(data), size_expected)))
                    continue
                dest = out_root.joinpath(*parts)
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(data)
            except Exception as exc:  # per-file failure, keep going
                failures.append((name, arch.name, "%s: %s" % (type(exc).__name__, exc)))
                continue
            written.append((name, arch.name, len(data), arch.blocks[bi][3], masks))

    manifest = out_root / "_manifest.tsv"
    with open(manifest, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("path\tarchive\tsize\tflags_hex\tcompression_hex\n")
        for name, arch_name, size, flags, masks in written:
            fh.write("%s\t%s\t%d\t0x%08X\t0x%02X\n" % (name, arch_name, size, flags, masks))

    by_ext = Counter(ext_of(w[0]) for w in written)
    by_archive = Counter(w[1] for w in written)
    print("files written: %d" % len(written))
    print("by extension:")
    for ext, count in sorted(by_ext.items()):
        print("  %s: %d" % (ext, count))
    print("by source archive:")
    for arch_name, count in sorted(by_archive.items(), key=lambda kv: kv[0].lower()):
        print("  %s: %d" % (arch_name, count))
    print("listed names not present in any archive (wanted extensions only): %d" % missing_listed)
    print("explicit names not found: %d" % len(not_found_extra))
    for name in sorted(not_found_extra):
        print("  missing: %s" % name)
    print("failures: %d" % len(failures))
    for name, arch_name, reason in failures:
        print("  %s [%s]: %s" % (name, arch_name, reason))
    print("manifest: %s" % manifest)
    return 0 if not failures else 1


def main(argv=None):
    sys.stdout.reconfigure(errors="backslashreplace")
    ap = argparse.ArgumentParser(description="Extract Diablo II fixtures from MPQ archives (read-only).")
    ap.add_argument("--profile", required=True, choices=("vanilla", "mxl"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--ext", default=DEFAULT_EXT,
                    help="comma-separated extensions to write (default: %(default)s)")
    args = ap.parse_args(argv)

    _self_check()

    out_root = Path(os.path.abspath(args.out))
    real_out = os.path.normcase(os.path.realpath(out_root))
    for game in (VANILLA_DIR, MXL_DIR):
        if is_inside(real_out, os.path.normcase(os.path.realpath(game))):
            sys.exit("error: --out %s is inside game directory %s; refusing" % (out_root, game))

    exts = {e.strip().lower().lstrip(".") for e in args.ext.split(",") if e.strip()}
    out_root.mkdir(parents=True, exist_ok=True)
    return run(args.profile, out_root, exts)


if __name__ == "__main__":
    sys.exit(main())
