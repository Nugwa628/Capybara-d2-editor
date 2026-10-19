# DS1 map format: editor spec (cross-checked)

Status: cross-check of `reference/d2-ds1-edit/docs/guides/ds1-map-format.md` against the
reference C source. Written by spec-maps, read-only pass.

Conventions
- `src/...` paths are relative to `reference/d2-ds1-edit/` (the MIT reference).
- `guide#slug` cites `reference/d2-ds1-edit/docs/guides/ds1-map-format.md`, by heading.
- Verdicts: AGREES, DIFFERS (explained), NOT IN SOURCE (the claim is game-binary or data
  evidence this repo does not reproduce; not refuted, not confirmed here).
- `docs/internal/ds1-validation.md` and `docs/internal/ds1-decoding.md` were NOT used. Their
  fixed layer counts are wrong.

Primary code: `src/core/ds1.c` (reader `ds1_read` 347-1348, writer `ds1_save` 1354-1809),
tile lookup 122-317, `src/misc.c` (block table 224-337, conflicts 136-220),
`src/render/preview.c` (draw passes 2574-2892), `src/main.c` (workspace keys 82-88).

---

## 1. Version ladder

Version is the first dword. Reader does not reject any version (no upper bound).
All gates are `>=` unless noted.

| Gate | Reader line | Field enabled | Verdict vs guide |
|---|---|---|---|
| v >= 2 | `ds1.c:941` | object count + objects | AGREES |
| v >= 3 | `ds1.c:585` | tileset filename list (count + strings) | AGREES |
| v >= 4 | `ds1.c:617` | explicit wall-layer count | AGREES |
| v >= 6 | `ds1.c:973` (written as `version > 5`) | 5th object dword `ds1_flags` | AGREES |
| v < 7 | `ds1.c:830` | orientation remapped through `dir_lookup[25]` | AGREES |
| v >= 8 | `ds1.c:556` | act dword | AGREES |
| v in 9..13 | `ds1.c:613` | skip 2 dwords (`ptr += 2`) | AGREES |
| v >= 10 | `ds1.c:566` | tagType dword | AGREES |
| v >= 12 | `ds1.c:1020` (also needs tagType 1 or 2) | group block | AGREES |
| v >= 13 | `ds1.c:1059` | 5th dword per group record | AGREES |
| v >= 14 | `ds1.c:1071` | NPC path block | AGREES |
| v >= 15 | `ds1.c:1142,1164,1195` | 3rd dword (`action`) per path point | AGREES |
| v >= 16 | `ds1.c:624` | explicit floor-layer count | AGREES |
| v >= 18 | `ds1.c:1025` | one skipped dword before group count | AGREES |

Note: the guide lists "v >= 5 type-1 monpreset remap" and "type-4 objects". The reader
has no version gate for either (see section 8). The `v >= 5` row is therefore a game
behaviour only, not an editor gate.

Verdict on "no maximum version": AGREES (no version check anywhere in `ds1_read`).

---

## 2. Layout table (file order, reader view)

All fields little-endian int32 unless noted. `wN` = wall count, `fN` = floor count.
Offsets are relative to the start of the field's predecessor; there is no fixed header
offset past the first three dwords.

| # | Field | Size | Type | Condition | Meaning | Reader line |
|---|---|---|---|---|---|---|
| 1 | version | 4 | int32 | always | file version; kept in `glb_ds1.version` | `ds1.c:536` |
| 2 | width-1 | 4 | int32 | always | width in tiles = value + 1 | `ds1.c:540` |
| 3 | height-1 | 4 | int32 | always | height in tiles = value + 1 | `ds1.c:544` |
| 4 | act | 4 | int32 | v>=8 | 0-based on disk; stored internally 1-based, clamped to 5 | `ds1.c:555-561` |
| 5 | tagType | 4 | int32 | v>=10 | 1 or 2 = tag layer + groups present | `ds1.c:565-574` |
| 6 | fileCount | 4 | int32 | v>=3 | number of NUL-terminated strings that follow | `ds1.c:585-587` |
| 7 | filenames | var | char[] | v>=3 | tileset list; kept in `file_buff` (not used for DT1 loading) | `ds1.c:592-608` |
| 8 | unknown A | 4 | int32 | v in 9..13 | skipped, always (5000) in corpus per guide | `ds1.c:613-614` |
| 9 | unknown B | 4 | int32 | v in 9..13 | skipped | `ds1.c:613-614` |
| 10 | wallCount | 4 | int32 | v>=4 (else 1) | number of wall+orientation pairs | `ds1.c:617-635` |
| 11 | floorCount | 4 | int32 | v>=16 (else 1) | floor layers | `ds1.c:624-630` |
| 12 | cell layers | w*h*4 each | 4 bytes/cell | see section 3 | walls, orientations, floors, shadow, tag | `ds1.c:641-664, 790-931` |
| 13 | objectCount | 4 | int32 | v>=2 | objects follow | `ds1.c:941-943` |
| 14 | objects | 16 or 20 each | int32 x4 (+flags) | flags v>=6 | type, id, x, y (sub-tiles), flags | `ds1.c:962-978` |
| 15 | unknown skip | 4 | int32 | v>=18 and tagType 1..2 | skipped | `ds1.c:1025-1026` |
| 16 | groupCount | 4 | int32 | v>=12 and tagType 1..2 | groups follow | `ds1.c:1028` |
| 17 | groups | 16 or 20 each | int32 x4 (+unk) | unk v>=13 | x, y, width, height (tag-layer units), unk | `ds1.c:1045-1064` |
| 18 | npcCount | 4 | int32 | v>=14 | path blocks follow | `ds1.c:1071-1077` |
| 19 | path blocks | 12 + pts | int32 | v>=14 | per block: pointCount, anchorX, anchorY, then points | `ds1.c:1079-1086` |
| 20 | path point | 8 or 12 | int32 x2 (+action) | action v>=15, else action=1 | x, y, action | `ds1.c:1189-1201` |

Read order in the writer matches the reader (v18 form): `ds1.c:1454-1805`.

---

## 3. Layer order

Two orders. Reader builds `lay_stream[]` and then reads one whole layer at a time (row-major,
x fastest, `y` outer, `x` inner) at `ds1.c:792-931`.

| Version | Order (stream values from `lay_stream`) | Lines |
|---|---|---|
| v < 4 | wall 1, floor 1, orientation 1, tag, shadow (5 blocks; wall forced 1, floor forced 1, tag forced 1 at `ds1.c:635-637`) | `ds1.c:641-649` |
| v >= 4 | wall 1, orientation 1, wall 2, orientation 2, ..., wall N, orientation N, floor 1..M, shadow, [tag if tagType 1 or 2] | `ds1.c:652-663` |

Notes
- Shadow is always present (`s_num = 1`, `ds1.c:528`, `660-661`).
- Tag is present only for tagType 1 or 2 when v>=10 (`ds1.c:572-573`, `662-663`). For v<4 a tag
  block is consumed even though tagType is 0 (`ds1.c:637`). For v in 4..9 no tag is read.
- Verdict: AGREES with guide#the-layers and the Appendix, including the five-block v<4 order.

Cell storage in memory: layer buffers are interleaved per cell (`index = cell * layerCount +
layer`, e.g. `ds1.c:1616-1617`), not per layer. The writer uses the same indexing (`ds1.c:1677`).

### Structural limits (editor)

- `WALL_MAX_LAYER=4`, `FLOOR_MAX_LAYER=2`, `SHADOW_MAX_LAYER=1`, `TAG_MAX_LAYER=1`
  (`src/structs.h:30-33`).
- The reader switch is keyed on the stream value (`ds1.c:798-888`). Wall streams are 1..4, so
  a file with five or more walls (`lay_stream = 1+x` reaches 5) would collide with the
  orientation case. The guide says no vanilla file exceeds 4 walls or 2 floors (data, not
  verified here). DIFFERS in that the reader has no guard; see open points.

---

## 4. Cell bit fields

Cell = 4 bytes, read as separate bytes `prop1..prop4` (walls/floors/shadows, `ds1.c:808-814,
845-851, 864-870`). Orientation is one byte of a separate dword stream (`ds1.c:829-833`).

| Field | Bits | Editor use | Source |
|---|---|---|---|
| prop1 | 0..7 | `== 0` means empty (renderer and tile lookup). Also used as the **draw-order height** in preview (see below). | `ds1.c:129,206,240`; `preview.c:868,1086,1326,1444` |
| prop2 | 8..15 | `sub_index` | `ds1.c:138,214,248` |
| prop3 | 16..23 | main_index bits 0..3 = prop3 bits 4..7 | `ds1.c:137` |
| prop4 | 24..31 | main_index bits 4..5 = prop4 bits 0..1; prop4 bit 7 (dword bit 31) = Hidden (renderer only) | `ds1.c:137`; `preview.c:703,886,1121,1344,3418` |

Derived key (matches guide#the-cell, the `(cell >> 20) & 0x3F` form):

    main_index = (prop3 >> 4) + ((prop4 & 0x03) << 4)     ; ds1.c:137,213,247
    sub_index  = prop2
    orientation = orientation stream byte (low byte only, ds1.c:829-833)

Verdict: AGREES with guide#the-cell on byte positions and on the main_index formula.

Other bits: the editor does not name them. `prop1` bits 0/1/6 (layer-type tag) and `prop4`
bits 2/3/4/5 (IsShadow 0x08 etc.) are not referenced by `ds1.c` or the render passes reviewed.
Mark NOT IN SOURCE for guide's bit-name table, except Hidden (0x80).

DIFFERS (editor interpretation): the guide says no decoder reads `prop1` (guide#the-cell). The
renderer sorts each tile's floor/wall/roof layers by the full `prop1` byte, descending, ties by
layer index ascending (`preview.c:868,886` via `wpreview_sort_order_data` at `557-578`; walls
use orientation 10 forced to 255 at `preview.c:1087-1088`). Treat `prop1` as a draw priority
in the editor, not only as a layer tag.

---

## 5. Which cells count as empty (renderer and writer)

### Renderer skip rules

| Pass | Empty test | Hidden (prop4 & 0x80) | Unknown (bt_idx -1) | Source |
|---|---|---|---|---|
| Walls (lower/upper) | `bt_idx == 0` skip | bt_idx = -1 unless orientation 10/11; drawn as "unknown" only if F9 (special) layer on | skipped when F9 off | `preview.c:1110-1111`, `1121-1132` |
| Floors | `bt_idx == 0` skip | bt_idx = -1 -> unknown diamond if F9 on | same | `preview.c:883-884,886-887` |
| Shadows | `bt_idx <= 0` skip (after type check BT_SHADOW) | bt_idx = -1 -> unknown diamond if F9 on | same | `preview.c:706-710,742-748` |
| Roofs | `bt_idx <= 0` skip; type must be BT_ROOF | bt_idx = -1 | not drawn | `preview.c:1341-1353` |
| Special (10/11) | `bt_idx == 0` skip; orientation must be 10 or 11 | n/a | n/a | `preview.c:1461-1470` |

"unknown" = `prop1 != 0` but no block-table entry matched (`bt_idx == -1`). "Hidden" in the
renderer is `prop4 & 0x80` (cell bit). `IS_HIDE(flags)` (`structs.h:52`) is the EDITOR's per-cell
state, a different thing. Do not conflate them.

### Writer "used" tests

- `minimize_ds1` (config, `ds1.c:1604`): a wall layer is kept if any cell has any nonzero byte
  in prop1..4 or orientation (`ds1.c:1618-1619`); a floor layer if any of prop1..4 is nonzero
  (`ds1.c:1646`). Layers above index 0 are dropped from the top down; layer 0 is always kept.

Verdict on guide claim "prop1 == 0 means no tile": AGREES as the editor rule (`ds1.c:129`).
The corpus figures (2,761,752 empty cells, zero dword) are DATA claims, NOT IN SOURCE.

---

## 6. Orientation remap for version < 7

`dir_lookup[25]`, reader local, `src/core/ds1.c:362-366`:

    index : 0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24
    value : 0  1  2  1  2  3  3  5  5  6  6  7  7  8  9 10 11 12 13 14 15 16 17 18 20

Applied at `ds1.c:830-831` as `o_ptr->orientation = dir_lookup[*bptr]`. No bounds check
(index is one unsigned byte, so values 25..255 read past the table). Verdict: values match guide#orientation
byte for byte. The guide also notes no bounds check; AGREES.

---

## 7. Tile key and lookup (cell -> block table)

Block table built per DS1 in `misc_make_block_table` (`src/misc.c:224-337`). Entry 0 is a
sentinel (orientation -1, `misc.c:252-261`). `bt_idx == 0` is the "no tile" marker used by the
renderer (`ds1.c:132,209,243`).

Lookups (per cell, `ds1_make_prop_2_block` at `ds1.c:267-317`):

| Layer | Required orientation | Match keys | Used_by_editor required | Fallback | Source |
|---|---|---|---|---|---|
| wall | cell orientation (1..19, not 0) | orientation, main, sub | yes | see 18/19 and sub-0 fallbacks | `ds1.c:122-194` |
| floor | 0 | main, sub | yes | none | `ds1.c:199-228` |
| shadow | 13 | main, sub | yes | none | `ds1.c:233-262` |

Fallbacks for WALLS only (`ds1.c:153-193`):
1. orientation 18 -> 19, or 19 -> 18; search same main and sub (`ds1.c:154-175`).
2. still not found: swap orientation as in step 1, sub_index forced to 0 (`ds1.c:177-192`).
Comment in source: "trick of O=18 M=3 I=1 ---> O=19 M=3 I=0" (`ds1.c:153`).

The guide does not describe these fallbacks (they are editor heuristics). Mark them editor-only.

Which block is chosen when several share a key: `used_by_editor` picks one block per key group
(see `dt1.md`, section 8). `ds1.c` only requires the flag.

---

## 8. Objects

Record layout (`ds1.c:962-978`):

| Offset | Size | Field | Condition |
|---|---|---|---|
| +0 | 4 | type | always |
| +4 | 4 | id | always |
| +8 | 4 | x (sub-tiles) | always |
| +12 | 4 | y (sub-tiles) | always |
| +16 | 4 | ds1_flags | v>=6 |

Record size 16 bytes (v<6) or 20 bytes (v>=6). Coordinates are sub-tiles (5 per tile).

Loader integrity rule (editor-only, `ds1.c:981-995`): objects with `x` or `y` outside
`[0, new_width*5)` / `[0, new_height*5)` are dropped at load. Source comment says the game
does not do this ("not done by the game I believe"). Dropped objects are not saved.

Object description (editor): `editobj_make_obj_desc` (`src/editor/objects.c:344-409`) looks up
(act, type, id) in `data/obj.txt`. If not found and type is 1 or 2, it renormalises act and id
with `glb_config.nb_type{1,2}_per_act` (`objects.c:380-391`) and retries. It does NOT use a
monpreset/objpreset table or the game's `id < 150` split (guide#objects).

Type-1 / type-2 / type-4 semantics:
- Game remaps (guide): type-1 id via `monpreset` per act; type-2 id via `table[act*150+id]`,
  `id-150` for id >= 150; type-4 via a 3-char code table. NOT IN SOURCE for these remaps;
  the editor uses obj.txt keyed by (act, type, id).
- Guide: v<5 uses raw ids, type-2 rejected at 573. NOT IN SOURCE (no such check in `ds1.c`).
- Guide: type-4 never occurs in vanilla. NOT IN SOURCE (data).

Verdict: field layout AGREES; object id semantics DIFFERS (editor uses obj.txt lookup, not the
game's remap). Flags dword: stored and written back unchanged (`ds1.c:976`, `1763`).

---

## 9. Groups (tag layer) and NPC paths

### Groups (`ds1.c:1020-1066`)

- Gate: v>=12 AND tagType in {1, 2} (`ds1.c:1020-1022`). Reader does NOT read groups for other
  tagTypes.
- v>=18: one skipped dword (`ds1.c:1025-1026`).
- Record: x, y, width, height; plus unk at v>=13 (`ds1.c:1045-1064`).
- Reads are guarded by `ds1_ptr_in_buffer` (`ds1.c:1047-1058`), so truncated group tails read as
  zero (no out-of-bounds read there). This is how `ACT1/OUTDOORS/trees.ds1` is handled
  (comment `ds1.c:1013-1018`).
- Writer (`ds1.c:1769-1787`): writes a zero dword, then group_num, then 5 dwords per group. It
  writes this block whenever `tag_type != 0`, which is broader than the reader's {1, 2} gate.
  For tagType values 3+ the file is re-serialised with groups the reader never read. Edge case.

### NPC paths (`ds1.c:1071-1207`)

- Count guarded by `ds1_ptr_in_buffer` (`ds1.c:1073-1077`).
- Block: pointCount, anchorX, anchorY (`ds1.c:1081-1086`).
- Association is by coordinate: the first two objects matching (x, y) are searched
  (`ds1.c:1091-1106`). If two or more objects share the coordinate, the paths are dropped and
  skipped in the stream (`ds1.c:1108-1152`, warning printed at `1113-1119`).
- If exactly one object matches, paths attach to it (`ds1.c:1153-1201`). Blocks whose anchor
  matches no object are skipped (`ds1.c:1159-1174`).
- Path point: x, y, then action if v>=15 (`ds1.c:1191-1197`); action defaults to 1 otherwise
  (`ds1.c:1201`). Editor cap: `WINDS1EDIT_PATH_MAX` (`ds1.c:1179-1185`).
- Writer (`ds1.c:1790-1805`) writes the path blocks in object order, one block per object that has
  paths. Verdict: AGREES with guide#npc-paths on field order and coordinate keying.

---

## 10. DS1EDIT_* workspace keys (saved inside the tileset list)

Key strings (`src/main.c:82-88`, enum `src/structs.h:1338-1346`):

| Enum | Key | Value written | Writer line | Reader line |
|---|---|---|---|---|
| WRKSPC_TILE_X | `DS1EDIT_WRKSPC_TILE_X` | tile x of view centre (tile units; divided by 5 in object modes O/P/L) | `ds1.c:1564-1567,1550-1559` | `ds1.c:1272-1273` |
| WRKSPC_TILE_Y | `DS1EDIT_WRKSPC_TILE_Y` | tile y, same rule | `ds1.c:1569-1572` | `ds1.c:1276-1277` |
| WRKSPC_ZOOM | `DS1EDIT_WRKSPC_ZOOM` | `glb_ds1.cur_zoom` | `ds1.c:1574-1577` | `ds1.c:1280-1281` (ignored, `ds1.c:1312`) |
| WRKSPC_VERSION | `DS1EDIT_VERSION` | `Build YYYY/MM/DD <build>, <dll>` | `ds1.c:1579-1585` | `ds1.c:1284-1285` (ignored) |
| WRKSPC_SAVE_COUNT | `DS1EDIT_SAVE_COUNT` | incremented on each save, `%lu` | `ds1.c:1588-1596` | `ds1.c:1288-1300` |

Format: `KEY=value` followed by NUL, appended as additional entries of the tileset list
(`file_count += WRKSPC_MAX`, `ds1.c:1508-1509`; written after the real names,
`ds1.c:1547-1599`). Read back only when `glb_config.workspace_enable` is set (`ds1.c:1243`).
Read matches by prefix then `=` (`ds1.c:1256-1266`). Reader clamps tile_x/tile_y to the
map and recentres the preview (`ds1.c:1313-1338`).

Writer strips any existing `KEY=` entries before re-adding them (`ds1.c:1482-1545`).

The game ignores the tileset list (guide#the-tileset-list), so these entries are harmless to
the game. They are NOT tileset names. A tool that treats the list as tilesets must skip them.

---

## 11. Writer behaviour (`ds1_save`, `ds1.c:1354-1809`)

| Item | Behaviour | Line |
|---|---|---|
| Version | always 18 | `ds1.c:1454` |
| Act | written as internal act - 1 (0-based) | `ds1.c:1466` |
| tagType | written as stored | `ds1.c:1470` |
| Tileset list | real names kept verbatim; `DS1EDIT_*` entries stripped and re-appended | `ds1.c:1476-1545` |
| Two unknown dwords (v9..13) | not written (v18) | n/a |
| Wall count / floor count | written; reduced only with `minimize_ds1` | `ds1.c:1602-1667` |
| Wall layer | prop1..4 as a block, then orientation as a block (orientation byte + 3 zero bytes) | `ds1.c:1670-1699` |
| Floor layer | prop1..4 only (no orientation) | `ds1.c:1702-1717` |
| Shadow | prop1..4 only | `ds1.c:1720-1735` |
| Tag | written only if tag_type != 0; a v<4 tag (consumed on read) is therefore dropped | `ds1.c:1738-1753` |
| Objects | 20 bytes each (flags always written) | `ds1.c:1756-1766` |
| Groups | only if tag_type != 0; zero dword + count + records with unk | `ds1.c:1769-1787` |
| NPC paths | always written (may be count 0) | `ds1.c:1790-1805` |
| Backup | if target exists, renamed to `<name>-NNN.ds1` (first free NNN from 000), then new file written | `ds1.c:1419-1436` |
| Autosave | `<name>.tmp`, existing tmp removed first | `ds1.c:1407-1413` |
| Copy-on-save | if a project is open and the file is outside it, save redirects into the project tree | `ds1.c:1379-1394` |
| Orientation upper bytes | zero | `ds1.c:1694-1696` |

What the writer drops (relative to the file as read):
- v<4 tag layer (tagType is 0 for v<10).
- Unknown dwords (v9..13 skip pair).
- Objects outside the map bounds (dropped at load, section 8).
- NPC paths on objects that share a coordinate (dropped at load, section 9).
- Any act value 5 is written as 4 (clamp at read).
- Layers beyond the used count, only when `minimize_ds1` is on.

Load-time normalisation that changes data on save: v<7 orientations are already remapped
(section 6), so the saved file contains the remapped values in the v18 interpretation. v<4
layers are re-serialised in the v>=4 interleaved order.

Guide claim "the editor ... rewrites ... changes nothing the game will look at" (tileset list):
the editor preserves real names; the game ignores them. AGREES on behaviour.

---

## 12. Truncated and short files

- Reader allocation: exact file length, no slack (`ds1.c:513` disk path; the MPQ path uses the
  unpacked size, `src/mpq/MpqView.c:243-251`). Guide's "+800 bytes slack" is a game-binary
  allocator behaviour (`RESOURCE_AllocateAndOpen`, guide#the-header). NOT IN SOURCE for the editor.
- Editor bounds checks: only the group block and NPC count (`ds1.c:1047-1058,1073`). Object
  and layer reads are unguarded (`ds1.c:798-888,962-978`). A file shorter than its declared
  layout reads past the buffer there. Guide's `trees.ds1` case (12 bytes past EOF) lands in the
  guarded group area, so the editor copes with that file. A strict implementer must add guards
  for all reads.
- `expansion/Siege/xtransition.ds1` (4 bytes past EOF per guide) is a data claim; NOT IN SOURCE.

---

## 13. Render passes (order of draw)

`wpreview_draw_tiles` (`src/render/preview.c:2574`). Per tile, `y` outer, `x` inner, only for
tiles inside the view rectangle.

| Pass | Lines | Draws |
|---|---|---|
| 1 | 2687-2725 | lower walls (`draw_w upper=FALSE`), floors, shadows (BT_SHADOW only) |
| 1b | 2731-2755 | object shadows (if animation layer on) |
| 2 | 2761-2786 | objects behind walls (orderflag 1) |
| (grid) | 2793-2795 | tile grid if TG_OVERFLOOR |
| 3 | 2797-2835 | upper walls (`draw_w upper=TRUE`) + objects in front (orderflag 0/2) |
| 4 | 2841-2862 | roofs (BT_ROOF) |
| 5 | 2868-2892 | special tiles, orientation 10/11 (F9 special layer) |
| 6 | 2898+ | walkable overlay (debug) |

Wall split (`preview.c:1102-1106`): upper pass draws orientation < 15, lower pass draws
orientation > 15. Orientation 15 is the roof (not drawn as a wall). Orientations 10/11 are
skipped by both wall passes (`preview.c:1113-1119`). Roof vertical offset is
`y1 -= roof_y * height_mul / height_div` (`preview.c:1367`).

Palette selection for rendering: `palette_resolve_index(txt_act, act)` at `preview.c:2599-2600`
(see `palette.md`).

---

## 14. Cross-check: guide claims vs source

| Guide claim (section) | Source | Verdict |
|---|---|---|
| Version ladder, 15 gates (#eighteen-versions-and-no-rejection) | section 1 above | AGREES |
| No upper version bound | `ds1.c:536-1348` (none) | AGREES |
| width/height stored minus one (#width-and-height) | `ds1.c:540,544` | AGREES |
| act 0-based, clamped to 4 (#the-act-field) | `ds1.c:555-561` (stored +1, clamp 5) | AGREES (equivalent) |
| tagType gates tag layer + group block (#tagtype-gates-two-blocks) | `ds1.c:566-574,1020-1021` | AGREES |
| Tileset list read and discarded by game (#the-tileset-list) | editor keeps it (`ds1.c:585-608`) and never loads DT1s from it (`misc.c:762`, `txtread.c:817`) | AGREES on effect |
| Two dwords skipped v9..13 | `ds1.c:613-614` | AGREES |
| Wall count forced 1 below v4; floor forced 1 below v16 | `ds1.c:635-636,627-630` | AGREES |
| Layer order: interleaved v>=4, five-block v<4 (#the-layers) | `ds1.c:641-664` | AGREES |
| Cell bits: main_index formula (#the-cell) | `ds1.c:137,213,247` | AGREES |
| prop1 never read by game (#the-full-bit-map) | editor sorts by prop1 at `preview.c:868,1086` | DIFFERS (editor interprets prop1 as draw priority) |
| Bit 27 IsShadow | no reference in `ds1.c`/`preview.c` | NOT IN SOURCE |
| Hidden = prop4 bit 0x80 | `preview.c:703,886,1121` | AGREES |
| Empty cell = prop1 == 0 | `ds1.c:129,206,240` | AGREES (rule); corpus counts NOT IN SOURCE |
| Orientation byte, upper 3 bytes zero | `ds1.c:829-833`, `1693-1696` | AGREES |
| dir_lookup[25] for v<7 (#orientation) | `ds1.c:362-366` | AGREES, value-by-value |
| Shadow orientation 13, floor 0 implicit | `ds1.c:217,251`; `misc.c:287-307` | AGREES |
| Orientation 10/11 special, warp main_index 30..33 directions (#orientation-10-and-11) | special = `preview.c:1113,2868`; warp direction table not found in `ds1.c` or render passes | AGREES for special; warp table NOT IN SOURCE |
| Object record 16/20 bytes, flags v>=6 (#objects) | `ds1.c:962-978` | AGREES |
| type-1 via monpreset; type-2 via `table[act*150+id]` (#what-type-and-id-mean) | editor uses obj.txt (act,type,id) (`objects.c:344-409`) | DIFFERS (game remap not implemented; editor uses its own table) |
| Type-2 rejected at 573 below v6 | no such check | NOT IN SOURCE |
| Type-4 items recognised | no type-4 handling in `ds1.c` | NOT IN SOURCE (editor passes type through) |
| Group block v>=12, tagType 1..2; v18 skip; unk v>=13 (#groups) | `ds1.c:1020-1066` | AGREES |
| Vanilla 'trees.ds1' stops mid-group block (#groups) | guard `ds1.c:1047-1058` | AGREES on handling |
| NPC paths keyed by coordinate; ambiguous -> discarded (#npc-paths) | `ds1.c:1091-1152` | AGREES |
| Path action default 1 below v15 | `ds1.c:1201` | AGREES |
| Path point order x, y, action (#npc-paths) | `ds1.c:1191-1197` | AGREES |
| RESOURCE_AllocateAndOpen +800 slack (#reading-a-ds1-without-falling-over) | editor allocates exact size (`ds1.c:513`, `MpqView.c:243`) | NOT IN SOURCE (game allocator); editor differs |
| Writer always v18 (not in guide, editor behaviour) | `ds1.c:1454` | (editor behaviour) |

---

## 15. Open points

1. Writer tag gate: groups written for any tagType != 0 (`ds1.c:1769`) but read only for 1..2
   (`ds1.c:1020-1021`). Decide whether tagType 3+ round-trips.
2. Reader: five or more walls or three or more floors mis-decode (switch collides with
   orientation/shadow cases, `ds1.c:798-888`). The guide's maxima are not enforced.
3. Reader: no bounds check on object and layer reads; only group and NPC are guarded.
4. Objects outside bounds are silently dropped at load (`ds1.c:981-995`); the game keeps them
   per the source comment. Changes round-trip behaviour for crafted files.
5. `dir_lookup` index is unchecked for orientation values 25..255 (`ds1.c:831`).
6. prop1 is used as draw priority in the editor; the guide calls it a layer tag. Confirm the
   Rust port uses the same ordering.
7. Guide's act clamp and editor's clamp agree numerically but the editor stores 1-based act
   internally (`ds1.c:558`). Keep the conversion explicit in any port.
8. Guide corpus figures (cell counts, object counts, version census) were not reproduced here.
9. Warp direction table (main_index 30..33) and the object id remap are not implemented in the
   editor; the guide is the only source for them.
10. Orientation 13 on wall cells (eight vanilla cells, guide#orientation): the editor would look
    up orientation 13 as a wall key and may match a shadow block. Unverified.
11. Hidden (prop4 0x80) cells: the renderer shows an unknown-tile diamond only with F9 on. Confirm
    intended behaviour before porting.
