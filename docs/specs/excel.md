# Excel tables (.txt) and data loading: editor spec (cross-checked)

Status: cross-check of `reference/d2-ds1-edit/docs/guides/excel-tables-and-data-loading.md`
against the reference C source. Written by spec-maps, read-only pass.

Conventions
- `src/...` paths are relative to `reference/d2-ds1-edit/`.
- `guide#slug` cites `reference/d2-ds1-edit/docs/guides/excel-tables-and-data-loading.md`.
- Verdicts: AGREES, DIFFERS (explained), NOT IN SOURCE (game-binary or data claim, not reproduced).

Key finding up front: the reference EDITOR never reads `.bin` excel tables and never uses
fixed record sizes. It parses `.txt` by column NAME at run time and computes its own in-memory
stride. Every record size, field offset, and `.txt`/`.bin` fork claim in the guide is a game
fact, not an editor fact.

Primary code: `src/core/txtread.c` (parser 20-466, loaders 466-1218), `src/misc.c`
(column lookup 1245+, MPQ/mod resolution 1189-1241), `src/main.c` (required column lists 94-~240,
MPQ open order 853-862), `src/config.c` (MPQ slots 307-310), `src/core/cof.c` (Objects.txt
consumers 403-521), `src/editor/objects.c` (obj.txt lookup 344-409), `src/core/asset_export.c`
and `src/ui/props_panel.c` (second readers of LvlTypes/LvlPrest).

---

## 1. Tables the editor loads

| Table (game name) | Editor file | Loader | Required columns (main.c) | Record concept | Verdict |
|---|---|---|---|---|---|
| LvlTypes.txt | `Data\Global\Excel\LvlTypes.txt` | `read_lvltypes_txt` `txtread.c:737-875`; cache `txt_ensure_lvltypes` `646-661` | `Id, Act, Name, File 1 .. File 32` (`main.c:94-105`) | one row per tileset level type | editor reads Id, Act, File 1..32 |
| LvlPrest.txt | `Data\Global\Excel\LvlPrest.txt` | `read_lvlprest_txt` `txtread.c:879-1069`; cache `664-679` | `Def, Dt1Mask, File1..File6, Name, LevelId` (`main.c:107-113`) | one row per preset (DS1 layout) | editor reads Def, Dt1Mask, File1..6 (auto-detect only) |
| Levels.txt | `Data\Global\Excel\Levels.txt` | `read_levels_txt` `txtread.c:1204-1218` (archive fallback) | `Name, Id, Act, LevelType, SizeX..Position, SaveMonsters` (`main.c:115-143`) | one row per level | loaded for area browser; field use not traced in this pass |
| obj.txt | `data/obj.txt` in the EDITOR data dir (not a game table) | `read_obj_txt` `txtread.c:1073-1146` | `Act, Type, Id, Direction, Index, Objects.txt_ID, Monstats.txt_ID, Base, Token, Mode, Class, HD..SH, S1..S8, Colormap, Description` (`main.c:145-167`) | editor's object descriptor table | editor-specific |
| Objects.txt | `Data\Global\Excel\Objects.txt` | `read_objects_txt` `txtread.c:1150-1199` | `Id, SizeX, SizeY, FrameCnt0-7, FrameDelta0-7, CycleAnim0-7, Lit0-7, BlocksLight0-7, Start0-7, BlocksVis, ..., OrderFlag0-7, Yoffset, Xoffset` (`main.c:169-~245`) | one row per object | used by cof.c |

Not loaded by the editor (grep found no file reference): `monpreset`, `monstats.txt`,
`superuniques`, `lvlwarp`, `objpreset`, `lvlmaze`, `lvlsub`, `levelsdefs`. Guide#the-catalog and
the record-size claims for these do not apply to the editor.

Loaded by other editor paths (not in the table above):
- `Data\Global\AnimData.d2` (binary, 256-bucket hash, 160-byte records): `src/core/animdata.c:13-87`;
  record struct `src/structs.h:441-452` with a compile-time size check of 160.
- `Data\Global\Excel\MonStats2.txt` (text, own parser): `src/core/monstats2_build.c:25`,
  `src/core/monstats2.c`. Used by the CLI sprite path (`src/cli/cli.c:988-991`).

---

## 2. Text format rules (as the parser implements them)

### 2.1 Splitting

| Rule | Behaviour | Line |
|---|---|---|
| Cell separator | TAB only | `txtread.c:32-37` |
| Line end | LF, or CR followed by ANY character (CR consumes two bytes) | `txtread.c:39-54` |
| Bare CR (no LF) | treated as line end; the next byte is skipped | `txtread.c:47-53` |
| End of text | NUL byte (`txt_read_in_mem` appends one, `txtread.c:534-548`) | `txtread.c:27-30` |
| Header row | first line; names matched with `stricmp` (case-insensitive) | `txtread.c:90-120,103` |
| Empty header cell | ignored for matching; still consumes a column index | `txtread.c:98-111` |
| Duplicate header name | last matching column wins | `txtread.c:103-107` |
| `*eol` header | not counted as a column (obj.txt) and not added as filter | `txtread.c:77,283` |
| Extra columns | only obj.txt keeps unknown headers (user filter columns); other tables keep required columns only | `txtread.c:372-376,265-333` |

### 2.2 Records

| Rule | Behaviour | Line |
|---|---|---|
| Record count | number of line ends after the header (`line_num`) | `txtread.c:183-187` |
| Blank line | counts as a record (all cells empty) | `txtread.c:183-187` (no skip) |
| Last line without line end | its cells are stored, but it is NOT counted, so callers do not iterate it | `txtread.c:183-187` vs `235-245` |
| Expansion separator row | NOT special-cased. It is a record whose numeric cells are 0 | no filter anywhere (grep "Expansion" only in comments, `txtread.c:790`) |
| Callers iterate | `i < line_num` (`txtread.c:770,1120`; `cof.c:459`) | - |

### 2.3 Cell types and storage

| Rule | Behaviour | Line |
|---|---|---|
| Type decision | NUM if every non-empty cell of the column is digits or `-`; otherwise STR. Empty cells do not decide. | `txtread.c:139-173` |
| Column never non-empty | CT_NULL: stores nothing and takes NO space in the record | `txtread.c:154-160,441-446` |
| Number parse | `atol` into a `long` slot | `txtread.c:222-226` |
| String width | max cell length of that column + 1 (no truncation) | `txtread.c:147-148,444-445` |
| `xxx` | not special; a string | NOT IN SOURCE as special (guide#the-txt-format) |
| `*` comment columns | not special (only `*eol`) | NOT IN SOURCE (guide claim) |
| Encoding | raw bytes; no BOM stripping, no charset conversion | `txtread.c:518-551` |
| Stride | per column: `sizeof(long)` for NUM, `size+1` for STR; line_size = sum | `txtread.c:437-447` |

Consequences:
- A UTF-8 BOM makes the first header name wrong (no match), so the required column is missing and
  `txt_load` fails (`txtread.c:406-427`). LvlTypes, LvlPrest and Levels retry the archive copy
  (`txtread.c:612-639`); Objects and obj.txt do not (`txtread.c:1164-1168`, `1093-1097`).
- A CT_NULL column has offset equal to the next column. Reads that do not check `size` or `type`
  will read the wrong bytes. `cof.c:483,490,499` check `size`; `txtread.c:1123-1138` (obj.txt) does not.
- `sizeof(long)` differs by platform (4 on Windows, 8 on LP64). Only matters for an in-memory image.

### 2.4 Missing columns

`misc_get_txt_column_num(table, name)` (`misc.c:1245-1275`) searches the REQUIRED list only, and
calls `ds1edit_error` (fatal) when the name is not in it. Adding a column read requires adding it
to `main.c` lists first.

`txt_load` returns NULL if any required column is absent from the header (`txtread.c:407-427`).

---

## 3. Where the editor reads each file, and in what order

| Step | Behaviour | Line |
|---|---|---|
| Mod overlay | `glb_config.mod_dir[0..]` searched first | `misc.c:1203-1217` |
| Archives | `glb_mpq_struct[0..]` in index order: slot 0 = patch_d2, 1 = d2exp, 2 = d2data, 3 = d2char | `misc.c:1219-1237`; `config.c:307-310`; `main.c:853-862` |
| Lookup by name | hash lookup of the exact path, no listfile needed (`test_tell_entry`) | `src/mpq/MpqView.c:220-231` |
| Fallback | LvlTypes/LvlPrest/Levels: retry archives only if the mod copy fails to parse | `txtread.c:612-639` |
| Data dir | obj.txt, palettes, cmaps from `glb_ds1edit_data_dir` | `txtread.c:1085`; `misc.c:498,561,603` |

Guide#mpq-load-order says the game searches Patch_D2 > D2Exp > D2Xtalk > D2XMusic > D2Xvideo >
D2Data > D2Char > ... AGREES for the four archives the editor configures (same relative order);
the editor does not configure the others (NOT IN SOURCE for them).

---

## 4. The `.txt` / `.bin` fork

- Editor: text only. `read_*` always calls `txt_read_in_mem` on `.txt` (`txtread.c:751,892,1159,1212`).
  No `.bin` table is read. (`.bin` files the editor writes are caches and debug dumps:
  `misc.c:498` cmap, `misc.c:561/603` pal, `undo.c`, `main.c:290-296` debug paths.)
- Guide's flag `g_dwData_9e20` (D2Common 1.13c, default 1 = binary), `.bin` records with a u32
  count, and the `ENCRYPTED|FIX_KEY` archive flags: NOT IN SOURCE (game binary).
- Consequence for a port: the editor's behaviour is "edits to `.txt` are always seen". The game's
  behaviour (guide) is "edits to `.txt` are ignored unless `.bin` is removed". Guide#the-txt-bin-fork
  describes the game. Do not copy the editor's text-only read into a port that claims game parity
  without checking the game path.

---

## 5. Record sizes and field offsets

Game constants in the guide (NOT IN SOURCE for the editor):

| Table | Guide record size | Guide field offsets | Editor |
|---|---|---|---|
| Objects.txt | 0x1C0 (448) | DrawUnder 0x1B6, OpenWarp 0x1B7, AutoMap 0x1BC | none; columns found by name (`txtread.c:1179-1197`) |
| SuperUniques | 0x34 (52) | - | not loaded |
| LvlTypes (ds1 guide) | 0x788 (1928), 60-byte names at 0x00, 0x3C, ... | File columns from 0x00 in 60-byte names (guide) | none; File N by name (`txtread.c:829`) |
| AnimData.d2 | 0xA0 (160) | - | 160-byte struct, asserted at compile time (`structs.h:451-452`); read as raw records (`animdata.c:70`) |
| DATATBLS_GetObjectsTxtRecord | 0x1B8 | - | not relevant |

Editor offsets are computed per load (section 2.3). Do not hard-code them.

---

## 6. Column use by the editor (with lines)

### LvlTypes.txt

| Column | Use | Line |
|---|---|---|
| `ID` (matched to `Id`) | row search for the requested type | `txtread.c:772-774` |
| `Act` | palette act (after override) | `txtread.c:782-784` |
| `File 1` | first DT1 column; File N = `file_1_idx + N-1` in the editor's column array | `txtread.c:794,817,829` |
| `File 1..32` | DT1 paths; "0" skipped; path `/` -> `\`; prefix `glb_tiles_path` | `txtread.c:817-848` |
| all `File N` (type STR) | unanimous `ActN\` folder -> palette act override | `txtread.c:706-735` |
| `Name` | required, not read by the parser | `main.c:96` |

### LvlPrest.txt

| Column | Use | Line |
|---|---|---|
| `Def` | row search (explicit def) | `txtread.c:967,1033-1035` |
| `Dt1Mask` | bit (N-1) enables File N, so bit 0 enables File 1; slot 0 (editor DT1) is always on | `txtread.c:979-993,1043-1057` |
| `File1..File6` | auto-detect by file name when def is -1 | `txtread.c:917-922,929-955` |
| `Name`, `LevelId` | required, not read by the parser | `main.c:111` |

Mask rule disagreement inside the reference:
- `txtread.c:844` and `992` (loader): File 1 gated by bit 0.
- `asset_export.c:1114-1120`: File 1 gated by bit 0 (agrees with loader).
- `props_panel.c:168`: File 1 always included; others gated by bit (fi-1). DIFFERS.
Treat the loader as authority. The UI listing is wrong for File 1 if it disagrees.

### obj.txt (editor file)

| Column | Use | Line |
|---|---|---|
| `Act`, `Type`, `ID`, `Description` | descriptor keyed by (act, type, id) | `txtread.c:1123-1139` |
| `Objects.txt_ID` | links to Objects.txt row (by Id value) | `txtread.c:1143`; `cof.c:403-407` |
| `Monstats.txt_ID`, `Base`, `Token`, `Mode`, `Class`, `HD..SH`, `S1..S8`, `Colormap` | required; token/mode used by COF lookup | `main.c:153-164`; `cof.c:412-436` |

Lookup from a DS1 object (`objects.c:344-409`): exact (act, type, id) in obj.txt; if missing and
type is 1 or 2, act and id are renormalised with `glb_config.nb_type1_per_act` /
`nb_type2_per_act` (`objects.c:380-391`) and retried. No monpreset or game id-150 rule.

### Objects.txt

| Column | Use | Line |
|---|---|---|
| `Id` | search for object id (first match; `id == 0` skipped) | `txtread.c:1179`; `cof.c:439-452` |
| `FrameDelta0..7` | speed multiplier by mode; 0 -> 256 | `txtread.c:1181-1188`; `cof.c:472-477` |
| `Xoffset`, `Yoffset` | sprite offsets when column size > 0 | `cof.c:483-496` |
| `OrderFlag0..7` | draw order by mode; if 0, OrderFlag0 is used | `txtread.c:1190-1197`; `cof.c:499-521` |

Mode index (`cof.c:412-436`): NU=0, OP=1, ON=2, S1=3, S2=4, S3=5, S4=6, S5=7.

### Levels.txt

Loaded for the area browser (`read_levels_txt`, `txtread.c:1204-1218`). Fields used are not
traced in this pass; the required list is `main.c:115-143`.

---

## 7. Cross-check: guide claims vs source

| Guide claim (section) | Source | Verdict |
|---|---|---|
| Tab-separated, header row, CRLF (#the-txt-format) | `txtread.c:20-58` | AGREES (LF also accepted) |
| Row 0 is the header, excluded from counts (#the-txt-format) | `txtread.c:90-120` | AGREES |
| Expansion separator is not a record (#the-txt-format) | editor counts it (`txtread.c:183-187`) | DIFFERS (editor counts it; game claim NOT IN SOURCE) |
| 575 lines - header - Expansion = 573 records (#the-txt-format) | data count NOT IN SOURCE. Editor would give 574 data rows. | DIFFERS in method (data NOT IN SOURCE) |
| Blank cell vs `xxx` (#the-txt-format) | blank = empty string; `xxx` plain string | NOT IN SOURCE for `xxx` meaning |
| `*` comment columns (guide claim) | only `*eol` special | NOT IN SOURCE |
| Objects.txt 160 columns (#the-txt-format) | data NOT IN SOURCE | NOT IN SOURCE |
| Generic loader, FormatDescriptor, record size 0x1C0 (#from-bytes-to-memory) | editor has no descriptor; stride computed | NOT IN SOURCE (game) |
| SuperUniques 0x34, hcIdx 66 abort (#from-bytes-to-memory) | not loaded | NOT IN SOURCE |
| AnimData stride 0xA0, 256-bucket hash (#from-bytes-to-memory) | 160-byte record struct (`structs.h:451-452`); 256 buckets (`structs.h:460`; `animdata.c:86`); hash = byte sum of uppercased name (`animdata.c:13-25`) | AGREES on stride and bucket count; hash details not compared with the animdata guide in this pass |
| .txt/.bin fork, default binary (#the-txtbin-fork) | editor always `.txt` (`txtread.c:751,892,1159`) | NOT IN SOURCE (game); editor differs in behaviour |
| `.bin` = u32 count + records, first record Dummy (#the-txtbin-fork) | no `.bin` reader | NOT IN SOURCE |
| Objects.bin 573 records, 448 each (#the-txtbin-fork) | no `.bin` reader | NOT IN SOURCE |
| MPQ order Patch > Exp > ... > Data > Char (#mpq-load-order) | `config.c:307-310`, `misc.c:1219-1237` | AGREES for the four configured; others NOT IN SOURCE |
| Patch_D2 has no listfile; lookup by hash (#mpq-load-order) | editor looks up by name (`MpqView.c:220-231`) | AGREES on method |
| Objects.txt is overridden in Patch_D2 (#mpq-load-order) | data NOT IN SOURCE | NOT IN SOURCE |
| Objects accessor OBJECTS_GetObjectName index bounds (#runtime-lookup) | editor searches by Id (`cof.c:439-466`), no index bounds in game sense | NOT IN SOURCE (game) |
| Editor reads objects by row index (none claimed) | editor searches by Id value | (editor behaviour) |
| LvlTypes 0x788 record, File columns at offsets (ds1 guide) | editor uses names | NOT IN SOURCE (game) |
| Dt1Mask bit mapping (ds1 guide) | `txtread.c:987-993,1051-1057` | AGREES with loader; DIFFERS in `props_panel.c` (File 1) |

---

## 8. Open points

1. Blank line counts as a record (`txtread.c:183-187`). Check that the shipped LvlTypes,
   LvlPrest and Objects have no blank lines, or skip them in the port.
2. Last record without a trailing line end is dropped from iteration (`txtread.c:183-187`). Port
   should count it, or require a trailing newline.
3. Bare CR is treated as CR-LF and skips a byte (`txtread.c:47-53`). Port should treat CR alone as
   a line end without skipping.
4. No BOM handling (`txtread.c:518-551`). Port should strip a UTF-8 BOM before header matching.
5. CT_NULL columns take no space (`txtread.c:441-446`). Port should treat an all-blank column as
   empty, not as a zero-width column that aliases the next one.
6. `read_lvltypes_txt` File loop: blank (not "0") cells reach `dt1_add` (`txtread.c:831-848`); a
   CT_NULL column reuses stale `tmp` (`txtread.c:831-838`). See `dt1.md` open points 11 and 12.
7. `Expansion` separator row is a record in the editor (`txtread.c:183-187`). Harmless for Id
   lookups (`cof.c:439`, `if (id)` and value match), but a port that indexes by row must handle it.
8. `Dt1Mask` File 1 gating differs between `props_panel.c:168` and `txtread.c:844`.
9. Game behaviour for `.txt` vs `.bin` is the guide's claim (unverified here). The editor does
   not implement the binary path. A port that must match the game needs its own check.
10. `misc_get_txt_column_num` is fatal for names not in the required list (`misc.c:1264-1268`). Any
    new column read must be added to `main.c` first.
11. Editor palette and DT1 choices depend on LvlTypes row content (see `palette.md` section 8 for
    the Id 35 override).
