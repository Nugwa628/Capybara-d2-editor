# DT1 tile format: editor spec (cross-checked)

Status: cross-check of `reference/d2-ds1-edit/docs/guides/dt1-tile-format.md` against the
reference C source. Written by spec-maps, read-only pass.

Conventions
- `src/...` paths are relative to `reference/d2-ds1-edit/`.
- `guide#slug` cites `reference/d2-ds1-edit/docs/guides/dt1-tile-format.md` by heading.
- Verdicts: AGREES, DIFFERS (explained), NOT IN SOURCE (game-binary or data claim not reproduced here).

Primary code: `src/core/dt1.c` (load, block table, zoom), `src/core/dt1_draw.c` and
`src/core/dt1_decode.c` (sub-tile decoders), `src/misc.c` (block table, variant choice,
type classes), `src/txtread.c` (DT1 list from LvlTypes/LvlPrest), `src/render/preview.c`
(draw passes and roof offset), `src/core/asset_export.c` and `src/ui/props_panel.c` (second
readings of Dt1Mask).

---

## 1. File header (276 bytes)

| Offset | Size | Field | Read at | Editor check | Guide verdict |
|---|---|---|---|---|---|
| 0x000 | 4 | version major (`x1`) | `dt1.c:571` | must equal 7 (`dt1.c:211`) | AGREES |
| 0x004 | 4 | flags (`x2`), reads 6 | `dt1.c:572` | must equal 6 (`dt1.c:211`) | DIFFERS: guide says the game never compares this field to 6 (guide#the-fixed-header); editor is stricter. All 354 vanilla files carry 6. |
| 0x008 | 260 | filename buffer, zero on disk | not read | none | AGREES (skipped) |
| 0x10C (268) | 4 | number_of_blocks | `dt1.c:573` | `>= 0` and fits in file (`dt1.c:213-220`) | AGREES |
| 0x110 (272) | 4 | block_header_offset (`bh_start`) | `dt1.c:574` | `>= 0`, `<= file length` (`dt1.c:213-216`) | AGREES on value (276 in corpus). Editor does not require 276. |

Validation summary (`dt1.c:203-223`): buffer >= 276; signatures 7 and 6; `block_num >= 0`;
`bh_start >= 0`; `bh_start <= len`; `block_num <= (len - bh_start) / 96`. On failure the DT1 is
dropped (`dt1.c:587-589`, `dt1_reset_failed_load` at `dt1.c:225-231`).

Constants: `DT1_FIXED_HEADER_SIZE 276`, `DT1_BLOCK_HEADER_SIZE 96` (`dt1.c:15-16`).
The sub-tile header size 20 is hard-coded (`dt1.c:239`).

Guide's "276 versus 272" explanation (guide#276-versus-272) AGREES: the editor reads 272 for
the pointer and validates count against 96-byte blocks.

---

## 2. Block header (96 bytes, array at `bh_start`, one per tile)

Read in `dt1_bh_update` (`dt1.c:168-201`). Editor struct `BLOCK_S` (`src/structs.h:1156-1181`).

| Offset | Size | Field | Editor type | Read line | Use in editor | Verdict |
|---|---|---|---|---|---|---|
| +0 | 4 | direction | long | `dt1.c:177` | stored, not used for decisions | AGREES |
| +4 | 2 | roof_y | **WORD = int16_t** (`types.h:15`, `structs.h:1159`) | `dt1.c:178` | roof vertical offset (`preview.c:1367`) | DIFFERS: guide says unsigned (MOVZX); editor reads signed. Same value unless roof_y >= 32768. |
| +6 | 1 | sound | UBYTE | `dt1.c:179` | stored | AGREES |
| +7 | 1 | animated | UBYTE | `dt1.c:180` | floor animated when == 0x01 (`misc.c:292`) | AGREES on test value |
| +8 | 4 | size_y | long | `dt1.c:181` | drives zero_line, height, y_add | AGREES (always <= 0 in corpus) |
| +12 | 4 | size_x | long | `dt1.c:182` | sprite width (160 for orientations 10/11, `dt1.c:380-381`) | AGREES |
| +16 | 4 | reserved | - | skipped (`dt1.c:183`) | none | AGREES |
| +20 | 4 | orientation | long | `dt1.c:184` | class, see section 4 | AGREES |
| +24 | 4 | main_index | long | `dt1.c:185` | key | AGREES |
| +28 | 4 | sub_index | long | `dt1.c:186` | key | AGREES |
| +32 | 4 | rarity | long | `dt1.c:187` | variant weight (section 8) | AGREES (guide#the-block-header: not a frame) |
| +36 | 4 | per-file constant | - | skipped (`dt1.c:188`) | none | AGREES |
| +40 | 25 | sub-tile flags, 5x5 | UBYTE[25] | `dt1.c:189-190` | collision/flags; stored reordered (section 3) | AGREES after reorder |
| +65 | 7 | reserved | - | skipped (`dt1.c:191`) | none | AGREES |
| +72 | 4 | tiles_ptr (absolute) | long | `dt1.c:192` | base of sub-tile headers | AGREES |
| +76 | 4 | tiles_length | long | `dt1.c:193` | not used by the editor (sub-tile walk uses count) | AGREES on value |
| +80 | 4 | tiles_number | long | `dt1.c:194` | loop count (`dt1.c:452`) | AGREES |
| +84 | 12 | three runtime slots | - | skipped (`dt1.c:195`) | none. Guide: 1378 blocks carry a leaked pointer at +88; NOT IN SOURCE (data). | AGREES on handling |

---

## 3. Sub-tile flags (block +40)

Reorder table `idxtable[25]` (`dt1.c:172`), applied in `dt1.c:189-190`:

    stored index k = idxtable[t] for file byte t (t = 0..24, file offset +40+t)
    idxtable = {20,21,22,23,24, 15,16,17,18,19, 10,11,12,13,14, 5,6,7,8,9, 0,1,2,3,4}

Equivalent: `k = y_grid*5 + x_grid` where `y_grid = 4 - t/5` and `x_grid = t%5`.
Guide's grid mapping (guide#the-flags-and-how-the-diagram-was-decoded) AGREES.

Bits: the editor does not name any bit. Guide's "bits 3, 6, 7 never set" is a data claim
(NOT IN SOURCE).

---

## 4. Sub-tile header (20 bytes, array at `tiles_ptr`)

Read in `dt1_fill_subt` (`dt1.c:235-249`). Index base: `tiles_ptr + 20*s` (`dt1.c:239`).

| Offset | Size | Field | Editor type | Read line | Verdict |
|---|---|---|---|---|---|
| +0 | 2 | x_pos | WORD (int16) | `dt1.c:240` | AGREES (guide says unsigned pixel column; values 0..128 so no difference) |
| +2 | 2 | y_pos | WORD (int16) | `dt1.c:241` | AGREES (signed, needed for negatives) |
| +4 | 2 | reserved | - | skipped (`dt1.c:242`) | AGREES |
| +6 | 1 | x_grid | int | `dt1.c:243` | stored, not used for placement | AGREES |
| +7 | 1 | y_grid | int | `dt1.c:244` | stored | AGREES |
| +8 | 2 | format | WORD | `dt1.c:245` | 0x0001 = isometric; anything else = RLE (`dt1.c:465-476`) | AGREES |
| +10 | 2 | length | **long (4 bytes)** | `dt1.c:246` | pixel bytes | DIFFERS in width: guide says uint16. Editor reads int32; bytes +12..+13 are zero in corpus, so values match. |
| +12 | 2 | reserved | - | skipped (`dt1.c:247`) | AGREES |
| +14 | 2 | reserved | - | skipped | AGREES |
| +16 | 4 | data_offset | long | `dt1.c:248` | relative to `tiles_ptr` (`dt1.c:460`) | AGREES |

Pixel data pointer: `buffer + tiles_ptr + data_offset` (`dt1.c:460`). Sub-tile headers are not
checked for overrun against `tiles_length`.

---

## 5. Sub-tile pixel decoders

### Isometric (format 0x0001)

`draw_sub_tile_isometric` (`src/core/dt1_draw.c:21-46`); index-buffer twin
`decode_sub_tile_isometric` (`src/core/dt1_decode.c:21-47`).

    xjump = {14,12,10, 8, 6, 4, 2, 0, 2, 4, 6, 8,10,12,14}
    nbpix = { 4, 8,12,16,20,24,28,32,28,24,20,16,12, 8, 4}   (sum 256)

Row `y` (0..14) starts at `x = xjump[y]` and writes `nbpix[y]` literal indices. Length must be
exactly 256, else the sub-tile is skipped (`dt1_draw.c:29`, `dt1_decode.c:30`).

Verdict: AGREES with guide#isometric-format-0x0001 (same 15 numbers, same order, `dt1_draw.c:26-27`).

### RLE (every other format)

`draw_sub_tile_normal` (`dt1_draw.c:48-79`); twin `decode_sub_tile_normal`
(`dt1_decode.c:51-83`).

    loop while length > 0:
      b1 = byte; b2 = next byte; advance 2
      if (b1 || b2): x += b1; read b2 literal indices -> pixels at x..; x advances
      else (0,0):    x = 0; y += 1            (end of row)

Verdict: AGREES with guide#rle-with-transparency. Note: the editor does not special-case a
lone 0x7F byte (guide says the encoder emits it for long transparent runs). Reader would consume
the following byte as b2. Guide says this is consistent by construction; not verified here.

Index 0 handling: index buffers are zero-filled (`dt1_all_zoom_make` uses `calloc`, `dt1.c:438`),
and palette index 0 has alpha 0 (`src/core/palette.c:42`). Literal index 0 pixels are therefore
transparent in the rendered bitmap.

Decoders stop at `length` and do not bounds-check the literal run against it (`dt1_decode.c:69-75`).
Malformed data can read past the sub-tile.

---

## 6. Block geometry (where a sub-tile is drawn)

Per block, `dt1_all_zoom_make` (`src/core/dt1.c:329-551`):

    w = size_x; h = -size_y
    if orientation in {10, 11}: w = 160                    (dt1.c:380-381)
    default y_add = 96                                     (dt1.c:387)
    if orientation in {0, 15} (floor, roof) and size_y != 0:
        size_y = -80; h = 80; y_add = 0                    (dt1.c:388-396)
    else if orientation < 15 and size_y != 0:             (upper wall, shadow, special)
        size_y += 32; h -= 32; y_add = h                   (dt1.c:397-405)
    if w == 0 or h == 0: skip block                        (dt1.c:408-417)
    per sub-tile: draw at (x0, y0) = (x_pos, y_add + y_pos)  (dt1.c:458-459)

| Class | Orientations | size_y after load | y_add | Guide claim | Verdict |
|---|---|---|---|---|---|
| floor | 0 | -80 | 0 | ZERO top-left, y 0..64 | AGREES |
| roof | 15 | -80 | 0 | same as floor | AGREES |
| upper wall / shadow / special (10, 11) / other < 15 | 1..14 (not 15), 13 shadow, 10/11 special | size_y + 32 | h (= -size_y - 32) | ZERO bottom, y negative | AGREES |
| lower wall | 16..19 (and any > 15) | unchanged | 96 | ZERO 96 px down, y down to -96 | AGREES |

Guide's loader sketch (guide#coordinate-systems) says floor/roof set size_y = -80 unconditionally.
Editor does so only when size_y != 0 (`dt1.c:390`, `399`). Edge case; same on corpus.

Class assignment for the block table (`src/misc.c:287-337`), used for lookup and drawing:

| Orientation | Type | zero_line (`misc.c`) | Line |
|---|---|---|---|
| 0 | BT_STATIC or BT_ANIMATED (animated == 1) | 0 | 288-302 |
| 13 | BT_SHADOW | -size_y | 303-308 |
| 10, 11 | BT_SPECIAL | -size_y | 312-317 |
| 15 | BT_ROOF (roof_y copied) | 0 | 318-324 |
| 1..14 (not 10, 11, 13, 15) | BT_WALL_UP | -size_y | 325-330 |
| 16..19 (> 15) | BT_WALL_DOWN | 96 (hard-coded) | 331-336 |

Verdict: AGREES with guide#orientation (groups); the per-value meanings (1..7, 12) are guide
claims supported by Siramy figures and data, not by this source. Editor does not name them.

---

## 7. Orientation semantics used by the renderer

Wall split (`src/render/preview.c:1068-1106`):
- Lower pass (`upper == FALSE`) draws orientation > 15 (`preview.c:1105`).
- Upper pass (`upper == TRUE`) draws orientation < 15 (`preview.c:1103`).
- Orientation 15 is roof and drawn only by `wpreview_draw_r` (`preview.c:1311-1424`), which
  accepts any cell whose block is BT_ROOF (`preview.c:1352`).
- Orientation 10/11 are drawn by pass 5 (`preview.c:1113-1119`, `2868-2892`).
- Roof offset: `y1 -= roof_y * height_mul / height_div` (`preview.c:1367`).

Draw pass order: `docs/specs/ds1.md` section 13.

---

## 8. Tile key and variant choice

### Key

    key = (orientation, main_index, sub_index)

Cell-side derivation is in `docs/specs/ds1.md` section 7. Block-side: `BLOCK_S` fields at +20/+24/+28.

Block table construction (`src/misc.c:224-261`): one entry per block of every DT1 that is loaded
for the map, plus sentinel at index 0. Each entry records `dt1_idx_for_ds1` (slot 0..32 in the
DS1's DT1 list) and `dt1_idx` (global DT1 index) (`misc.c:280-281`).

### Sort and group

1. `qsort` by `misc_qsort_helper_block_table_1` (`misc.c:33-76`): orientation, main, sub, then
   `dt1_idx_for_ds1` where 0 maps to `DT1_MAX` (so slot 0 sorts last), then rarity ascending,
   then block index (`misc.c:55-63`, `65-74`).
2. `misc_check_tiles_conflicts` (`misc.c:136-220`) walks the sorted table in groups of equal
   (orientation, main, sub) (`misc.c:166-172`).
3. Second `qsort` by `misc_qsort_helper_block_table_2` (`misc.c:80-131`, display order).
   Order at `misc.c:365-373`.

### Choice inside a group (`misc.c:147-212`)

| Case | Game (`used_by_game`) | Editor (`used_by_editor`) | Lines |
|---|---|---|---|
| sum of rarity == 0 | last block of the first DT1 in the group (`last_block` where `dt1_idx == first dt1_idx`) | same block | `misc.c:196-199` |
| sum of rarity > 0 | every block with rarity > 0 | one block: the first with the highest rarity (see quirk) | `misc.c:201-211` |

Guide claim (guide#rarity-and-the-tile-the-game-actually-draws): "the editor shows the first
block with the highest rarity". The source does NOT always do that. See the quirk below.

### QUIRK: group head is excluded from the maximum (DIFFERS)

`misc.c:156-163` sets `max_rarity = -1` and seeds `first_block` from the group head, but the max
comparison (`misc.c:183-187`) only runs on blocks after the head. The head's own rarity is never
compared.

| Group (rarity in sorted order) | Editor picks | Expected by "first highest" |
|---|---|---|
| A=5, B=3 | B | A |
| A=5, B=5 | B | A |
| A=5, B=0 | A | A |
| A=0, B=2, C=5 | C | C |

Impact: any group whose first block has the highest non-zero rarity and at least one later
block with non-zero rarity is shown with the wrong tile in the editor's picker and in
`used_by_editor`. The game path (`used_by_game`) is unaffected: it sets every non-zero block
(`misc.c:207-211`).

Also: the loop reads `bt[i]` before the bound check (`misc.c:166-172`, check at 171). When
`i == bt_max` it reads one element past the table. Guard before reading.

### Slot 0 (ds1edit.dt1)

Slot 0 of every DS1 is the editor's own `ds1edit.dt1` (`misc.c:811`, `txtread.c:821-826`,
loaded from the data directory by `dt1_add_special`, `dt1.c:677-730`). Its blocks join the same
key table and sort last. Its orientation 10/11 blocks are drawn as an overlay on the game's
special tiles (`dt1.c:484-528`, editor-only).

Guide's rule (guide#rarity): "rarity sum 0: last block of the first DT1 in the level's
tileset list". AGREES, with the caveat that "first" means lowest DS1 slot (File 1 first),
and slot 0 (editor DT1) is treated as last.

---

## 9. Which DT1s a DS1 loads

Full resolution is in `docs/specs/excel.md`. Summary here:

1. `misc_open_1_ds1` (`misc.c:712-788`): `ds1_read` -> `read_lvlprest_txt(def)` ->
   `read_lvltypes_txt(type)` -> `misc_make_block_table` -> `ds1_make_prop_2_block`.
2. `read_lvlprest_txt` sets `dt1_mask[0] = TRUE` and `dt1_mask[b] = bit (b-1) of Dt1Mask` for
   b = 1..32 (`txtread.c:987-993`, and again for the Def path at `txtread.c:1051-1057`).
3. `read_lvltypes_txt` loads slot 0 = `ds1edit.dt1`; then for f = 1..32 reads `File f` and loads
   it if `dt1_mask[f]` (`txtread.c:817-855`). Only the literal "0" is skipped (`txtread.c:839-841`).
   A blank cell in a string column is NOT skipped: it becomes `glb_tiles_path` alone and goes to
   `dt1_add` (`txtread.c:831-848`). See open point 11. Paths are converted from `/` to `\`
   (`txtread.c:838`) and prefixed with `glb_tiles_path` (`txtread.c:842-843`).
   A column whose type is CT_NULL (every cell blank) leaves `tmp` unchanged from the previous
   iteration (`txtread.c:831-838`), so the previous DT1 can be loaded again.
4. Unlisted maps (Def not found and no name match): all 33 masks TRUE (`txtread.c:1017-1018`).

Disagreement inside the reference on File 1 gating:

| Reader | File 1 gated by Dt1Mask bit 0? | Source |
|---|---|---|
| read_lvlprest_txt / read_lvltypes_txt | yes | `txtread.c:992,844` |
| asset_export tile list | yes | `asset_export.c:1114-1120` |
| props_panel (UI listing and sync) | no: File 1 always on, bit (fi-1) for fi > 1 | `props_panel.c:157-169` |

Guide claim (guide, excel): Dt1Mask bit mapping. The loader path agrees with bit (N-1) for File N.
The props panel differs for File 1. Treat the loader (`txtread.c`) as authority.

Default mask when no LvlPrest row: asset_export uses 0x7FFFFFFF (`asset_export.c:1088-1100`);
the loader uses all-TRUE (`txtread.c:1017-1018`).

---

## 10. Loading and memory

- `dt1_add` (`dt1.c:619-673`): if the same path is loaded, bump `ds1_usage`; else read via
  `misc_load_mpq_file` (mod dir, then MPQ chain), parse, and decode every block up front
  (`dt1.c:610` calls `dt1_all_zoom_make`). Guide's lazy LRU path (guide#how-the-game-loads-a-dt1)
  is not implemented. Verdict: AGREES (guide says the same about the editor).
- Only ZM_11 bitmaps are generated; other zooms are GPU-scaled (`dt1.c:530-531`).
- Two representations per block: a bitmap (`block_zoom[z]`) and a palette-index cache
  (`block_cache[z]`), rebuilt for a new palette by `dt1_rebuild_bitmaps_from_cache`
  (`dt1.c:735-768`, `rgba_cache.c:70-100`).
- Files are not decoded if `dt1_headers_are_valid` fails (`dt1.c:576-590`).
- Version 4.1 files (six in the guide) fail the 7/6 signature check and are skipped
  (`dt1.c:211`). Guide#six-files-nobodys-tool-can-read: NOT IN SOURCE for their structure.

---

## 11. Cross-check: guide claims vs source

| Guide claim (section) | Source | Verdict |
|---|---|---|
| Key is (orientation, main, sub) (#the-catalogue-not-the-map) | `ds1.c:137-145`; `misc.c:166-171` | AGREES |
| Hash `(sub*2 - orientation + main) & 0x7F` (game) | none in editor | NOT IN SOURCE |
| 276 vs 272, block array at 276 (#276-versus-272) | `dt1.c:15,574` | AGREES |
| Header: version 7 at +0 (#the-fixed-header) | `dt1.c:211,571` | AGREES |
| Header: flags at +4 read as 6 and treated as a version minor (guide says it is a flags field) | `dt1.c:572,211` | DIFFERS: editor requires 6 (guide says game does not check) |
| Filename buffer zero on disk | not read | AGREES |
| Block header field offsets and sizes (#the-block-header) | `dt1.c:177-194` | AGREES, except roof_y signedness (DIFFERS) |
| rarity at +32, not a frame (#32-is-rarity) | `dt1.c:187`; `misc.c:61,65-74` | AGREES |
| +84 three runtime pointers (#84-is-three-pointers) | skipped `dt1.c:195` | AGREES on handling; leaked-pointer count NOT IN SOURCE |
| direction determined by orientation (#direction-is-a-function-of-orientation) | not used for decisions in editor | AGREES (editor does not branch on it) |
| Sub-tile header 20 bytes (#the-sub-tile-header) | `dt1.c:239` | AGREES |
| length is uint16 (#the-sub-tile-header) | `dt1.c:246` reads int32 | DIFFERS in width; same values |
| data_offset relative to tiles_ptr (#the-sub-tile-header) | `dt1.c:460` | AGREES |
| Two pixel encodings; format 0x0001 isometric, else RLE (#two-pixel-encodings) | `dt1.c:465-476`; `dt1_draw.c:29` | AGREES |
| Isometric 15 rows, widths sum 256 (#isometric-format-0x0001) | `dt1_draw.c:26-29`; `dt1_decode.c:27-31` | AGREES |
| RLE decoder (#rle-with-transparency) | `dt1_draw.c:55-78` | AGREES |
| 0x7F lone byte encoder rule | not in repo | NOT IN SOURCE |
| Three coordinate systems, y_add by class (#three-coordinate-systems) | `dt1.c:387-405` | AGREES |
| Lower wall base offset 96 (`size_y` -96 floor) | `dt1.c:387`; `misc.c:335` | AGREES |
| Floor 5x5 grid positions (#the-5x5-grid) | editor places by x_pos/y_pos only; table not used | NOT IN SOURCE (table is game data; editor uses positions in the file) |
| Flag byte t -> grid (x=t%5, y=4-t/5) (#the-flags) | `dt1.c:172` | AGREES |
| Flag bits used (0x01,0x02,0x04,0x10,0x20) | no bit meanings in editor | NOT IN SOURCE |
| Orientation 15 = roof; roof_y unsigned MOVZX (#orientation) | roof draw `preview.c:1367`; roof_y signed `structs.h:1159` | AGREES on role; DIFFERS on signedness |
| Orientations 16..19 one group (#orientation) | `misc.c:331-336`; `preview.c:1105` | AGREES |
| Orientations 8/9 doors, 10/11 special, 13 shadow, 14 tree (#orientation) | `misc.c:303-336` (13, 10/11); 8/9/14 not distinguished | AGREES for 10/11/13; 8, 9, 14 NOT IN SOURCE as special |
| Floor orientation 0 implicit; shadow 13 implicit (ds1 side) | `ds1.c:217,251`; `misc.c:288,303` | AGREES |
| Special tile orientation 10/11 (#orientation) | `misc.c:312-317`; `dt1.c:380-381,486` | AGREES |
| Rarity-weighted choice, sum 0 -> last of first DT1 (#rarity) | `misc.c:196-199` | AGREES |
| Editor shows first block with highest rarity (#rarity) | `misc.c:156-187` | DIFFERS (group head excluded; section 8 quirk) |
| Game draws any non-zero rarity block (#rarity) | `misc.c:207-211` (game flag, not the draw) | AGREES on the flag; draw selection is game-side, NOT IN SOURCE |
| Six version-4.1 files unreadable (#six-files) | `dt1.c:211` rejects | AGREES (rejected) |
| Eager decode of every block (#how-the-game-loads) | `dt1.c:610` | AGREES |

---

## 12. Open points

1. Signed vs unsigned roof_y: editor reads int16 (`structs.h:1159`), guide says MOVZX. Only
   matters for roof_y >= 32768. Pick unsigned for the port unless a file needs negative values.
2. Header flags at +4: editor rejects anything but 6 (`dt1.c:211`). Guide says the game does not
   check. Loosen if mod tilesets carry other values.
3. Sub-tile length: editor reads 4 bytes (`dt1.c:246`). Read two bytes in the port (guide), and
   confirm bytes +12..+13 are zero where length must not spill.
4. Variant quirk (section 8): group head excluded from max. Decide whether the port reproduces
   the editor or the documented rule. Documented rule is the one the guide describes.
5. Read past block-table end in `misc_check_tiles_conflicts` (`misc.c:166-171`).
6. File 1 gating differs between `props_panel.c:168` and `txtread.c:844`. Use txtread.
7. Lone 0x7F in RLE: not handled specially here; verify on a file that uses it (guide says
   "lone 0x7F ... parsed as an ordinary pair").
8. Sub-tile reads are not checked against `tiles_length` or the file size (`dt1.c:452-478`).
9. `dt1_all_zoom_make` keeps `size_y` unchanged for floor/roof when it is 0 (`dt1.c:390`). Guide
   sets -80 unconditionally. Confirm no zero-height floor blocks exist in the corpus.
10. Sentinel behaviour: block 0 is a sentinel with orientation -1 that sorts first (`misc.c:252-261`,
    `365`). The port must keep index 0 reserved, because `bt_idx == 0` means "no tile" in the
    cell lookup (`ds1.c:132`).
11. Blank File cells: `read_lvltypes_txt` only skips the literal "0" (`txtread.c:840`). A blank cell in
    a string-typed File column reaches `dt1_add` with a bare tiles prefix (`txtread.c:831-848`) and
    fails. Confirm LvlTypes.txt uses "0" for empty slots, or make the port skip blanks.
12. CT_NULL File column reuses the previous `tmp` (`txtread.c:831-838`). Port should treat an all-blank
    column as empty.

---

## 13. Writing (P11, `Dt1::to_bytes`, `Block::{isometric, rle, with_pixels}`)

Rules the writer follows; each was checked against every readable file of both corpora
(619 version-7.6 files, 41,080 tiles, 895,820 sub-blocks), which all write back byte for byte.

- **One canonical layout.** Every shipped file has the tile header array at 276, then each
  tile's region in tile order: its 20-byte sub-block headers, then their data in block order,
  data offsets relative to the region start, `tiles_length` (+76) = region length, no gaps,
  no sharing, nothing after the last region. The writer always produces this layout, so a
  file with shared or gapped data (legal to read, never shipped) parses back to the same
  model but not the same bytes.
- **Tiles without blocks** point at the current end of the file (the start of the next
  tile's region) with length 0, as the game's own files and the reference writer do.
- **Kept tile-header bytes.** +36 (4 bytes) is non-zero in most tiles and varies between
  tiles; +88..+92 holds a leaked pointer in some expansion tiles. Both are kept in
  `Tile::preserved` together with +16 and +65..+72 (zero everywhere). Added tiles get
  zeros (the reference's blocker tile uses `ff 00 ff 00` at +36; nothing reads it).
- **Written as zero:** the 260-byte name buffer, sub-block header +4..+6 and +12..+16 (zero in
  every shipped file). The length at +10 is written as a `u16`; the reader takes a `u16`, so
  a block over 65535 bytes is `NotRepresentable` rather than silently truncated.
- **Version.** Only major 7 is written; the `flags` dword is stored as-is. The 4.1 files
  (six per install) do not parse and so cannot be written.
- **Format word.** `raw_format` is what is stored; a model whose `format` disagrees with it
  (`Isometric` iff `raw_format == 1`) is rejected instead of changing meaning on reload.
- **Isometric encoding** stores all 256 diamond positions; a position the image leaves
  transparent (or does not cover) becomes index 0, which draws nothing. Pixels outside the
  diamond are dropped.
- **RLE encoding** (the shipped convention, reproduced byte for byte for every corpus
  block): per row, `(skip, count, literals)` for each run of opaque pixels with `skip`
  counted from the end of the previous run, trailing transparent pixels not encoded, then
  `(0, 0)` - also after the last row. Rows are terminated down to the last opaque one and at
  least `min_rows` (walls `0x1001`: 32, `0x2005` floor pieces: 15); `with_pixels` keeps the
  block's previous row count. Never in the corpus but accepted by the decoder: runs over 255
  continue with `skip = 0`, skips over 255 use `(255, 0)` pairs.
- **Index 0 is not storable as a colour.** Decoders give literal 0 alpha 0, so an RLE
  literal 0 is dropped on re-encode (shipped files contain none: re-encoding is exact) and
  an opaque index-0 pixel in an input image decodes as transparent.
- Opaque pixels left of or above a block's `(x, y)` cannot be expressed in RLE (skips are
  unsigned) and are `NotRepresentable`; the caller picks a block origin that covers them.
- The writer does not recompute derived header fields (`width`, `height`, `roof_height`):
  a caller that rebuilds blocks updates them (the reference sets `width = max(x + 32)`).

---

## 14. Conventions for new tiles (measured)

Measured over every readable file of both corpora (619 files, 41,080 tiles; the report is
`measure_shipped_conventions` in `crates/capy-mod/tests/tiles_create_corpus.rs`, run with
`CAPY_FULL_CORPUS=1`). `capy_mod::tiles_create` builds new tiles by these rules; rebuilding every
corpus tile from its own render reproduces 100% of the pixels and 100% of the sub-blocks byte for
byte (99.56% of whole tiles, 592 of 619 files; the misses are listed below).

Coordinates are block `x`/`y` (anchor space, section 6). "Art height" is the pixel height of the
drawable area.

| Class | Orientations | Blocks | Drawable area | Header |
|---|---|---|---|---|
| floor | 0 | 5x5 grid of 32x15 diamonds at `x = 64 + 16(gx - gy)`, `y = 8(gx + gy)`, `grid_x/grid_y = gx/gy` | the 160x79 diamond at (0,0) | height -128 (all 23,998), width 160, roof 0 |
| roof | 15 | as floor | as floor | height -128, width 160, `roof_height` 80 most often (763 of 1,735; then 240, 160, 190, 0, 120) |
| upper wall | 1-9, 12, 14 | 32x32 cells, `x` in 0, 32, ..., 128, `y` multiples of 32 from `height + 32` to -32, grid 0/0 | x 0..160, y `-art..0` | `height = -(art + 32)` = top block y - 32 (every tile with blocks), `width = max x + 32` (every tile) |
| shadow | 13 | as upper wall | as upper wall | as upper wall; heights -128 (1,140), -96 (607), -64 (103) |
| special | 10, 11 | as upper wall | as upper wall | as upper wall (width is still `max x + 32`; the editor draws them 160 wide) |
| lower wall | 16-19 | as upper wall, `y` multiples of 32 from -96 to `art - 128` | x 0..160, y `-96..art - 96` | `height = -art`, `width = max x + 32` |
| corner halves | 4 / 3 | orientation 4 uses x 0..64 (pixels x 0..80), orientation 3 x 64..128 (pixels 80..160) | one picture split at x = 80 | same main/sub (719 of 723 orientation 3 tiles have their orientation 4 half); width 96 / 160 |

Where walls stand (`measure_wall_planes` in the same test file; anchor space, where the cell's
floor diamond is top (80,-80), right (160,-40), bottom (80,0), left (0,-40) for every wall class,
section 6): the lowest opaque pixel per column of shipped left walls (1, 5, 8, the half 4) runs along
the north-west edge, about one sub-tile (8 px) inside it (median -39 at x 4, -50 at x 68); right walls
(2, 6, 9, the half 3) along the north-east edge the same way. The median lowest pixel of a whole
upper wall is 29 px above the bottom corner (cell y 51 of 80); only 5% reach it. A picture filling the
whole template therefore covers the whole cell; `wall_foot` names the edge, and the New tile preview
and `tile_template` show it. Lower walls (16-19) start at the cell's top edges and hang down (median
lowest pixel 615 to 650 px below the anchor for 16, 17 and 19, 250 for 18). Rebuilding every corpus tile through the "As is"
picture path gives the same pixels and the same map placement (`capy_render::TileGeometry`) as the
shipped tile (`crates/capy-cli/tests/tile_create_placement.rs`; 41,080 tiles, the only placement
differences are the header-only rebuild misses listed below).

Sub-block formats and order:

- **Floor and roof cells**: a cell whose 256 diamond positions are all opaque is isometric (format
  1; 556,423 blocks, none has a 0 byte); a partly covered cell is RLE `0x2005` with exactly 15
  terminated rows (47,494 blocks, never a pixel outside the cell's diamond); an empty cell has no
  block. Order: `gy` 4 down to 0, within it `gx` 4 down to 0 (the reverse of row-major).
- **Wall, shadow and special cells**: RLE `0x1001`, always 32 terminated rows (252,336 blocks),
  grid 0/0, empty cells have no block (no shipped wall block is empty). Order: upper walls top row
  first, lower walls bottom row first; within a row right to left (x 128 down to 0).
- Only formats 1, `0x1001` and `0x2005` occur.

Other header fields:

| Field | Convention |
|---|---|
| direction | 0: 3, 1: 1, 2: 2, 3: 3, 4: 3, 5: 1, 6: 2, 7: 4, 8: 1, 9: 2, 10: 1, 11: 2, 12-14: 3, 15: 5, 16-19: orientation - 10. Every tile but 42 floors (direction 0, in `whimsyfence.dt1`). |
| rarity | 0 is the most common for every orientation (floors 17,967 of 23,998; 1 is next) |
| sound, animated | 0 most common; animated floors use 1 or 4 (783) |
| +36 | `ff 00 ff 00` most common (11,709 floors, most walls); varies |

Sub-tile flags (`y * 5 + x`, row y = 0 first), most common pattern per orientation:

| Orientation | Most common | Default for new tiles |
|---|---|---|
| 0 floor | none (13,221 of 23,998), then 0x01 everywhere (3,012) | none |
| 1, 5 | 0x07 on column x = 0 (962 of 3,601 for 1) | 0x07 on column 0 |
| 2, 6 | 0x07 on row y = 0 (952 of 3,707 for 2) | 0x07 on row 0 |
| 3, 4 | 0x07 on row 0 and column 0 (212 / 220) | the same |
| 7 | none (99), 0x01 at (0,0) (82), 0x07 at (0,0) (75) | 0x07 at (0,0) |
| 8 | 0x07 at (0,0) and (0,4) (34 of 224) | the same |
| 9 | 0x07 at (0,0) and (4,0) (41 of 163) | the same |
| 10 | none (225, the 13-block marker diamonds), 0x07 on column 0 (170) | 0x07 on column 0 |
| 11 | 0x07 on row 0 (198 of 314) | 0x07 on row 0 |
| 12, 14 | none, then scattered patterns | none |
| 13, 15, 16-19 | none (1,723 of 1,874; 1,727 of 1,735; nearly all lower walls) | none |

0x07 = block walking + sight barrier + jump/flight barrier (`tiles_flags`).

Rebuild misses (182 tiles; pixels and blocks are identical in every case, only header fields
differ):

- 42 floors in `mxl/zone/whimsy/whimsyfence.dt1` store direction 0 instead of 3.
- 91 lower walls (`act2/arcane/sanctuary.dt1`, `inv_wall.dt1`, `worldstone.dt1`, `rockcliff.dt1`,
  `cliff.dt1`) have blocks reaching 32 to 128 pixels below their declared height (396 lower walls
  follow `height = -(art)` exactly). The inferred shape grows to hold every block, so the rebuilt
  height is larger.
- 49 wall and shadow tiles without blocks store height 0 and width 0; a rebuilt empty wall keeps
  the 32-pixel minimum art height (stored -64).
