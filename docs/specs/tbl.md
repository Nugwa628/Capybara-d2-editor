# D2 `.tbl` string tables: layout, hash, lookup: cross-check

Status: cross-check of `reference/d2-ds1-edit/docs/guides/tbl-string-tables.md` against the reference
tree. **The reference tree has no `.tbl` reader.** Every layout, hash and lookup claim in this file is
therefore **NOT IN SOURCE**. What can be checked is internal consistency and one hash example, which
was recomputed independently. Read-only; no code in this repo changes.

## Sources and conventions

- Guide: `reference/d2-ds1-edit/docs/guides/tbl-string-tables.md` (cited as `guide#anchor`). Its claims
  come from `D2Lang.dll` 1.13c disassembly. The companion `tbl-string-tables.verification.md` was not
  read in full.
- Search of the reference tree for `.tbl`, string-table and hash code:
  - `src/` matches: only MPQ hash and block tables (`src/mpq/mpqtypes.h:43-47`,
    `src/mpq/MpqView.c:306-360`). These are MPQ structures, not D2 string tables.
  - `tools/d2mpq.py:341-348`: Huffman count table for MPQ decompression. Unrelated.
  - `src/core/txtread.c` reads `.txt` tables (`txt_gets`, `txt_load`) and does not read `.tbl`.
  - No `*.tbl` file anywhere in the reference tree.
- Verdicts: **AGREES**, **DIFFERS**, **NOT IN SOURCE**, **NOT IN GUIDE**.
- Editor impact: display text the editor shows comes from plain `.txt` columns (for example the
  `Description` column of `assets/editor/obj.txt`, `txtread.c:1138`). The editor does not resolve keys
  through `string.tbl`. An implementer who needs localized text must add a `.tbl` reader from the guide.

---

## 1. File layout (guide, not in source)

Four regions back to back (`guide#on-disk-layout`):

| Region | Start | Size | Content |
|---|---|---|---|
| Header | `0x00` | 21 bytes (`0x15`) | counts, offsets, CRC |
| ID table | `0x15` | `H × 2` bytes | `H` little-endian words; entry `i` = node index of numeric ID `i` |
| Node array | `0x15 + 2H` | `N × 17` bytes | open-addressing hash table |
| String data | `X` | to end `S` | NUL-terminated key and display strings, referenced by absolute offset |

### 1.1 Header (`guide#the-header`)

| Offset | Size | Type | Field | Status | Guide ref |
|---|---|---|---|---|---|
| 0x00 | 2 | u16 | CRC of `[X, S)` | verified in guide | `#the-header` |
| 0x02 | 2 | u16 | H, ID-table entries | verified in guide | `#the-header` |
| 0x04 | 4 | u32 | N, node slots and hash modulus | verified in guide | `#the-header` |
| 0x08 | 1 | u8 | unknown, observed `1` | **unverified** | `#the-header` |
| 0x09 | 4 | u32 | X, string-data start | verified in guide | `#the-header` |
| 0x0D | 4 | u32 | P, probe-count ceiling | verified in guide (equals longest chain) | `#what-p-is-actually-for` |
| 0x11 | 4 | u32 | S, total file size | verified in guide | `#the-header` |

Field sizes sum to 21 bytes (2+2+4+1+4+4+4), which matches `0x15`. **Internal check: AGREES.**

### 1.2 ID table

`H` words at `0x15`. Entry `i` gives the node index of numeric string ID `i` (`guide#the-id-table`).

### 1.3 Node (17 bytes)

| Offset in node | Size | Type | Field | Status |
|---|---|---|---|---|
| +0x00 | 1 | u8 | Active (`1` = occupied) | verified in guide |
| +0x01 | 2 | u16 | StringID (local numeric ID) | verified in guide |
| +0x03 | 4 | u32 | unknown | **unverified** (`#the-one-field-this-chapter-couldnt-verify`) |
| +0x07 | 4 | u32 | KeyOffset | verified in guide |
| +0x0B | 4 | u32 | TextOffset | verified in guide |
| +0x0F | 2 | u16 | TextLength, bytes incl. NUL | verified in guide |

Field sizes sum to 17 bytes (1+2+4+4+4+2). **Internal check: AGREES.** Stride `16x + x` (`guide#the-node-array`) is
a compiler detail. Any stride-17 implementation is equivalent.

### 1.4 String data

Raw NUL-terminated bytes. Key and display strings share the region (`guide#the-string-data-blob`).
**Character encoding is not stated in the guide** (NOT IN GUIDE). See §6.

---

## 2. Hash and key lookup (guide, not in source)

### 2.1 Hash (`guide#the-hash`)

ELF/PJW variant. Pseudo-code from the guide:

```c
uint hash(char *key, uint modulus) {
    uint h = 0;
    for (; *key; key++) {
        h = h * 16 + (unsigned char)*key;
        if (h & 0xF0000000)
            h = ((h & 0xF0000000) >> 24 ^ h) & 0x0FFFFFFF;
    }
    return h % modulus;
}
```

- `modulus` is always **N**, never H (`guide#the-hash`).
- **Independent check:** I re-implemented this in Python. `hash("qstsa5q1") = 0x0AB94BC1`, and
  `0x0AB94BC1 % 2819 = 1495`. Both match the guide's worked example (`guide#worked-example-qstsa5q1`).
  This confirms the algorithm as written, not the file.
- Key bytes are hashed as unsigned. Encoding of non-ASCII keys is not stated (§6).

### 2.2 Lookup (`guide#the-node-array`, `guide#worked-example-qstsa5q1`)

```
bucket = hash(key, N)
for attempt in 0 .. P-1:
    slot = (bucket + attempt) mod N
    if node[slot].Active == 1 and key(node[slot]) == key: return node[slot]
return miss
```

Important details from the guide:
- An **empty slot does not stop the probe**. The guide says the walk continues to the next slot on
  `Active == 0`, and only `P` ends it.
- `P` is the longest probe chain measured by the table compiler (`guide#what-p-is-actually-for`). Stopping
  at `P` is a bound, not a guarantee of absence.
- Vanilla `expansionstring.tbl` has 2,818 occupied slots out of 2,819 (`guide#worked-example-qstsa5q1`).
  Long chains are normal there.

### 2.3 Worked example (guide)

`qstsa5q1` → bucket 1495 (occupied by `Hwanin's Refuge`) → probe to slot 444, 1,769 probes later. Entry
2618 of the ID table points to 444 (`guide#worked-example-qstsa5q1`). Display text `Siege on Harrogath`.
NOT IN SOURCE (data not in tree). The probe count and ID round trip cannot be checked here.

---

## 3. Numeric IDs (guide)

`GetStringByID` (`guide#numeric-ids-a-second-front-door`):

| Numeric ID | Table | Local ID |
|---|---|---|
| 0 – 9999 | `string.tbl` | as is |
| 10000 – 19999 | `patchstring.tbl` | id − 10000 |
| 20000 and up | `expansionstring.tbl` | id − 20000 |

- Implemented with 16-bit wrap (`+0xD8F0`, `+0xB1E0`), equivalent to the subtraction once the result wraps
  (guide).
- Used at startup for `K`, `M`, `B`, `T` abbreviations, IDs 5328–5331 in `string.tbl` (guide).

NOT IN SOURCE.

---

## 4. Precedence and miss

### 4.1 Key precedence (`guide#precedence-when-a-key-lives-in-more-than-one-table`)

1. `patchstring.tbl`; hit → result + 10000 (global ID space).
2. `expansionstring.tbl`; hit → result + 20000.
3. `string.tbl`; hit → result.
4. Miss → fallback (§4.2).

87 keys appear in both `string.tbl` and `patchstring.tbl`; for 56 the text differs (guide). Examples:
`9bl` → `Stiletto` (patch wins over `Stilleto`), `AmaOnly` → `(Amazon Only)` (patch wins over eleven spaces).
NOT IN SOURCE.

### 4.2 Miss (`guide#on-a-miss`)

- Falls through to `FUN_6fc09fb0`. Keeps a linked list of missed keys (static root `DAT_6fc10a60`), so each
  miss is built once.
- Builds a fallback text that includes the key and the debug marker at `0x6fc10568`: `" -not xlated call ken w"`.
  The exact final concatenation is **not re-derived** by the guide (its own admission in `#on-a-miss`).
- No crash on a miss. Guide contrasts this with the `Objects.txt` out-of-range abort in the excel chapter.

NOT IN SOURCE.

---

## 5. Cross-check of guide claims

| # | Guide claim (anchor) | Repo source | Verdict | Note |
|---|---|---|---|---|
| T1 | Three tables: `string.tbl`, `patchstring.tbl`, `expansionstring.tbl` under `data\local\lng\eng\` (`#three-files-one-job`) | none | NOT IN SOURCE | |
| T2 | Header 21 bytes, fields as in §1.1 (`#the-header`) | none | NOT IN SOURCE | Internal sum checked: AGREES (§1.1). |
| T3 | Node 17 bytes, fields as in §1.3 (`#the-node-array`) | none | NOT IN SOURCE | Internal sum checked: AGREES (§1.3). |
| T4 | ELF/PJW hash, modulus N (`#the-hash`) | none | NOT IN SOURCE | Recomputed for `qstsa5q1`: AGREES with guide's example. |
| T5 | Linear probe capped at P, empty slot continues (`#the-node-array`) | none | NOT IN SOURCE | |
| T6 | P equals longest chain: 2,453 / 5,192 / 1,058 (`#what-p-is-actually-for`) | none | NOT IN SOURCE | Not recomputable without data. |
| T7 | CRC-16 poly `0x1021`, init `0xFFFF` over `[X,S)` (`#worked-example-qstsa5q1`, `#the-header`) | none | NOT IN SOURCE | Guide says standard CCITT table. Repo has no CRC for this format. |
| T8 | Numeric ID ranges 0 / 10000 / 20000 (`#numeric-ids-a-second-front-door`) | none | NOT IN SOURCE | |
| T9 | Precedence patch → expansion → base, offsets +10000 / +20000 (`#precedence-...`) | none | NOT IN SOURCE | |
| T10 | Miss gives fallback text with debug marker, no crash (`#on-a-miss`) | none | NOT IN SOURCE | Final string not re-derived even by the guide. |
| T11 | Table stats: string 5,391 active / N 5,393 / P 5,192; patch 1,169 / 1,171 / 1,058; expansion 2,818 / 2,819 / 2,453 (`#table-statistics-vanilla-1-13c`) | none | NOT IN SOURCE | Internal check: `N - H` is 2, 2 and 1, and `active = H` in every row. Consistent but not checkable. |
| T12 | 277 entries share key `x` in `string.tbl`, used for numeric-only text (`#numeric-ids-...`) | none | NOT IN SOURCE | |
| T13 | 1.09d `string.tbl` and `expansionstring.tbl` byte-identical to 1.13c (`#version-differences`) | none | NOT IN SOURCE | |
| T14 | `Levels.txt` keys such as `Rigid Highlands` resolve to `Frigid Highlands` (`#why-the-indirection-matters...`) | none | NOT IN SOURCE | Repo's `Levels.txt` handling not in scope here. |
| T15 | Modders override via MPQ order and `patchstring.tbl` (`#what-a-modder-changes`) | none | NOT IN SOURCE | |

No row is **DIFFERS** because the repo has no `.tbl` implementation to differ from.

---

## 6. Open points

1. **Character encoding of key and display strings is not stated.** The guide says only "raw NUL-terminated bytes".
   Decide the Rust type (`Vec<u8>` plus encoding, or decoded text) once real `string.tbl` bytes are sampled.
   The guide's `" -not xlated call ken w"` marker is ASCII. Non-ASCII text (Korean, accented Latin) is
   not discussed at all.
2. **Empty-slot probing.** The guide says the probe continues past empty slots and stops only at `P`.
   That is unusual for open addressing. Confirm on real data before relying on early exit. Stopping at the
   first empty slot would be a bug if the guide is right and the compiler packed differently.
3. **Duplicate keys.** First reachable entry wins. Shadowed entries are reachable only by numeric ID.
   Decide whether the port should index by numeric ID instead.
4. **CRC validation.** The guide describes CRC-16 over `[X, S)`. Repo has none, so a Rust reader must
   either implement it or skip it. Skipping loses corruption detection, not correctness.
5. **Unverified fields.** Header byte `0x08` and node `+0x03` dword are unverified in the guide and unused by
   its reader.
6. **Miss text.** Decide whether a miss returns the key, empty text, or the guide's fallback. Matching the
   game means the fallback, which contains a developer note; the editor may prefer the bare key.
7. **Data availability.** No `.tbl` sample exists in the reference tree. Every table statistic and
   worked-example bucket needs a real `string.tbl` to check. The guide's stats were built from
   `tools/d2mpq.py` extractions, which are not in this tree.
8. **Companion ledger not reconciled.** `tbl-string-tables.verification.md` may hold corrections to the
   guide. Read it before implementing, especially for the P and CRC claims.

---

## 7. Writer (`d2_formats::tbl::TblDoc`)

Checked against the 40 string tables of the vanilla and Median XL corpora (the other 32 `.tbl` files
under `data/local/font` are glyph tables, a different format, and are not handled here).

- **Layout reproduced exactly.** Re-laying out any of the 40 tables from its `(key, text)` list alone
  (`TblDoc::layout`) gives back the original bytes. So the compiler's rules are:
  - `N` (header 0x04) is kept; it grows only when there are more strings than slots.
  - Strings go in id order to `key_hash(key, N)` or the next free slot (linear, wrapping). The lowest id of
    a duplicate key therefore owns the home slot.
  - ID table entry `i` = node slot of string `i`; node id field = `i`.
  - `P` (0x0D) = longest probe sequence **plus one** (a lone string in its home slot has `P = 1`).
  - Node `+3` (the "unknown" dword) = the key's hash value (`key_hash(key, N)`, before probing).
  - Node text length = text bytes + 1. String data: `key NUL text NUL` per id, in id order.
  - CRC (0x00) = CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection) over `[X, S)`;
    `S` (0x11) = file size.
  This settles open points 4 (CRC) and 5 (node `+3`) for the writer.
- **Unedited tables write back the loaded bytes**, so a layout the compiler did not produce still
  round-trips; after an edit the table is re-laid out as above.
- **Ids.** Strings are addressed by id = position. `set` on an existing key keeps its id; a new key is
  appended (`add` appends even a duplicate). `remove` shifts later ids down (no gaps), which invalidates
  compiled excel tables that store string ids. `TblDoc::parse` refuses tables whose active ids are not
  exactly `0..n` (none in the corpus).
- **Encoding.** Latin-1, one char per byte (as the reader). Chars above U+00FF, NUL, empty keys and
  texts of 65535+ bytes are rejected.
- **Lookup.** The test suite emulates the game (hash, then up to `P` slots, empty slots do not end the
  walk) and finds every key after edits, including heavily colliding and fully packed tables.
