# Palettes and colour: editor spec (cross-checked)

Status: cross-check of `reference/d2-ds1-edit/docs/guides/palettes-and-colour.md` against the
reference C source, plus one byte-level probe of a cached `Pal.pl2`. Written by spec-maps,
read-only pass.

Conventions
- `src/...` paths are relative to `reference/d2-ds1-edit/`.
- `guide#slug` cites `reference/d2-ds1-edit/docs/guides/palettes-and-colour.md` by heading.
- Verdicts: AGREES, DIFFERS (explained), NOT IN SOURCE.
- Probe: `testdata/private/oracle/assets/editor/pal0.bin` (Act 1) and `pal4.bin` (Act 5) are
  the editor's own cache of `Pl2` (`misc.c:598-613`). They live in the MAIN checkout, not this
  worktree, and were read-only. Size 443,175 bytes.

Primary code: `src/core/palette.c`, `src/core/palette.h`, `src/misc.c` (gamma 430-477, load
and cache 555-613, colormaps 485-551, pl2 correction 1301-1328), `src/main.c` (palette load
869-918), `src/render/preview.c` (blend 291-379, objects 2131-2155, palette choice 2597-2617),
`src/ui/compat.h` (blend 80-130), `src/structs.h` (COF offsets 92-97, GC enum 128-137).

---

## 1. Files the editor reads

| File | Location | Read by | Used for | Verdict vs guide |
|---|---|---|---|---|
| `Data\Global\Palette\ActN\Pal.pl2`, N = 1..5 | MPQ chain (mod dir first) | `ds1edit_load_palettes` `main.c:879-897` | base palette + transform buffer | AGREES (guide: `Pal.PL2` in the same folder) |
| `data/pal%d.bin` (index 0..4) | editor data dir | `misc_load_pal_from_disk` / `misc_save_pal_on_disk` `misc.c:555-613` | cache of the raw Pl2 bytes | editor-only |
| `data/cmap%d_%d.bin` | editor data dir | `misc_make_cmaps` `misc.c:485-551` | cache of editor-built colormaps | editor-only |
| `Data\Global\Palette\ACTN\pal.dat` | - | NOT read | - | NOT IN SOURCE (guide#the-dat-palette) |

Act 5 palette missing is a warning, not fatal (`main.c:905-908`). Missing Act 1..4 is fatal
(`main.c:905-906`). A missing palette leaves `d2_pal` NULL and `palette_d2_to_rgba` fills a grey
ramp (`palette.c:12-22`).

Cache rule: a palette is taken from disk if `pal%d.bin` exists, else from the MPQ, then written
to disk (`main.c:883-912`). The cache holds the raw Pl2 bytes, before correction (save at
`main.c:912`, correction at `main.c:916`). Gamma is NOT part of the cache.

---

## 2. Pl2 layout (as used by the editor)

Total: 443,175 bytes = 0x6C327 (`pal0.bin` and `pal4.bin` probe: 443,175). Guide#the-buffer-table-by-table
says the same size and the same overall structure (AGREES).

| Offset (bytes) | Size | Content | Editor use | Source |
|---|---|---|---|---|
| 0x0000 | 1024 | base palette: 256 entries x 4 bytes | `palette_d2_to_rgba` reads R=[4i], G=[4i+1], B=[4i+2], pad=[4i+3] | `palette.c:25-43`; `palette.h:33` |
| 0x0400 | 8192 | 32 tables x 256 bytes (darkness per guide) | `CM_SHADOW` colormap: row `c/8` for source index c | `palette.c:185-207` |
| 0x2400 .. 0x34FF | 4096 + 256 | highlight 16 x 256 (0x2400), saturation (0x3400) per guide | not referenced by the editor | NOT IN SOURCE |
| 0x3500, 0x13500, 0x23500, 0x33500, 0x43500 | 65536 each | 256x256 index tables | see section 5 | `structs.h:92-97` |
| 0x5B500 | 65536 | 256x256 table (guide: screen blend) | named COF_ALPHABRIGHT, see section 5 | `structs.h:97` |

Colormap offsets are stored as ROW indices x 256 (`misc.c:1310,1313,1316,1319,1322,1325`).
Row index times 256 gives the byte offset:

| Constant | Row | Byte offset | Guide offset | Guide label | Editor name |
|---|---|---|---|---|---|
| `COF_25TRANS` | 53 | 13,568 (0x3500) | 0x03500 | "~75% (0xBF)" | 25% |
| `COF_50TRANS` | 309 | 79,104 (0x13500) | 0x13500 | "~50% (0x7F)" | 50% |
| `COF_75TRANS` | 565 | 144,640 (0x23500) | 0x23500 | "~25% (0x3F)" | 75% |
| `COF_ALPHA` | 821 | 210,176 (0x33500) | 0x33500 | additive | alpha |
| `COF_LUMINANCE` | 1077 | 275,712 (0x43500) | 0x43500 | multiply | luminance |
| `COF_ALPHABRIGHT` | 1461 | 374,016 (0x5B500) | 0x5B500 | screen blend | alpha-bright |

Offsets AGREE with the guide. The names and percentages DIFFER (section 5).

Correction pass `misc_pl2_correct` (`misc.c:1301-1328`): for each of the six tables above, row 0
is overwritten with identity (`data[0][c] = c`). Index 0 maps to itself in those tables. Runs at
load, after the cache write (`main.c:916`). Idempotent on reload.

Guide#loading-the-palette-writes-a-second-copy: "pl2 bytes 0..3 are blue, green, red, pad".
See section 3 (DIFFERS, open).

---

## 3. Base palette byte order (DIFFERS, unresolved)

| Source | Index 150, first 4 bytes | Reading |
|---|---|---|
| Editor `palette.c:27-29`, `palette.h:33` | byte0 = R | RGB order, pad last |
| Guide `palettes-and-colour.md` (#loading-the-palette) | `54 78 BC 00` | B,G,R,pad (guide says index 150 = RGB 188,120,84 tan) |
| Probe `pal0.bin` (Act 1) offset 600 | `84 120 188 0` | matches guide's bytes |
| Probe `pal4.bin` (Act 5) offset 600 | `84 120 188 0` | same bytes at index 150 in the Act 5 cache |
| Golden `test/golden/act2_palace.png` (Act 2 scene) | warm stone, rugs, brown | consistent with warm (tan) colours |

Consequence: if the guide is right, `palette_d2_to_rgba` as written yields (84,120,188) for index
150, a blue-grey, not tan. The golden image looks warm, which suggests the rendered palette is
correct and the source reading in `palette.c` is not what the rendering path uses, or the golden
was produced by another path. NOT RESOLVED. Before porting, print `vga_pal` entry 150 from the
reference build, or render one known tile.

Editor's gamma is applied per channel to whatever channel order is read (`palette.c:32-37`), so
the swap, if any, is a plain R/B exchange.

---

## 4. Gamma correction

Table (`misc.c:447-467`):

    inv = 1.0 / gamma_value(gt)
    table[gt][0] = 0
    table[gt][i] = clamp(round(255 * (i/255)^inv), 0, 255)   for i = 1..255

Gamma values (`misc.c:430-438`), GC enumeration `structs.h:128-137`, names `main.c:92`:
- gt 0..20: 0.60 + 0.02*gt (0.60 .. 1.00, 21 values)
- gt 21..40: 1.10 + 0.10*(gt-21) (1.10 .. 3.00, 20 values)
- GC_MAX = 41 tables.

Default: `gamma_correction = 1.30` (`config.c:115` sample text, `config.c:323` default), i.e. GC_130
(`main.c:92`, `cli.c:979`).

Application:
- `misc_pal_d2_2_vga(i)` builds `vga_pal[i]` from `d2_pal[i]` with `gamma_table[cur_gamma]`
  (`misc.c:22-29`, `palette.c:32-37`). Only the base palette RGB is gamma-corrected.
- Shadow and alpha colormaps are not gamma-corrected (they are index tables).
- Changing gamma: `misc_update_pal_with_gamma` (`misc.c:471-477`), then the DT1 bitmaps are
  rebuilt from cache (`dt1.c:735-768`).

Comment `misc.c:441-446`: the table used to be read from `data/gamma.dat` (41 x 256 bytes), verified
byte-identical to this formula. Guide does not cover gamma (NOT IN GUIDE).

---

## 5. Blend and transform tables: names, offsets, and probe

Editor code uses the COF-named rows ONLY to force row 0 to identity (section 2). It does not draw
with them. Object blending is done with GPU blend modes keyed on COF `trans_b` (section 6).

Probe (Act 1, `pal0.bin`), first read of index 150, bg/src interpretation as in guide verification
line 109 (`nearest((c_bg*inv + c_src*alpha)/255)`, flat offset = bg + 256*src):

| Row | Entry [src=0][bg=150] | Entry [src=150][bg=0] | Reading |
|---|---|---|---|
| 53 | 146 (0.75 x tan) | 244 (0.25 x tan) | bg weight 0.75, src weight 0.25 |
| 565 | 179 | 146 (0.75 x tan) | src weight 0.75 |
| 309 | 141 | 141 | 50/50 |
| 821 | 150 | 150 | additive/identity at these inputs (not discriminating) |
| 1077 | 0 | 0 | multiply with black gives 0 (expected) |

So rows 53 and 565 behave as expected for COF_25TRANS and COF_75TRANS if the name is read as
source-weight. The guide labels row 53 "~75% (0xBF)" because 0xBF is the background weight there.
Both readings agree on the bytes; they disagree on what the percentage refers to. Settle this by
using the editor's semantics (source weight) and checking one composited sprite.

Other tables (guide, NOT IN SOURCE in the editor): highlight (0x2400), hue-rotation family
(0x53500-0x5B000), tint table (0x6B727), palshift (2048 bytes per monster token), item transforms
(8 files). The editor applies palshift to sprite pixels in `cof.c:149-169` (table pointer from
`cof.c:71-77`; the file source is in `cof.c:216` and was not traced further).

---

## 6. Blend modes keyed on COF trans_b

Source of values: COF layer record bytes trans_a (q[3]) and trans_b (q[4]) (`cof.c:312-314`,
`compose_cof.c:87-88`, `structs.h:332-333`).

Gate for blending (`preview.c:2131` and `2151`, both paths):

    blend iff  trans_a != 0  AND  trans_b <= 6  AND  trans_b != 5
    else draw opaque

Mapping (`preview.c:291-323` unscaled, `preview.c:325-379` scaled; `compat.h:94-130` identical):

| trans_b | Mode | Implementation | Source line |
|---|---|---|---|
| 0 | 25% opacity (alpha 0.25) | tinted bitmap, alpha 0.25 | `preview.c:296` (scaled: `preview.c:344-346`) |
| 1 | 50% opacity | tinted, alpha 0.50 | `preview.c:299` |
| 2 | 75% opacity | tinted, alpha 0.75 | `preview.c:302` |
| 3 | additive | blender ONE, ONE | `preview.c:305` |
| 4 | multiply | blender DEST_COLOR, ZERO | `preview.c:310` |
| 5 | not drawn blended (gated out) | opaque draw | `preview.c:2131` |
| 6 | inverse-src-colour screen | blender ONE, INVERSE_SRC_COLOR | `preview.c:315` |
| other | gated out, opaque | - | `preview.c:2131` |

Note: `preview.c:320` (default branch) uses 50% if reached, but the gate prevents it for listed values.

Stale comment: `compat.h:80` says "3+=screen/luminance". The code uses 3 = additive, 4 = multiply,
6 = inverse screen. The comment is wrong for 3 and 4; the trans_b 0/1/2 description ("0=75%
transparent") matches the code (0 = 25% opacity).

Sprite shadow (not COF): alpha 0.5 (`preview.c:820-826`); global `a5_trans_alpha` default 0.5
(`globals.c:17`).

Guide's blend LUTs (alpha/additive/multiply tables) are NOT the mechanism for object blending in
this editor. Guide has no trans_b mapping, so there is nothing to compare (NOT IN GUIDE).

---

## 7. Editor-built colormaps

| Map | Builder | Algorithm | Lines |
|---|---|---|---|
| CM_SELECT (mouse highlight) | `palette_build_select_colormap` | index 168 -> white; index 255 -> `(x + 4y)/3` clamped; others 50% average, then nearest colour | `palette.c:102-143`; constants `palette.c:99-100` |
| CM_TRANS | `palette_build_trans_colormap` | 50% average of x and y, then nearest colour (`palette_find_closest`) | `palette.c:154-170` |
| CM_SHADOW | `palette_build_shadow_colormap` | copy from Pl2 at `1024 + 256*(c/8)`; zero-fill past end | `palette.c:185-207` |

Nearest colour: squared Euclidean in RGB, index 0 excluded (`palette.c:47-67`).
Caching per (act, map): `cmap<act>_<map>.bin` (`misc.c:485-551`).

Shadow row rule (`palette.c:197`): source index c reads row c/8, so 32 shadow rows serve 256 source
indices. The port must keep this, or shadows change colour.

---

## 8. Per-act palette selection

1. Act column of the LvlTypes row: `read_lvltypes_txt` returns it (`txtread.c:782-784`).
2. Override by tileset folder: if every non-empty File column (type STR) of that row is under one
   `ActN\` folder, the palette act is N, even when the Act column says otherwise
   (`txtread.c:786-804`; path test `txtread.c:682-697`; unanimity `txtread.c:706-735`).
   Source comment names LvlTypes Id 35 "Act 5 - Lava": tilesets are Act4 art, so the palette is Act 4.
   Data claim (Id 35 tileset folders) NOT verified here.
3. Editor resolves the index (`palette.c:70-85`):
   - if txt_act in 1..5, use it;
   - else if DS1 act in 1..5, use it;
   - else 1.
   Index = act - 1 (`palette.c:82-85`).
4. Render chooses `vga_pal[index]` per frame (`preview.c:2597-2607`); CLI `force_pal_num` overrides
   (`preview.c:2608-2617`). When the index changes, DT1 bitmaps are rebuilt (`dt1.c:735-768`).

Unlisted maps (no LvlTypes row): read_lvltypes_txt returns 0 (`txtread.c:865-869`), and the DS1 act
decides.

---

## 9. Index 0 transparency

- RGBA palette: index 0 alpha = 0, others 255 (`palette.c:20,42`).
- Nearest-colour search excludes index 0 (`palette.c:53`).
- Sprite drawing uses `pal_color(index)` = `colors[index & 0xFF]` (`compat.h:42-50`).
- DT1 index buffers are zero-filled; literal 0 pixels are transparent (`dt1.c:438`).
- Pl2 colormap row 0 is identity in the six COF tables (section 2), so a transparent source
  stays transparent through a blend.

The guide does not state an alpha rule for index 0 (NOT IN GUIDE). The editor's rule is
alpha 0 at `palette.c:42`.

---

## 10. Cross-check: guide claims vs source

| Guide claim (section) | Source | Verdict |
|---|---|---|
| pal.dat: 256 RGB triplets, 768 bytes (#the-dat-palette) | not read by the editor | NOT IN SOURCE |
| LoadPaletteFile: size 0x300 -> raw RGB, else PCX (#the-dat-palette) | not present | NOT IN SOURCE |
| Empty name -> grey ramp (#the-dat-palette) | `palette.c:12-22` (NULL or size < 1024 -> grey ramp) | AGREES (editor analogue) |
| Pl2 base palette stored as blue, green, red, pad (#loading-the-palette-writes-a-second-copy) | `palette.c:27-29` reads R first | DIFFERS (open, section 3) |
| Pl2 total 443,175 bytes (#the-pl2-file-is-a-cache) | probe: `pal0.bin`, `pal1.bin`, `pal4.bin` = 443,175 | AGREES |
| Darkness: 32 levels x 256 at 0x400 (#the-buffer-table-by-table) | offset `palette.c:197`, `1024 + 256*(c/8)` | AGREES on offset; editor uses row c/8 (per index), not per distance |
| Highlight 16 levels at 0x2400 | not referenced | NOT IN SOURCE |
| Alpha ~75/50/25 at 0x3500/0x13500/0x23500 (#the-buffer-table-by-table) | offsets AGREE (`structs.h:92-95`); names inverted vs guide labels | DIFFERS in naming; probe supports editor naming under source-weight reading (section 5) |
| Additive 0x33500, multiply 0x43500 | names COF_ALPHA, COF_LUMINANCE (`structs.h:95-96`) | DIFFERS in naming only |
| Screen blend 0x5B500 | name COF_ALPHABRIGHT (`structs.h:97`) | DIFFERS in naming only |
| Hue-rotation family, tint table, saturation | not referenced | NOT IN SOURCE |
| Row 0 transparency for pl2 tables (not in guide) | `misc.c:1301-1328` | (editor behaviour) |
| palshift.dat 2048 bytes, 8 x 256 (#monster-colour) | applied at `cof.c:149-169`; file size not checked here | NOT IN SOURCE for the size |
| Item transform files grey.dat etc. (#item-colour) | not referenced | NOT IN SOURCE |
| Unit glow / render-mode colours (#unit-glow) | not referenced | NOT IN SOURCE |
| Version: same format in 1.09d (#version-differences) | not checked here | NOT IN SOURCE |
| Blend trans_b mapping (not in guide) | `preview.c:291-379` | (editor behaviour; see section 6) |
| Gamma (not in guide) | `misc.c:430-467` | (editor behaviour; section 4) |

---

## 11. Open points

1. Base palette byte order: editor reads R from byte 0; guide and probe bytes say byte 0 is blue
   for index 150 (section 3). Resolve by printing `vga_pal[150]` in the reference build or by
   rendering a known tile. Do not port either reading until resolved.
2. Row names for 25/50/75 and percentage meaning (section 5). Editor naming is consistent with a
   source-weight reading; guide percentages are background weights in row 53. Verify with one
   composited sprite.
3. Stale comment `compat.h:80`: trans_b 3 and 4 are additive and multiply, not "screen/luminance".
4. CM_SHADOW reads row `c/8` directly (`palette.c:197`). A "per-distance" port will not match.
5. Missing Act 5 palette leaves `d2_pal[4]` NULL and falls back to grey (`main.c:905-908`,
   `palette.c:12-22`). Decide whether the port should fail or fall back.
6. Gamma is applied only to the base palette; blend and shadow tables are not gamma-corrected.
   Confirm this matches the look the port needs.
7. The cache `pal%d.bin` stores raw Pl2 bytes, so changing gamma does not invalidate it. Keep
   this if the port reuses the cache format.
8. Item transforms, palshift, unit glow, and tint table are game-only as far as this editor goes.
   Not needed for the DS1 editor; mark out of scope unless sprite preview needs them.
