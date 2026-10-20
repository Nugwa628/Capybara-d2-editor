# COF files, object resolution, blend modes, draw order: cross-check

Status: cross-check of `reference/d2-ds1-edit/docs/guides/cof-pipeline-1.13c.md` and
`reference/d2-ds1-edit/docs/guides/monsters-and-objects.md` against the reference C source.
Read-only; no code in this repo changes.

## Sources and conventions

- Guides (cited as `guide#anchor` or `guide.md#section`):
  - `reference/d2-ds1-edit/docs/guides/cof-pipeline-1.13c.md`. This is a **function-address map of the
    game DLLs**. It has no byte-level COF layout, no blend table and no draw-order rules.
  - `reference/d2-ds1-edit/docs/guides/monsters-and-objects.md`. DS1 unit ID resolution in the game.
- Source (cited as `reference/d2-ds1-edit/...:line`):
  - `src/core/cof.c` (`anim_load_cof` 195-532, `anim_load_desc_gfx` 536-564, `anim_update_gfx` 624-703),
    `src/core/cof.h`, `src/structs.h:328-360` (`LAY_INF_S`, `COF_S`).
  - `src/core/compose_cof.c` / `.h`: independent COF byte parser, used by compose export.
  - `src/render/preview.c`: blend table 291-379, layer draw 2060-2185, shadows 2188-2303, orderflag
    draws 2361-2494, draw passes 2574-2841, frame advance 1915-1946.
  - `src/editor/objects.c:15-80` (draw order), `src/core/txtread.c:1080-1200` (table columns),
    `src/core/animdata_math.c`, `src/main.c:1842` (tick rate), `src/main.c:331-334,445-451` (direction tables).
  - `assets/editor/obj.txt` (header line and rows).
- Verdicts: **AGREES**, **DIFFERS** (explained), **NOT IN SOURCE**, **NOT IN GUIDE** (as in `dc6-dcc.md`).

---

## 1. COF file layout (repo)

Two independent parsers exist. Their offsets agree (layers start at byte 28). They differ on
validation and on duplicate layers (§5).

| Offset | Size | Type | Field | Check / behaviour | Source |
|---|---|---|---|---|---|
| 0 | 1 | u8 | layer count (`lay`) | `cof.c` none; `compose_cof.c:48` requires 1..16 | `cof.c:283`; `compose_cof.c:43,48` |
| 1 | 1 | u8 | frames per direction (`fpd`) | `cof.c` none; `compose_cof.c:50` requires 1..1024 | `cof.c:287`; `compose_cof.c:44,50` |
| 2 | 1 | u8 | direction count (`dir`) | `cof.c` none; `compose_cof.c:52` requires 1..64 | `cof.c:292`; `compose_cof.c:45,52` |
| 3 | 1 | u8 | version | not read by `cof.c` (skipped within the 25 bytes); read by `compose_cof.c:46` | `compose_cof.c:46` |
| 3..27 | 25 | bytes | unknown, skipped | none | `cof.c:296`; `compose_cof.h:16` |
| 28 | 9 × lay | record | layer records, in COF order | see §1.1 | `cof.c:298-319`; `compose_cof.c:62-104` |
| 28 + 9·lay | fpd | u8 × fpd | per-frame flags, skipped | none | `cof.c:368`; `compose_cof.c:107-110` |
| then | dir·fpd·lay | u8 | priority table, see §1.2 | `cof.c` none; `compose_cof.c:117` checks length | `cof.c:373-383`; `compose_cof.c:112-125` |

There is **no** animation rate, bounding box or x/y offset field in the COF. The 25 bytes are not
decoded anywhere in the repo. Speed comes from AnimData (see `animdata.md`), and x/y offsets come from
Objects.txt (§2.3). **Guide does not describe any of this** (NOT IN GUIDE for the layout itself).

### 1.1 Layer record (9 bytes)

| Offset in record | Size | Field | Use | Source |
|---|---|---|---|---|
| +0 | 1 | composit index 0..15 | selects `lay_inf[idx]`; order is `HD TR LG RA LA RH LH SH S1 S2 S3 S4 S5 S6 S7 S8` | `cof.c:302`, `cof.c:207-209`, `compose_dcc_path.c:6-9` |
| +1 | 1 | shadow A (`shad_a`) | zero means no projected shadow for this layer | `cof.c:306`; `preview.c:2235` |
| +2 | 1 | shadow B (`shad_b`) | **parsed, never used** | `cof.c:308`; grep: no other use |
| +3 | 1 | transparency A (`trans_a`) | nonzero gates blending (§3) and shadow level 28 vs 10 | `cof.c:312`; `preview.c:2131,2151,2259` |
| +4 | 1 | transparency B (`trans_b`) | blend mode index (§3) | `cof.c:314`; `preview.c:2131` |
| +5 | 4 | weapon class (`wclass`) | copied into a 4-byte buffer; used as the DCC filename suffix. The 4th byte is assumed NUL. | `cof.c:318`, `cof.c:332`; trimmed in `compose_cof.c:90-102` |

Duplicates: `cof.c:306-318` writes `lay_inf[idx]` for each record, so the **last** record wins.
`compose_cof.c:81-82` keeps the **first**. See §5.

### 1.2 Priority table

- Index: `((dir · fpd) + frame) · lay + slot`, where `slot` is the draw position (`0..lay-1`).
  Value is the composit index to draw there. (`compose_cof.h:47-49`; `compose_cof.c:141-142`.)
- The draw loop reads it as `priority[(cur_dir·lay·fpd) + slot + new_frame·lay]` (`preview.c:2095-2097`,
  `preview.c:2224-2226`). Values `>= 16` are skipped (`preview.c:2098`).
- **`cur_dir` is the raw user direction** (`cof.c:278` sets `cof->cur_dir = user_dir`; `preview.c:2095`
  reads it). It is **not** passed through `new_dirN`. See §5, item 1.

### 1.3 Filenames

| Item | Format | Source |
|---|---|---|
| COF path | `{base}\{tok}\Cof\{tok}{mod}{clas}.cof` | `cof.c:253-255` |
| Layer DCC path | `{base}\{tok}\{CC}\{tok}{CC}{skin}{mod}{wclass}.dcc`, `CC` = component code | `cof.c:322-333`; `compose_dcc_path.c:40-44` |
| DC6 fallback | same path with last char replaced: `.dcc` → `.dc6` | `cof.c:350-351,360-361` |

Here `{base}`, `{tok}`, `{mod}`, `{clas}` come from obj.txt (`cof.c:549-553`). `{skin}` is obj.txt column
`HD`..`S8` for that component (`cof.c:322-323`). `{wclass}` comes from the COF layer record (`cof.c:332`).

**Guide comparison.** `guide#cof-filename-convention` gives `data\global\chars\{TOKEN}\COF\{TOKEN}{MODE}{WEAPON_CLASS}.cof`.
The repo's format matches in order (`cof.c:255`) but uses `Cof` where the guide has `COF`
(file lookup is case-insensitive). Verdict: **AGREES** (tokens and order), with **DIFFERS** on source of the
class token (repo: obj.txt `Class` column, not `GetWeaponClassToken`). Guide `#stage-1` gives only the
function addresses (`BuildCofPathString` `6fd93c30`), not format details beyond that.

---

## 2. How an object's COF and sprites are resolved (repo)

### 2.1 obj.txt row → COF

`assets/editor/obj.txt`, header (tab-separated, row 1):
`Act Type Id Description Objects.txt_ID Monstats.txt_ID Direction Base Token Mode Class HD TR LG RA LA RH LH SH S1 S2 S3 S4 S5 S6 S7 S8 Colormap Index *eol`

| Column | Role | Source |
|---|---|---|
| Act, Type, Id, Description | DS1 unit identity (`Type` 1 monster, 2 object) | `txtread.c:1123-1138` |
| Objects.txt_ID | row ID to look up in Objects.txt (`0` = none) | `txtread.c:1143`; `cof.c:403-407` |
| Monstats.txt_ID | monster stats link, not used by `cof.c` | `obj.txt` header |
| Direction | default direction, passed as `user_dir` | `cof.c:553-558` |
| Base, Token, Mode, Class | COF path tokens; **all four must be non-empty** | `cof.c:549-560` |
| HD..S8 | per-component skin token used in DCC names | `cof.c:322-323` |
| Colormap, Index | palette shift file and 1-based index | `cof.c:216-226` |

Flow (`anim_update_gfx` → `anim_load_desc_gfx` → `anim_load_cof`):
1. `anim_update_gfx` counts DS1 object usage and loads COFs only for used rows (`cof.c:636-680`).
2. `anim_load_desc_gfx` reads the row and calls `anim_load_cof` only when Base, Token, Mode and Class are non-empty (`cof.c:560-561`).
3. `anim_load_cof` loads the COF, then each layer, then AnimData speed, then Objects.txt (`cof.c:253-523`).

### 2.2 Palette shift

- Loaded only if both `Colormap` and `Index` columns are present and `Colormap` is non-empty and `Index != 0`
  (`cof.c:218-226`).
- Offset is `(Index - 1) · 256`, and the file must hold at least `offset + 256` bytes. Otherwise it is dropped (`cof.c:237-242`).
- Applied only to layers with `trans_a == 0` (`cof.c:343-347`, `cof.c:354-361`). Remap is RGB → palette index →
  shifted index → RGB (`cof.c:164-174`).

### 2.3 Object mode, Objects.txt row, speed and offsets

| Step | Rule | Source |
|---|---|---|
| Mode index | `NU`=0, `OP`=1, `ON`=2, `S1`=3, `S2`=4, `S3`=5, `S4`=6, `S5`=7. Any other mode returns the COF with no object data (monster COF). | `cof.c:412-436` |
| Objects.txt lookup | `id = obj.txt[Objects.txt_ID]`. If `id != 0`, scan Objects.txt column `Id` for an equal value. Not found returns early, with `objects_line = -1` and `orderflag` unset. | `cof.c:403-407,439-469` |
| Speed multiplier | `spd_mul = FrameDelta<mode>` from Objects.txt; `0` becomes `256`. | `cof.c:472-477`; `txtread.c:102-109` |
| Speed divisor | `spd_div = 256`. | `cof.c:480` |
| Default before override | `spd_mul = AnimData speed` (if record found), else `1`. `spd_div = 256`. | `cof.c:385-400` |
| X/Y offset | Objects.txt `Xoffset`, `Yoffset`, if the columns exist. Default `0`. | `cof.c:482-496` |
| Order flag | `OrderFlag<mode>` from Objects.txt. If that is `0`, use `OrderFlag0`. | `cof.c:498-524`; `txtread.c:111-118` |

- **`id == 0`** skips the lookup entirely (`cof.c:439`). The guide's note that Objects.txt record 0 crashes
  the game (`monsters-and-objects.md#adding-an-object-to-a-ds1`) is **not reproduced** in this repo; the
  editor just treats 0 as "no object row".
- **Default `orderflag`** is `0` (`memset` at `cof.c:274`), so any object without a matching row draws in pass 4.
- **Frame count** comes from the COF `fpd` (`cof.c:287`). The AnimData `FramesPerDir` is read but its
  assignment is commented out (`cof.c:396-397`).

### 2.4 Monsters vs objects

The COF loader does not separate them up front. Both load their layers. Object data is attached only when
both hold: the mode is one of `NU`..`S5` (`cof.c:412-436`), and `Objects.txt_ID` is non-zero and found
(`cof.c:439-469`). Monster rows in obj.txt leave `Objects.txt_ID` empty (`assets/editor/obj.txt`, first rows),
so they keep `orderflag == 0` and the AnimData speed or the default `1/256`. A monster mode such as `TN` or
`RN` returns early at `cof.c:436`.

Checked against the file (awk over `assets/editor/obj.txt`, column 2 = Type): all 300 Type 1 rows have an empty
`Objects.txt_ID`; all 750 Type 2 rows have one. Type 2 rows use modes `NU` and `ON` in the sample rows.

### 2.5 Guide vs. repo on the game's ID rules (monsters-and-objects)

| # | Guide claim | Source | Verdict | Note |
|---|---|---|---|---|
| M1 | Type 2 unit ID < 150 indexes a hardcoded per-act table; ≥ 150 subtracts 150; negatives read earlier blocks | none | **NOT IN SOURCE / DIFFERS (scope)** | The repo does not implement DS1 unit ID resolution. obj.txt holds an explicit `Objects.txt_ID` per row (`txtread.c:1143`). |
| M2 | DS1 Act field clamped to 4 | none | NOT IN SOURCE | Repo has an Act column in obj.txt, not clamped (`txtread.c:1123`). |
| M3 | Objects.txt record 0 crashes the game | none | **DIFFERS** | Repo treats `id == 0` as "no row" (`cof.c:439`). |
| M4 | Type 1 units resolve through MonPreset/MonStats | none | NOT IN SOURCE | Repo reads only obj.txt. |

---

## 3. Blend modes (`trans_a`, `trans_b`)

Gate, `preview.c:2131` (normal scale) and `preview.c:2151` (stretch):
`if (trans_a != 0 && trans_b <= 6 && trans_b != 5)` → blended path (`wpreview_draw_blended_bitmap`
`preview.c:291`, or the scaled variant `preview.c:325`). Otherwise the layer is drawn opaque (`preview.c:2137`, `preview.c:2159`).

| `trans_b` | Blended draw (normal scale) | Blended draw (stretch) | Source |
|---|---|---|---|
| 0 | tinted alpha 0.25 | tint 0.25 | `preview.c:295-297`; `preview.c:343-347` |
| 1 | tinted alpha 0.50 | tint 0.50 | `preview.c:298-300`; `preview.c:348-352` |
| 2 | tinted alpha 0.75 | tint 0.75 | `preview.c:301-303`; `preview.c:353-357` |
| 3 | additive: `ALLEGRO_ONE`, `ALLEGRO_ONE` | same | `preview.c:304-308`; `preview.c:358-362` |
| 4 | `ALLEGRO_DEST_COLOR`, `ALLEGRO_ZERO` (multiply) | same | `preview.c:309-313`; `preview.c:363-367` |
| 5 | **not blended** by the gate. The `default` branch (alpha 0.50) is unreachable from the layer loop. | same | `preview.c:2131`, `preview.c:320`, `preview.c:373-377` |
| 6 | `ALLEGRO_ONE`, `ALLEGRO_INVERSE_SRC_COLOR` (screen-like) | same | `preview.c:314-318`; `preview.c:368-372` |
| 7..255 | **not blended** (gate `<= 6`) | same | `preview.c:2131,2151` |
| any, with `trans_a == 0` | **not blended** | same | `preview.c:2131,2151` |

Blender reset after each case: `ALLEGRO_ADD, ALLEGRO_ALPHA, ALLEGRO_INVERSE_ALPHA` (`preview.c:307,312,317`).

Shadows (`preview.c:2188-2303`): only drawn when `shad_a != 0` (`preview.c:2235`). Level is `28` if `trans_a`
else `10` (`preview.c:2259`). Scaled alpha is `1 - level/32` (`preview.c:2277`). Comment at
`preview.c:2256` says this is D2 darkness on a 0-32 scale.

**Guide.** `cof-pipeline-1.13c.md` has no blend table. Verdict for the whole section: **NOT IN GUIDE**.
Stage 6 (`#stage-6-rendering-pipeline-d2clientdll`) mentions only `GetUnitPaletteTransform`, which is
a different mechanism.

---

## 4. Draw order and orderflag

### 4.1 Sort (`editobj_set_drawing_order`, `objects.c:48-80`)

- Tile coordinates: `tx = x / 5`, `sx = x % 5`, `ty = y / 5`, `sy = y % 5` (`objects.c:55-61`).
- Unused object slots get `ty = 32000` so they sort last (`objects.c:64-66`).
- Comparator (`objects.c:15-43`): by `ty`, then `tx`, then `obj_sub_tile_order`, then index.
  **Bug:** the equality test uses `[sx][sy]` (`objects.c:31-32`) but the returned difference uses `[sy][sx]`
  (`objects.c:36-37`). Equal keys fall back to index, unequal keys use the swapped table entry. Verify
  before reuse. See §5, item 4.

### 4.2 Passes (`wpreview_draw_tiles`, `preview.c:2574-2841`)

| Pass | Work | Object filter | Source |
|---|---|---|---|
| 1 | base terrain: lower walls, floors, tile shadows | none | `preview.c:2687` |
| 2 | object shadows | `shad_a != 0` per layer | `preview.c:2731`, `preview.c:2306` |
| 3 | objects behind walls | `orderflag == 1` only | `preview.c:2761-2786`, `preview.c:2478` |
| grid | tile grid if `TG_OVERFLOOR` | none | `preview.c:2793-2795` |
| 4 | upper walls, then objects in front | `orderflag == 0 or 2` | `preview.c:2797-2835`, `preview.c:2410` |
| 5 | roofs | none | `preview.c:2841` |
| 6 | special tiles (optional) | none | `preview.c:2868` |
| 7 | walkable overlay (debug, optional) | none | `preview.c:2898` |

- **Objects with other `orderflag` values are never drawn** (`preview.c:2410`, `preview.c:2478`).
- The per-tile cursor walks `drawing_order` in order. Each pass keeps its own cursor (`objdraw_cur_idx`).
- The big-screenshot path repeats the same passes without viewport culling (`preview.c:3364-3750`).

### 4.3 Per-layer draw inside one object (`wpreview_draw_an_object`, `preview.c:2060-2185`)

- Layer order: `priority[...]` (§1.2).
- Position: `dx = dx0 - win.x0 + (xoffset + off_x) · mul / div`, `dy` likewise (`preview.c:2117-2120`).
  `off_x`, `off_y` are the layer's bitmap origin (`cof.c:133-134`).
- Frame index: `new_frame = cur_frame + frame_delta` (wrapped by `fpd`, `preview.c:2080-2082`).
  If `new_frame >= bmp_num`, `last_good_frame` is used (`preview.c:2107-2110`, `2165-2166`).

### 4.4 Frame advance (`wpreview_obj_animate`, `preview.c:1915-1946`)

Tick: `al_create_timer(1.0/25.0)` → **40 ms** per tick (`main.c:1842`; counted at `interface.c:164`).
Per tick:
```
n = ticks · spd_mul + spd_mod
cur_frame += n / spd_div      (spd_div = 256)
spd_mod    = n % spd_div
cur_frame  wraps at fpd
```
- `spd_mul = 256` → 1 frame per tick → 25 fps. Frame delay `= 40 · 256 / spd_mul` ms (`animdata_math.c:7-21`;
  `animdata.h:20-23` says the same). Clamped to `[1, 10000]` ms, `0` or `> 65535` → default.
- Order of defaults: AnimData → Objects.txt FrameDelta override (§2.3).

**Guide.** `cof-pipeline-1.13c.md#stage-2-animation-data-frame-management` describes the game's accumulator
(`AdvanceAnimSubAccumulator`, `AdvanceAnimFrameWithWrap`) and the Objects.txt columns `FrameCnt`, `FrameDelta`,
`Start` (`cof-pipeline-1.13c.md:104,112`). Verdicts:
- Accumulator and wrap: AGREES in principle (`spd_mod` ≈ sub-accumulator; `preview.c:1939-1943`).
- `FrameCnt` and `Start`: **NOT IN SOURCE**. Repo reads only `FrameDelta<mode>` (`txtread.c:102-109`).

---

## 5. Open points

1. **Direction index mismatch (`cof.c` path).** `cof.c:103-110` maps the user direction through
   `new_dirN` (`main.c:331-334`: `dir8 = {4,0,5,1,6,2,7,3}`) to pick the **sprite** direction. The
   **priority** table is indexed by the raw `cur_dir = user_dir` (`cof.c:278`, `preview.c:2095`). For
   8 directions, user 0 uses sprite 4 but priority row 0. Unless the COF priority table is in
   screen order, one of them is wrong. The compose path (`compose_render.c:470-471`) uses the file
   direction for both, so it is internally consistent. Decide which is correct before porting.
2. **Duplicate layer records.** Last-wins in `cof.c:306-318`, first-wins in `compose_cof.c:81-82`. Real
   COFs with duplicates exist (Baal Throne, Mephisto; `compose_cof.c:74-80`). The editor's behaviour
   is the one the viewer shows, so match `cof.c` unless proved otherwise.
3. **Header comment is stale.** `compose_cof.h:16-17` says layer records start at offset 29. Code
   (`compose_cof.c:28`, `cof.c:296`) starts them at 28. The code is right per `compose_cof.c:22-27`.
4. **Sub-tile sort key.** `objects.c:31-37` uses `[sx][sy]` for equality and `[sy][sx]` for the difference.
   Confirm intent.
5. **`wclass` not NUL-terminated.** `cof.c:318` copies 4 bytes; `cof.c:324` formats with `%s`. Real
   COFs use `HTH\0` style padding, but this is not guaranteed. The compose parser (`compose_cof.c:90-91`)
   terminates it.
6. **No bounds checks in `cof.c`.** `rand() % fpd` with `fpd == 0` (`cof.c:288`); `lay_inf[idx]` with
   `idx >= 16` (`cof.c:302-315`); priority `memcpy` past end of buffer (`cof.c:383`). See `dc6-dcc.md` §2.5
   (F1-F5).
7. **Object speed with no AnimData record.** Default `spd_mul = 1` (`cof.c:386`) gives a near-static
   animation. Confirm this is wanted.
8. **Wrong guide-to-repo naming.** `guide#stage-3` names `GetEquipSlotAnimComponent` etc. These are
   character-equipment functions. The repo has no equipment logic (obj.txt is a fixed table).
9. **Not verified here:** game addresses (every `6fd…` or `6fe…` row in the guide), the claim that
   `GetObjectsTxtRecord` uses `id * 0x1C0 + base`, the 1.09d/1.13c differences, and the companion
   `cof-pipeline-1.13c.verification.md`. No binary or MPQ data is in the reference tree.
10. **obj.txt Type/Id resolution** (guide §2.5) is not implemented in the repo. If the Rust side needs
    game-accurate DS1 unit resolution, it must come from a different source than `obj.txt`.

---

## Writer (`Cof::to_bytes`)

`d2_formats::cof::Cof` keeps what the parser does not decode, so a parsed file writes back
byte-identically:

- `header`: bytes 3..28 as stored (25 bytes). For a new COF, `Cof::object_header(x_min, x_max,
  y_min, y_max, rate)` builds the header the game's own object COFs have: version `0x14` at byte 3,
  `00 00 13 00` after it, then the box (four `i32` LE at 8, 12, 16, 20) and the animation rate (`i16`
  LE at 24). Same layout ds1-studio's `writeCof` writes.
- `trailing`: bytes after the priority table (the parser ignores them).
- `missing_bytes`: a short file (three Median XL COFs) is written short again; the zero fill is cut.

`to_bytes` refuses a model whose key-frame table is not `frames_per_direction` long, whose priority
table is not `directions * frames * layers` long, or with more than 255 layers.

Gate: all 8,817 COFs of the vanilla and Median XL corpora (8,815 readable; `amblxbow.cof` is noise in
both) write back identically (`corpus_cof_files_write_back_identically`, full run with
`CAPY_FULL_CORPUS=1`).
