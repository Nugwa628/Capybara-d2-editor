# AnimData.d2: layout, hash, lookup, speed: cross-check

Status: cross-check of `reference/d2-ds1-edit/docs/guides/animdata-d2.md` against the reference C source.
Read-only; no code in this repo changes. The data file itself is not in the reference tree, so
every number about the vanilla table (3,558 records, bucket counts, dead duplicates) is **NOT IN SOURCE**.

## Sources and conventions

- Guide: `reference/d2-ds1-edit/docs/guides/animdata-d2.md` (cited as `guide#anchor`). It is a
  disassembly study of D2Common.dll 1.13c, not a description of the editor's code.
- Source (cited as `reference/d2-ds1-edit/...:line`):
  - `src/core/animdata.c` (loader, hash, lookup), `src/core/animdata.h`.
  - `src/core/animdata_math.c` (frame delay from speed).
  - `src/structs.h:441-462` (`ANIM_DATA_RECORD_S`, `ANIM_DATA_S`).
  - `src/core/cof.c:385-400` (use of speed and fpd), `src/render/preview.c:1915-1946` (frame advance),
    `src/main.c:1842` (40 ms tick), `src/core/compose_apng.c:40` (delay for export).
  - `test/test_animdata_speed.c` (unit tests for the delay converter).
- Verdicts: **AGREES**, **DIFFERS**, **NOT IN SOURCE**, **NOT IN GUIDE**.

---

## 1. File layout

### 1.1 Whole file

No header, no magic, no trailer. The loader reads blocks back to back until the end of the buffer
(`animdata.c:61-98`). The guide's model is 256 buckets, each `int32 count` followed by `count` records
(`guide#anatomy-of-the-file`). Repo's loader does not count 256. It just walks blocks.

| Item | Repo | Source |
|---|---|---|
| Path | `Data\Global\AnimData.d2` via `misc_load_mpq_file` | `animdata.c:32,47-52` |
| Min size | buffer must be > 4 bytes | `animdata.c:53` |
| Block | `int32 nb_rec`, then `nb_rec` records of 160 bytes | `animdata.c:62-66,70,91` |
| Next block | `lptr += 40 * nb_rec` (in `int32` units, so 160 bytes per record) | `animdata.c:91` |
| End of walk | pointer `>= buffer + size` after skip | `animdata.c:92-97` |

### 1.2 Record (160 bytes, packed)

| Offset | Size | Type | Field | Use in repo | Source |
|---|---|---|---|---|---|
| 0x00 | 8 | char[8] | `cof_name` (token+mode[+class], NUL-padded) | key, read with `strncpy(…,8)` | `structs.h:443`; `animdata.c:72,141` |
| 0x08 | 4 | int32 | `frames_per_dir` | read; **not applied** to frame count (§4) | `structs.h:444`; `animdata.c:144`; `cof.c:396-397` |
| 0x0C | 4 | int32 | `speed` | base rate: `spd_mul = speed`, `spd_div = 256` | `structs.h:445`; `animdata.c:145`; `cof.c:398-399` |
| 0x10 | 144 | UBYTE[144] | `flags` (per-frame event bytes) | **never read** | `structs.h:446`; grep: no use |

- Size is checked at compile time: `typedef char ...[(sizeof == 160) ? 1 : -1]` (`structs.h:451-452`).
- Fields are `int32_t`, not `long`. `structs.h:432-440` documents the LP64 bug that caused a 168-byte stride
  when `long` was used. Port this as `i32`/`u32` with explicit widths.
- Guide's field table (`guide#record-layout`) matches byte for byte (AGREES).

### 1.3 In-memory index

| Array | Type / size | Source |
|---|---|---|
| `block_pointer[256]` | `ANIM_DATA_RECORD_S *` | `structs.h:460` |
| `nb_records[256]` | `int` | `structs.h:461` |
| `buffer`, `buffer_size` | the raw file | `structs.h:458-459` |

Filled by `animdata.c:84-88`. Empty blocks are not stored.

---

## 2. Hash

Source: `animdata.c:13-26`.

```
nb = strlen(name)
for each i: if name[i] == '.' then nb = i        // LAST '.' wins
hash = 0 (UBYTE)
for i in 0..nb-1: hash += toupper(name[i])       // mod 256 by UBYTE
return hash
```

| Rule | Repo | Guide | Verdict |
|---|---|---|---|
| Uppercase before sum | `toupper` | ASCII `a`-`z` only (`guide#from-a-name-to-a-bucket`) | AGREES for ASCII |
| Sum all bytes of the name | yes, mod 256 | yes, `AND 0xFF` | AGREES |
| Stops at a dot | yes. Uses the **last** dot. | not described (name is always 8 chars, no dot) | **DIFFERS** (edge case, see §5) |
| `toupper` on negative `char` | UB for non-ASCII | not described | §5 |

Worked example, `AMTNHTH`: `65+77+84+78+72+84+72 = 532`, `532 mod 256 = 20` (guide). Repo gives the same
value by the same arithmetic. Not run here: no data file in the tree.

---

## 3. Lookup

### 3.1 Algorithm (repo)

`animdata_get_cof_info_quiet(name, &fpd, &speed)` (`animdata.c:120-153`):

1. NULL pointers: zero outputs, return -1.
2. `hash = animdata_hash_value(name)`.
3. If `block_pointer[hash]` is set, scan `i < nb_records[hash]`:
   - copy the record name (8 bytes) into an 80-byte zeroed buffer,
   - `stricmp(rec_name, name) == 0` → set `*fpd`, `*speed`, return 0.
4. Miss: `*fpd = 0`, `*speed = 0`, return -1.

`animdata_get_cof_info` (`animdata.c:106-117`) does the same lookup and prints debug text. `cof.c:394`
uses it at load time. `compose_apng.c:40` uses `animdata_get_frame_delay_ms`, which calls the quiet variant.

### 3.2 Case and matching

| Rule | Repo | Guide | Verdict |
|---|---|---|---|
| Case-insensitive match | `stricmp` (`animdata.c:142`) | on-disk names uppercase; compare on uppercased bytes (`guide#from-a-name-to-a-bucket`) | AGREES |
| First match in bucket wins | yes (loop returns on first) | yes, "first match the scan reaches" (`guide#quirks-in-the-real-table`) | AGREES |
| Max name length | query compared to an 8-byte field, so queries longer than 8 never match | compare aborts with errors `0xd6`/`0xd7` if > 8 chars | **DIFFERS** (silent miss vs error; same visible result) |
| `.cof` in query | hash stops at the dot, but `stricmp` compares the full string with `.cof`. **Result: miss.** | builds bare key `%s%s%s` with no extension (`guide#one-name-builder-two-destinations`) | **DIFFERS** (repo comment `animdata.c:104-106` says the extension "won't be used"; code shows it does matter) |

### 3.3 Duplicates and bucket identity

- A block's bucket is taken from its **first record's** name hash (`animdata.c:70-73`), not from its position.
  Repo does not check that position `i` and hash `i` agree.
- Two non-empty blocks with the same first-record hash: the later one overwrites `block_pointer[hash]`
  (`animdata.c:86-87`). The earlier block becomes unreachable. The guide says this does not happen in vanilla
  ("every record hashes to its own bucket", `guide#the-hash`). Not verifiable here.
- Duplicate names inside one bucket: first record wins. The guide reports dead duplicates in vanilla
  (`guide#quirks-in-the-real-table`, `MINUHTH`, `VMS1HTH`, `64A1HTH`, `3DNUHTH`). Repo would also use the first.
  NOT IN SOURCE (data).

### 3.4 Miss behaviour

| Caller | Repo | Guide | Verdict |
|---|---|---|---|
| `cof.c` | `fpd` and `speed` = 0 from `animdata.c:150-151`. Caller keeps `spd_mul = 1`, `spd_div = 256` (`cof.c:385-387`), which is 1/256 frame per tick. | `ANIM_LookupAnimDataByPath` returns a zeroed record; `GetAnimDataFrameInfo` substitutes `0x800` and `0x100` (`guide#when-the-lookup-misses`) | **DIFFERS** (repo policy; game's two paths disagree) |
| `compose_apng.c:40` | `animdata_get_frame_delay_ms` returns `default_ms` (`animdata.c:162-163`) | n/a | repo-only |

---

## 4. Speed and timing

### 4.1 Speed → frame delay (`animdata_math.c:7-21`)

```
if speed <= 0 or speed > 65535: return default
delay_ms = 10240 / speed              // 256 * 40 ms
if delay_ms <= 0: return default
if delay_ms > 10000: delay_ms = 10000
return delay_ms
```

Unit tests: `test/test_animdata_speed.c` covers 256 → 40 ms, 512 → 20, 128 → 80, 64 → 160, 0 and negatives → default,
70000 → default, 1 → clamp to 10000 (`test_animdata_speed.c:13-55`).

### 4.2 Speed → animation advance (editor)

| Step | Rule | Source |
|---|---|---|
| Tick | `al_create_timer(1.0/25.0)`, 40 ms | `main.c:1842`; `interface.c:164` |
| Base rate | `spd_mul = speed` (from AnimData), `spd_div = 256` | `cof.c:398-399` |
| Override | Objects.txt `FrameDelta<mode>` replaces `spd_mul` when the object row is found (`0` → `256`) | `cof.c:472-477` |
| Advance per tick | `n = ticks·spd_mul + spd_mod`; `cur_frame += n / 256`; `spd_mod = n % 256`; wrap at `fpd` | `preview.c:1937-1943` |

So `speed = 256` is one frame per tick (25 fps). `speed = 128` is one frame every two ticks. This matches
the guide's model (`guide#what-the-record-drives`, "a rate of 256 advances exactly one frame per tick"). AGREES on
the base formula.

### 4.3 What the repo does not do

- **Per-mode stat scaling.** Guide's `UNITS_SetGfxSelected` multiplies `AnimSpeed` by walk/cast/attack stats
  (`guide#playback-rate`). Repo has no unit stats and uses the raw value. DIFFERS (out of editor scope).
- **Frame events.** `flags[144]` is never read (`structs.h:446`; grep: no use). Guide's event semantics
  (`1` strike, `2` missile launch, `3`, `4`) are **inference** per the guide itself (`guide#what-the-record-drives`).
  NOT IMPLEMENTED. If the Rust port needs sound or missile triggers, it must read `0x10 + frame`.
- **FramesPerDir.** Read into `animdata_fpd` but the assignment to `cof->fpd` is commented out (`cof.c:396-397`).
  COF's own `fpd` byte governs frame count. Guide says the record's `FramesPerDir` sets the wrap point
  (`guide#playback-rate`). DIFFERS. Repo's choice is the safer one: COF and DCC must agree anyway.

---

## 5. Cross-check of guide claims

| # | Guide claim (anchor) | Source | Verdict | Note |
|---|---|---|---|---|
| A1 | 570,304-byte file, 256 buckets of `4 + count·160` (`#anatomy-of-the-file`) | `animdata.c:61-98` | AGREES (structure), DIFFERS (count) | Repo walks blocks until EOF and does not check for 256. Size check is not done. |
| A2 | Record: name 8, frames 0x08, speed 0x0C, events 0x10 (144) | `structs.h:441-452` | AGREES | Compile-time size check present. |
| A3 | Hash: uppercase, sum, mod 256 (`#from-a-name-to-a-bucket`) | `animdata.c:13-26` | AGREES | Repo also cuts at last `.`. |
| A4 | Bucket index = hash of the name (every record stored in its own bucket) | `animdata.c:70-73,86-87` | AGREES (for vanilla, per guide) | Repo infers bucket from first record, not position. |
| A5 | Compare names of 8 chars or fewer, errors otherwise | `animdata.c:141-142` | DIFFERS | Repo: silent miss. |
| A6 | Lookup key is bare `%s%s%s` with no `.cof` (`#one-name-builder-two-destinations`) | `animdata.c:104-106,142` | AGREES with guide's key; DIFFERS if caller passes `.cof` | See §3.2. |
| A7 | First match in bucket wins | `animdata.c:138-147` | AGREES | |
| A8 | Miss: zeroed record (lookup) / 0x800, 0x100 (accessor) | `animdata.c:150-151` | DIFFERS | Repo returns 0/0 to the caller, which falls back to `spd_mul = 1`. |
| A9 | `AnimSpeed` 256 = one frame per tick (`#playback-rate`) | `animdata_math.c:14-15`, `preview.c:1939-1941`, `main.c:1842` | AGREES | |
| A10 | Speed scaled by per-mode unit stats (`UNITS_SetGfxSelected`) | none | NOT IN SOURCE (no stats in editor) | Out of scope. |
| A11 | Events at `0x10 + frame`, codes 1/2/3/4 (`#what-the-record-drives`) | none | NOT IMPLEMENTED | Guide's meanings are inference. |
| A12 | `FramesPerDir` sets the wrap point | `cof.c:396-397` (commented out) | DIFFERS | Repo uses COF `fpd`. |
| A13 | AnimData loaded once at startup | `main.c:1643-1646`; `area_browser.c:815` | AGREES | Also reloaded from `area_browser.c`. |
| A14 | 3,558 records; 120 empty buckets; busiest 67 | none | NOT IN SOURCE | Data not in tree. |
| A15 | 29 duplicate names, 9 with different timing | none | NOT IN SOURCE | Data not in tree. Repo behaviour (first wins) would match the guide's "used" row. |
| A16 | `42DTHTH` declares 200 frames, events clamp to 144 | none | NOT IN SOURCE | Repo never reads events. |
| A17 | 1.09d file identical to 1.13c | none | NOT IN SOURCE | |
| A18 | Name is NUL-padded, uppercase on disk | `animdata.c:71,141` | AGREES (NUL-padding); uppercase not required | |
| A19 | `EAnimData.d2` exists, purpose unknown | none | NOT IN SOURCE | |

---

## 6. Bounds checks

The repo's loader and lookup do not check many bounds. An implementer needs all of these.

| # | Where | Issue | Suggested rule |
|---|---|---|---|
| B1 | `animdata.c:65` | reads `int32` count without checking 4 bytes remain. A trailing 1–3 bytes overreads. | Require `pos + 4 <= size` before reading a count. |
| B2 | `animdata.c:65,91` | negative `nb_rec` moves `lptr` backward (`40 * nb_rec`). Can loop without end. | Reject `nb_rec < 0`. Require `pos + 160*nb_rec <= size`. |
| B3 | `animdata.c:138-141` | `rec_ptr[i]` read for `i < nb_records`, no check against buffer end. | Validate once at load, when the block is accepted. |
| B4 | `animdata.c:24` | `toupper(name[i])` with `char` (signed) for non-ASCII is UB. | Cast to `unsigned char` before `toupper`, or use ASCII-only uppercase. |
| B5 | `animdata.c:18-21` | the last dot wins (`nb = i` on every dot). | Decide intent. Only matters if names contain dots. |
| B6 | `animdata.c:86-87` | a second non-empty block with the same first-record hash silently replaces the first. | Detect and report duplicate buckets at load. |
| B7 | `animdata.c:53` | `buffer_size <= 4` is the only size check. | Also reject files whose block walk does not end exactly at EOF (guide says vanilla does). |

---

## 7. Open points

1. **Extension handling.** `.cof` in a query fails the lookup (§3.2). The comment at `animdata.c:104-106`
   says the extension "won't be used". Decide: strip the extension before the compare, or reject it. The
   guide's bare-key rule implies stripping.
2. **Bucket identity.** Repo keys blocks by the first record's hash, not position (§3.3). Vanilla should be
   consistent (guide). Confirm on a real file, and make the Rust loader assert it.
3. **Miss policy.** Repo: speed 0 then `spd_mul = 1`, nearly static. Guide: two different fallbacks in the
   game. Pick one for the editor and document it.
4. **Frame events** are not read. Needed only if the port drives sound or missile triggers.
5. **Per-mode stat scaling** is absent. Expected for the editor; list it so no one assumes parity with the game.
6. **Vanilla data facts** (record count, dead duplicates, bucket spread, 1.09d identity) are **not verifiable**
   here: no `AnimData.d2` in the reference tree. The companion `animdata-d2.verification.md` was not read in full.
