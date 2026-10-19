# DC6 and DCC sprite formats: layout, decoder rules, cross-check

Status: cross-check of `reference/d2-ds1-edit/docs/guides/sprite-formats-dcc-dc6.md`
against the reference C source. Read-only reference work; no code in this repo changes.

## Sources and conventions

- Guide: `reference/d2-ds1-edit/docs/guides/sprite-formats-dcc-dc6.md` (cited as `guide#anchor`).
  Its companion ledger, `sprite-formats-dcc-dc6.verification.md`, was not reconciled line by line.
- Source (cited as `reference/d2-ds1-edit/...:line`):
  - `src/core/dc6.c`, `src/core/dc6.h`: DC6 header and frame decode, box, anchor.
  - `src/core/dcc.c`, `src/core/dcc.h`: DCC header, direction, bitstreams, two-stage decode.
  - `src/core/cof.c:72-191` (`anim_load_dcc`), `cof.c:343-363` (DCC-then-DC6 fallback).
  - `src/core/compose_render.c:95-292`: second DC6/DCC loader with its own checks.
- Verdicts: **AGREES** (source matches the claim), **DIFFERS** (source differs; explained),
  **NOT IN SOURCE** (claim is about the game binary or data, not checkable from the repo),
  **NOT IN GUIDE** (source behaviour the guide does not describe).
- The repo's DC6 and DCC parsers are both derived from Paul Siramy's `win_ds1edit` (`NOTICE`;
  `dc6.c:1`, `dcc.c:1`). The guide itself says they are one source, not two confirmations.
- Anything the guide calls "the game" is D2CMP.dll 1.13c. Nothing in this repo runs D2CMP.dll.

---

## 1. Layout tables

### 1.1 DC6 file

| Offset | Size | Type | Name (repo) | Condition / check | Meaning | Source |
|---|---|---|---|---|---|---|
| 0x00 | 4 | i32 LE | version | must equal 6 (else reject) | format version | `dc6.c:162,168` |
| 0x04 | 4 | i32 LE | flags (`dc6_unk1`) | must equal 1 | unknown, named flags in guide | `dc6.c:163,168` |
| 0x08 | 4 | i32 LE | encoding (`dc6_unk2`) | must equal 0 | unknown, named encoding in guide | `dc6.c:164,168` |
| 0x0C | 4 | i32 | termination | not read | uninitialised in files | `dc6.c` (never read) |
| 0x10 | 4 | i32 | directions | must equal `cof->dir` (`dc6.c:175`) | direction count | `dc6.c:165,175` |
| 0x14 | 4 | i32 | framesPerDirection | >0 implied; not checked | frames per direction | `dc6.c:166` |
| 0x18 | 4 × dirs × fpd | u32 | frame offset table | direction-major: index `dir*fpd + f` | absolute byte offset of each frame record | `dc6.c:167,199,254` |

Header is 24 bytes before the table (`dc6.c:161-167`, `lptr[6]` is the table). `compose_render.c:99`
requires `buf_len >= 24`.

### 1.2 DC6 frame record (at the table offset)

| Offset | Size | Type | Field | Used by decoder | Source |
|---|---|---|---|---|---|
| +0x00 | 4 | i32 | flip | **never read** | `dc6.c` (lptr[0] unused) |
| +0x04 | 4 | i32 | width | yes | `dc6.c:208,268` |
| +0x08 | 4 | i32 | height | yes | `dc6.c:209,269` |
| +0x0C | 4 | i32 | offsetX (left column) | yes | `dc6.c:210,270,213` |
| +0x10 | 4 | i32 | offsetY (bottom row, inclusive) | yes | `dc6.c:211,271,215-216` |
| +0x14 | 4 | i32 | unknown | never read | `dc6.c` |
| +0x18 | 4 | u32 | nextBlock | **never read**; do not use | `dc6.c` |
| +0x1C | 4 | i32 | length of RLE data (bytes) | yes | `dc6.c:272` |
| +0x20 | length | bytes | RLE scanline stream | yes | `dc6.c:273` |
| after data | 3 | bytes | slack | never read | `dc6.c` |

### 1.3 DCC file

| Offset | Size | Type | Field | Check in repo | Source |
|---|---|---|---|---|---|
| 0x00 | 1 | u8 | signature | **none** (guide says must be 0x74) | `dcc.c:1463` |
| 0x01 | 1 | u8 | version | **none** (guide says 6) | `dcc.c:1466` |
| 0x02 | 1 | u8 | directions | **none** (guide says ≤ 32) | `dcc.c:1469` |
| 0x03 | 4 | u32 | framesPerDirection | **none** | `dcc.c:1472` |
| 0x07 | 4 | u32 | tag | read, unused (`dcc.h:144` says "always 1") | `dcc.c:1475` |
| 0x0B | 4 | u32 | finalDc6Size | read into `header.final_dc6_size`, never used | `dcc.c:1478`; grep: no other use |
| 0x0F | 4 × dirs | u32 | direction offsets, absolute from file start | none | `dcc.c:1481-1483`, `dcc.c:1278` |

Header is 15 bytes plus `4*dirs` (`dcc.c:1463-1483`). There is no end sentinel in the repo.
Each direction's size is `dir_offset[d+1] - dir_offset[d]`, and the last direction runs to file end
(`dcc.c:1279-1283`). The guide describes a sentinel the game writes past the last offset (guide
`#the-file-header`); the repo computes the same extent from `dcc->size`, so the effect agrees.

### 1.4 DCC direction header (bit-packed, LSB-first)

Each direction begins at `dir_offset[d]`.

| Bits | Field | Source |
|---|---|---|
| 32 (read as 32 bits from the stream start, which is a dword boundary) | outsizeCoded | `dcc.c:1286` |
| 2 | compressionFlag (bit1 gates equalCell, bit0 gates encodingType + rawPixel) | `dcc.c:1293`, `dcc.c:275,293` |
| 4 | variable0_bits: width code, index into bit table | `dcc.c:1300` |
| 4 | width_bits (code) | `dcc.c:1307` |
| 4 | height_bits (code) | `dcc.c:1314` |
| 4 | xoffset_bits (code, signed field) | `dcc.c:1321` |
| 4 | yoffset_bits (code, signed field) | `dcc.c:1328` |
| 4 | optional_bytes_bits (code) | `dcc.c:1335` |
| 4 | coded_bytes_bits (code) | `dcc.c:1342` |

The 4-bit values are codes, not bit counts. `dcc_bits_width_table[16] = {0,1,2,4,6,8,10,12,14,16,20,24,26,28,30,32}`
(`dcc.c:22-23`). Code 4 means 6 bits; code 15 means 32.

### 1.5 DCC frame headers (`framesPerDirection` of them, packed back to back, no alignment)

Read order and width (`dcc.c:169-230`):

| Order | Field | Width | Signed | Source |
|---|---|---|---|---|
| 1 | variable0 | `variable0_bits` code | no | `dcc.c:174-176` |
| 2 | width | `width_bits` code | no | `dcc.c:180-182` |
| 3 | height | `height_bits` code | no | `dcc.c:186-188` |
| 4 | xoffset | `xoffset_bits` code | **yes** | `dcc.c:192-196` |
| 5 | yoffset | `yoffset_bits` code | **yes** | `dcc.c:199-203` |
| 6 | optional_bytes | `optional_bytes_bits` code | no | `dcc.c:206-209` |
| 7 | coded_bytes | `coded_bytes_bits` code | no | `dcc.c:212-215` |
| 8 | bottom_up | 1 bit | no | `dcc.c:218-220` |

Derived frame box (`dcc.c:233-252`): `xmin = xoffset`, `xmax = xmin + width - 1`.
If `bottom_up`: `ymin = yoffset`, `ymax = ymin + height - 1`. Otherwise `ymax = yoffset`, `ymin = ymax - height + 1`.
Direction box is the union of its frame boxes (`dcc.c:1350-1381`).

### 1.6 DCC direction payload, after the frame headers

| Order | Item | Width / count | Present when | Source |
|---|---|---|---|---|
| 1 | optional bytes (per frame, byte-aligned block) | sum of optional_bytes | sum > 0 (none in vanilla) | `dcc.c:369-421` |
| 2 | equalCell stream size | 20 bits | flag & 0x02 | `dcc.c:433-443` |
| 3 | pixelMask stream size | 20 bits | always | `dcc.c:446-453` |
| 4 | encodingType stream size | 20 bits | flag & 0x01 | `dcc.c:455-466` |
| 5 | rawPixel stream size | 20 bits | flag & 0x01 | `dcc.c:468-475` |
| 6 | pixel-value key | 256 bits, one per palette index | always | `dcc.c:494-505` |
| 7 | equalCell bitstream | `equal_cell_bitstream_size` bits | flag & 0x02 | `dcc.c:275-283` |
| 8 | pixelMask bitstream | `pixel_mask_bitstream_size` bits | always | `dcc.c:286-291` |
| 9 | encodingType bitstream | size bits | flag & 0x01 | `dcc.c:293-301` |
| 10 | rawPixel bitstream | size bits | flag & 0x01 | `dcc.c:303-309` |
| 11 | pixelCodeAndDisplacement | rest of direction (no size) | always | `dcc.c:312-317` |

Items 7 to 11 are contiguous at bit granularity, each starting where the previous ended
(`dcc.c:264-317`, `glb_bit` accumulator).

### 1.7 Pixel-value key

`pixel_values[]` is a 256-entry map from direction-local code to palette index. Entries are the set
bit positions, ascending (`dcc.c:494-505`). `dcc.h:132` declares it as `UBYTE pixel_values[256]`.

---

## 2. Decoder rules (with source refs)

### 2.1 DC6 scanline decode (`dc6_decomp_norm`, `dc6.c:52-88`)

| Opcode byte `c` | Action | Source |
|---|---|---|
| `0x80` | end of scanline: `x = x0`, `y--` (row advances upward) | `dc6.c:68-72` |
| `0x81`..`0xFF` | skip `c & 0x7F` pixels (transparent), `x += n` | `dc6.c:73-74` |
| `0x00`..`0x7F` | copy the next `c` bytes as palette indices; `c == 0` copies nothing | `dc6.c:75-84` |

- No repeat or run opcode. Row end is unconditional; a row may end before `width` (`dc6.c:70`).
- The `cmap` variant (`dc6_decomp_cmap`, `dc6.c:93-130`) maps each index through a 256-byte table first.
- Stream is consumed by byte count (`size`, `dc6.c:64`); a well-formed stream ends exactly at `length`.

### 2.2 DC6 placement and orientation (`anim_load_dc6`, `dc6.c:134-318`)

1. For every frame in the selected direction, compute `x1 = min(offsetX)`, `x2 = max(offsetX+width-1)`,
   `y2 = max(offsetY)`, `y1 = min(offsetY-height+1)` (`dc6.c:194-228`). Box size is `w = x2-x1+1`, `h = y2-y1+1`.
2. Every frame bitmap is `w × h` (`dc6.c:276`). Its origin is the box top-left (`x1`, `y1`).
3. Frame decode start (`dc6.c:296-297`):
   - `x0 = offsetX - x1` (left column of the frame).
   - `y0 = (h - 1) + offsetY - y2` (bottom row of the frame). Decoding moves upward (`y--` on `0x80`).
4. Frame bitmap row 0 is the top of the box. Scanlines are stored bottom-up.
5. `compose_render.c:173-175` uses the same rule with `x0 = fox - box_x1`, `y0 = foy - box_y1`
   (`box_y1` = top of box). This agrees with step 3.
6. The layer's `off_x`, `off_y` are the box origin (`dc6.c:236-237`).

### 2.3 DCC direction decode

Top-level order (`dcc_decode`, `dcc.c:1495-1537`; `dcc_dir_bitstream`, `dcc.c:1255-1444`):

1. Read the 15-byte file header and direction offset table (`dcc_file_header`, `dcc.c:1449-1488`).
2. For each requested direction (bitfield): read the direction header (§1.4), frame headers (§1.5),
   optional data, bitstream sizes, 256-bit key, and build bitstream cursors (`dcc.c:1286-1396`).
3. Stage 1, `dcc_fill_pixel_buffer` (`dcc.c:739-1025`): build a list of cell pixel entries, one per
   changed cell.
4. Stage 2, `dcc_make_frames` (`dcc.c:1064-1249`): expand each entry to pixels and composite frames.
5. Post-checks that each stream was fully consumed (`dcc.c:1414-1441`).

**Cell grid** (`dcc.c:515-612`, `dcc.c:619-733`):
- Direction grid: `cells_w = ceil(box_w/4)`, `cells_h = ceil(box_h/4)` (`dcc.c:526-532`).
  Last column/row is narrower (`dcc.c:571`, `dcc.c:580`).
- Frame grid is phase-aligned to the direction grid: first column width is `4 - ((frame.xmin - dir.xmin) % 4)`
  (`dcc.c:632`), same for rows (`dcc.c:645`). Frame cell `(x0,y0)` is relative to the direction box (`dcc.c:712-724`).

**Stage 1 per frame, per cell** (`dcc.c:808-1007`):

| Step | Condition | Bits read | Action | Source |
|---|---|---|---|---|
| a | cell never written in this direction (`cell_buffer[cur]==NULL`) | 0 | pixel_mask = `0xF` | `dcc.c:882-883` |
| b | cell previously written and equalCell stream present | 1 from equalCell | bit 1 = unchanged, skip the cell | `dcc.c:845-881` |
| c | cell written, bit 0 | 4 from pixelMask | mask of changed colour slots | `dcc.c:865-866` |
| d | `nb_pix = popcount(mask)` > 0 and encodingType stream present | 1 from encodingType | `encoding_type` | `dcc.c:893-910` |
| e | per changed code, `encoding_type == 1` | 8 from rawPixel | raw code | `dcc.c:916-928` |
| f | per changed code, `encoding_type == 0` | nibbles from pixelCodeAndDisplacement | delta from previous code; a nibble of 15 means "add the next nibble too" | `dcc.c:929-960` |
| g | code equals previous code (`read_pixel[i] == last_pixel`) | 0 | stop decoding codes for this cell (applies to both modes) | `dcc.c:962-966` |
| h | after decode | | new pixel-buffer entry: unchanged slots copied from previous entry, changed slots popped from decoded codes | `dcc.c:975-1004` |

**Stage 1 slot assignment (source behaviour the guide does not describe):**
`curr_idx = decoded_pix - 1`; iterating slots `0..3` in ascending order, each changed slot takes
`read_pixel[curr_idx--]`. Once the stack is empty, remaining changed slots get `0` (`dcc.c:989-997`).
In other words, the codes are assigned to changed slots in **reverse** decode order, and missing ones are
zero, not carried over. See §5.

**Stage 1 finish:** each stored code is replaced by `pixel_values[code]` (`dcc.c:1012-1019`).

**Stage 2 per frame, per cell** (`dcc.c:1097-1244`):

| Condition | Action | Source |
|---|---|---|
| current pixel-buffer entry is not for (this frame, this cell index) | copy path: if buffer cell's last size equals this cell's size, blit last position into the new position and into the frame; else clear the cell | `dcc.c:1116-1144` |
| entry matches | fill path: bits per pixel from the entry's four codes (rule below), pixels read row-major | `dcc.c:1145-1218` |
| after either | composite the frame cell into the frame bitmap; advance pixel-buffer cursor only on fill path; record last size/origin for the buffer cell | `dcc.c:1221-1239` |
| end of frame | save as the frame bitmap | `dcc.c:1243`, `dcc.c:1029-1058` |

**Bits per pixel** (`dcc.c:1149-1159`): `val[0]==val[1]` → 0 bits (whole cell is `val[0]`);
`val[1]==val[2]` → 1 bit; otherwise 2 bits. Index is `pixel_code`, read row-major
(`dcc.c:1191-1212`).

**Frame box vs bitmap.** Each DCC frame bitmap is the direction box size (`dcc.c:1086`, `dcc.c:1243`).
A frame's pixels sit at `(frame.xmin - dir.xmin, frame.ymin - dir.ymin)` inside it (`dcc.c:712-724`).
`cof.c:131-134` and `compose_render.c:280-287` take the direction box origin as the layer offset.

### 2.4 Byte and bit order

- Bitstream reads are LSB-first within each byte (`dcc.c:121-122`, `dcc.c:131-137`).
- `dcc_read_bytes` requires byte alignment and is used only for the file header and optional data (`dcc.c:41-46`).
- Signed fields are sign-extended from their own width (`dcc.c:146-150`). Zero-width reads of a signed field
  evaluate `1 << -1`, which is UB in C (`dcc.c:146`); the result is harmless only because the value is 0.

### 2.5 Decoder bounds checks

The repo checks some bounds and misses others. "Missing" means the code as written does not guard it.

**DC6 (`dc6.c`, plus `compose_render.c:95-203` for comparison)**

| # | Where | Status | Note |
|---|---|---|---|
| D1 | `dc6.c:161-167` header reads | missing | No check that `len >= 24 + 4*dirs*fpd`. `compose_render.c:99` checks `>= 24` only. |
| D2 | `dc6.c:199,254` table index `dir*fpd+i` | missing | Index not checked against `dirs*fpd`. Only `dc6_dir == cof->dir` is checked (`dc6.c:175`). |
| D3 | `dc6.c:200,255` `offset >= len` | partial | Negative offset (from i32) is not rejected. Header of 32 bytes is not checked to fit: needs `offset + 32 <= len`. `compose_render.c:126` uses `off >= buf_len-24`, which still permits up to 8 bytes of overread. |
| D4 | `dc6.c:272` `f_len` | missing | Not checked against `len - (offset+32)`. |
| D5 | `dc6.c:79-83` inner copy loop | missing | Inner `c2 = *ptr++` loop checks `i2 < c` but not `i < size`. A short stream reads past the buffer. |
| D6 | `dc6.c:81` `al_put_pixel(x,y,...)` | unknown | No clipping in decoder; `x` may run past `width`. Clipping depends on Allegro, not checked here. |
| D7 | `dc6.c:276` `al_create_bitmap(w,h)` | missing | `w`, `h` from the file are not capped. `w*h` may overflow. |
| D8 | `dc6.c:240` `dc6_fpd * sizeof(ptr)` | missing | No cap on `fpd`. |

**DCC (`dcc.c`, `dcc.h`, `cof.c`, `compose_render.c`)**

| # | Where | Status | Note |
|---|---|---|---|
| C1 | `dcc.c:1481-1483`, `dcc.h:147` | **missing** | `directions` is a byte (up to 255) written into `dir_offset[DCC_MAX_DIR=32]`. Overflow for `directions > 32`. |
| C2 | `dcc.c:1356-1358`, `dcc.h:158` | **missing** | `frames_per_dir` is unbounded. `frame[32][256]` overflows above 256. Repo's own `DCC_MAX_FRAME` is not enforced. |
| C3 | `dcc.c:1511-1513` `dcc_decode` | missing | Requested direction ≥ `header.directions` decodes nothing and returns success. `cof.c:131-132` then calls `al_get_bitmap_width(dcc->frame[dir][0].bmp)` on NULL. `compose_render.c:242-246` range-checks after decode, which is correct. |
| C4 | `dcc.c:1513`, `cof.c:113`, `compose_render.c:236` `1 << d` | UB for d ≥ 31 | Use 64-bit shift or range check. |
| C5 | `dcc.c:1278-1283` | missing | `dir_offset[d]` not checked to be ≤ file size or monotonic. Difference is `UDWORD`, so it wraps. Bit cursor data pointer may lie outside `dcc->ptr`. |
| C6 | `dcc.c:275-317`, `dcc.c:309,317` | missing | Declared stream sizes are not checked against the direction extent. `pcd->size = bs->size - glb_bit` underflows if `glb_bit > bs->size`. |
| C7 | `dcc.c:377-398` optional bytes | missing | Sum is `UDWORD`, malloc per frame, no cap. |
| C8 | `dcc.c:112-117` `dcc_read_bits` | present | Reads check `cur_bit_num >= size`. |
| C9 | `dcc.c:831-841` cell index | present | `curr_cell >= nb_cell` checked. |
| C10 | `dcc.c:977-987` pixel buffer | present | `DCC_MAX_PB_ENTRY = 85000` enforced (`dcc.h:14`). |
| C11 | `dcc.c:1110-1113`, `dcc.c:1067,1228` stage 2 | missing | `cell_idx` and `pbe` pointer not checked. Correct only if stage 1 is consistent. |
| C12 | `dcc.c:1131-1142`, `dcc.c:1152`, `dcc.c:1221` | missing | `cell->bmp` may be NULL (sub-bitmap creation can fail) and is used without a check. Only the pixel path (`dcc.c:1184`) checks it. |
| C13 | `dcc.c:1418,1425,1432,1439` | **bug** | `strcat(dcc_error, add_error)` on an uninitialised `add_error[80]`. |
| C14 | `dcc.c:1204-1208`, `dcc.c:1245-1248` | leak | Error paths do not free `frm_bmp` or the cell lists. |
| C15 | `dcc.c:146` | UB | `1 << (bits-1)` with bits = 0 (see §2.4). |
| C16 | `dcc.c:776-783`, `dcc.c:1086-1094` | present | Zero-size direction box fails at bitmap creation; checked. A zero-size frame box is not checked separately. |

**COF (`cof.c:72-532`)**

| # | Where | Status | Note |
|---|---|---|---|
| F1 | `cof.c:288` `rand() % cof->fpd` | **missing** | Division by zero when `fpd == 0`. `compose_cof.c:50` rejects `fpd <= 0`. |
| F2 | `cof.c:302-315` `lay_inf[idx]` | **missing** | `idx` is a file byte; no check `idx < 16`. `compose_cof.c:70` checks. |
| F3 | `cof.c:280-383` all reads | missing | Pointer walk without length check; `memcpy(priority, bptr, dir*fpd*lay)` at `cof.c:383`. `compose_cof.c:108-118` checks `len`. |
| F4 | `cof.c:318-333` `wclass` and `sprintf` | **missing** | 4 bytes copied with `memcpy`, no NUL. `%s` may run past the field. `compose_cof.c:90-91` NUL-terminates. |
| F5 | `cof.c:253,324` `name[256]` `sprintf` | missing | Unbounded `base`/`tok`/`sptr`. Use `snprintf`. |

**Layer bitmap load (`cof.c:131-134`)**
- `dcc->frame[dir][0].bmp` is read before checking `frames_per_dir >= 1`. See also C3.

---

## 3. Cross-check of guide claims

Sections in the guide are cited as `guide#<heading>`. Verdicts as defined above.

### 3.1 Which format the game asks for (`guide#which-format-the-game-asks-for`)

| # | Guide claim | Source | Verdict | Note |
|---|---|---|---|---|
| X1 | Extension is chosen by `BuildSpritePath`, no sniffing or fallback | `cof.c:343-363` | **DIFFERS** | Repo tries `.dcc` first, then rewrites the name to `.dc6` on any failure (`cof.c:350-351, 360-361`). Failure includes "file missing", so the repo accepts a `.dc6` under a `.dcc` name by fallback. The game does not. Mode, token-table and class logic is not in the repo. |
| X2 | Object ids `0x156`, `0x233` and monster ids etc. force DC6 | not in repo | **NOT IN SOURCE** | No id-to-extension table in `src/`. |

### 3.2 One descriptor, two files (`guide#one-descriptor-two-files`)

| # | Guide claim | Source | Verdict | Note |
|---|---|---|---|---|
| X3 | DCC `version` at +0x04 is 6; DC6 `version` is 6 | `dcc.c:1466`, `dc6.c:162` | AGREES (values) | Repo reads them; DCC version is not validated (see §1.3). |
| X4 | DCC direction count cap 32, DC6 cap 32 and `dirs*fpd < 0x4000` | none | **DIFFERS** | Repo enforces neither cap. See C1, C2, D2. |

### 3.3 DC6 (`guide#dc6-the-format-that-is-already-pixels`)

| # | Guide claim | Source | Verdict |
|---|---|---|---|
| X5 | 24-byte header: version, flags, encoding, termination, dirs, fpd | `dc6.c:161-167` | AGREES |
| X6 | `termination` not read | `dc6.c` (lptr[3] never read) | AGREES |
| X7 | Frame table is direction-major, immediately after header | `dc6.c:199,254` | AGREES |
| X8 | Frame header 32 bytes: flip, w, h, offX, offY, unknown, nextBlock, length, data at +0x20 | `dc6.c:208-273` | AGREES |
| X9 | `flip` unread by the repo decoder | `dc6.c` | AGREES (guide says unverified in game) |
| X10 | `nextBlock` unused; use the offset table | `dc6.c` | AGREES |
| X11 | 3 slack bytes not read | `dc6.c` | AGREES |
| X12 | Opcodes `0x00-0x7F` copy, `0x80` EOL, `0x81-0xFF` skip | `dc6.c:68-84` | AGREES |
| X13 | No repeat/run opcode | `dc6.c:68-84` | AGREES |
| X14 | Scanline may end early; `0x80` advances unconditionally | `dc6.c:68-72` | AGREES |
| X15 | Rows stored bottom-up | `dc6.c:71,296-297`; `compose_render.c:170-172` | AGREES (repo derives it the same way) |
| X16 | Worked example `invjav.dc6` bytes | no data file in repo | NOT IN SOURCE |
| X17 | Validator requires 0xEEEEEEEE / ignores junk | not in repo | NOT IN SOURCE (repo does not validate these) |

### 3.4 DCC (`guide#dcc-the-format-that-has-to-be-unpacked`)

| # | Guide claim | Source | Verdict | Note |
|---|---|---|---|---|
| X18 | 15-byte header: signature, version, directions, fpd, tag, finalDc6Size, then offsets | `dcc.c:1463-1483` | AGREES (layout) | |
| X19 | Signature `0x74` is enforced | `dcc.c:1463` (read only) | **DIFFERS** | Repo does not check. Guide's enforcement is about D2CMP. |
| X20 | `finalDc6Size` not read by the game | `dcc.c:1478` (read, unused) | AGREES | Repo reads it, does not use it. |
| X21 | Direction offsets absolute from file start | `dcc.c:1278` | AGREES | |
| X22 | First direction starts at `15 + 4*dirs` | not checked | NOT IN SOURCE | Repo does not assert it. |
| X23 | `outsizeCoded` is a dword before the bitstream | `dcc.c:1286` | AGREES | Repo reads it as 32 bits from the stream. |
| X24 | 30 bits: 2-bit flag, then seven 4-bit width codes | `dcc.c:1293-1346` | AGREES | Order matches. |
| X25 | Width codes index the table `0,1,2,4,...,32` | `dcc.c:22-23` | AGREES | |
| X26 | Frame-header field order and signedness | `dcc.c:169-230` | AGREES | |
| X27 | `variable0` and `optionalBytes` always width code 0 in vanilla | n/a | NOT IN SOURCE | Corpus claim. Repo decodes optional data if present (`dcc.c:369-421`). |
| X28 | `bottomUp` always 0 in vanilla; DCC frames top-down | `dcc.c:237-248` | AGREES (for vanilla) | Repo supports `bottom_up = 1` in the box math only (`dcc.c:237-242`). Nothing flips rows. Behaviour for `bottom_up=1` is untested. |
| X29 | Four flag values occur in vanilla | n/a | NOT IN SOURCE | Corpus claim. |
| X30 | Bitstream order and 20-bit size fields | `dcc.c:275-317,427-480` | AGREES | |
| X31 | Size fields present by flag (bit1 → equalCell; bit0 → encoding + raw) | `dcc.c:275,293,433,455` | AGREES | |
| X32 | 256-bit key, ascending indices | `dcc.c:494-505` | AGREES | |
| X33 | Bitstreams are contiguous at bit granularity | `dcc.c:264-317` | AGREES | |
| X34 | pixelCode stream has no size, runs to direction end | `dcc.c:312-317` | AGREES | |
| X35 | Cells are 4×4, `ceil(w/4)`, `ceil(h/4)` | `dcc.c:526-532` | AGREES | |
| X36 | Frame grid phase-aligned to direction grid | `dcc.c:632,645` | AGREES | |
| X37 | Stage 1: never-written cell → mask `0xF`, consumes no bits | `dcc.c:882-883` | AGREES | |
| X38 | Stage 1: equalCell bit 1 → skip cell | `dcc.c:845-881` | AGREES (with note) | Repo gates the read on `equal_cell_bitstream_size != 0` (`dcc.c:847`), not on the flag bit. Same result for valid data. |
| X39 | pixelMask is 4 bits per changed cell | `dcc.c:865-866` | AGREES | |
| X40 | encodingType: 1 bit when a slot changed | `dcc.c:893-910` | AGREES | |
| X41 | encodingType 1: 8 raw bits | `dcc.c:916-928` | AGREES | |
| X42 | encodingType 0: nibble deltas, 15 continues | `dcc.c:944-960` | AGREES | |
| X43 | Zero displacement means stop | `dcc.c:962-966` | **DIFFERS (partial)** | Repo stops when the *code* equals the previous code. That applies to **both** encoding types, including raw (§5 item 3). Guide only states it for deltas. |
| X44 | Mapping of decoded codes to changed slots | `dcc.c:989-1001` | **NOT IN GUIDE** | Reverse order with zero fill. See §5 item 1. |
| X45 | Codes translated through the 256-bit key at end of stage 1 | `dcc.c:1012-1019` | AGREES | |
| X46 | Stage 2 bits per pixel from distinct-code count | `dcc.c:1149-1159` | AGREES | |
| X47 | Pixels read row-major, `bpp` bits each, from pixelCode stream | `dcc.c:1191-1212` | AGREES | |
| X48 | Flat cell costs zero pixel bits | `dcc.c:1149-1152` | AGREES | |
| X49 | Equal-cell in stage 2 copies from previous position | `dcc.c:1121-1143` | **NOT IN GUIDE** | Copies only if the buffer cell's last size matches; otherwise clears. |
| X50 | Game never holds a DCC frame as a bitmap; re-encodes to RLE | `dcc.c:1029-1058` | **DIFFERS** | Repo keeps frames as Allegro bitmaps and does not re-encode to RLE. Mechanism only; no format impact. |
| X51 | DCC has only a decoder in D2CMP, no encoder | n/a | NOT IN SOURCE | Repo has no encoder either. |
| X52 | Worked example `AMTRLITTN1HT.dcc` header and bit fields | no data file | NOT IN SOURCE | |

### 3.5 Corpus, version and "what is independent" sections

| # | Guide claim | Verdict |
|---|---|---|
| X53 | Corpus counts (21,692 DCC, 1,654 DC6, 3.3M frames, zero parse failures) | NOT IN SOURCE (no MPQ data in repo) |
| X54 | Byte-identical DC6/DCC header constants across patches | NOT IN SOURCE |
| X55 | Repo parsers are not independent of each other (both Siramy-derived) | AGREES (`dc6.c:1`, `dcc.c:1`, `NOTICE`) |
| X56 | Repo DCC parser validates no header constant | AGREES (§1.3, C1-C2) |
| X57 | Repo DC6 parser never reads `flip` | AGREES (X9) |

Two claims the guide did not make, both in the repo's favour:
- The repo's DC6 loader rejects any `version/flags/encoding` mismatch (`dc6.c:168`), which is stricter than
  the guide's "the game stores these but never checks them". Verdict: **DIFFERS**. Note it so an
  implementer does not copy the guide's leniency by accident.
- The repo's DC6 loader rejects `dc6_dir != cof->dir` (`dc6.c:175`). The guide does not describe this.

---

## 4. Implementer summary

- DC6 is trivially parseable: fixed 24-byte header, offset table, 32-byte frame headers, RLE with three opcodes.
  Use offsets, not `nextBlock`. Do not check the `termination` or slack bytes.
- Place DC6 and DCC frames by `offsetX` (left) and `offsetY` (bottom row) inside the union box of the direction.
- DCC needs a bit reader (LSB-first), the bit-width table, and the two-stage decode. Stage 1 slot order is
  not in the guide. Follow §2.3 (reverse order, zero fill), then verify against real files.
- Add the bounds checks in §2.5 that the reference lacks. The most important are C1, C2, C3, D2, D5, F1, F2.

---

## 5. Open points

1. **DCC stage-1 slot order (highest priority).** `dcc.c:989-1001` assigns decoded codes to changed slots in
   reverse order and zero-fills when the code list stops early. The guide says "read one code per changed slot"
   and does not describe mapping. The guide's D2CMP-based decoder claims full corpus decode, but the
   guide does not report slot order. Action: confirm against D2CMP disassembly or a known-good vanilla
   DCC round trip before finalising.
2. **Raw-mode stop rule.** `dcc.c:962-966` stops on equal consecutive codes for both encoding types. The
   guide documents it for delta mode only. Confirm.
3. **Initial `last_pixel` = 0** (`dcc.c:892`). With raw codes, a first code of 0 terminates the cell. Confirm
   this is intended.
4. **`bottom_up = 1`.** Repo does box math only. No vanilla data uses it. Decide the behaviour before accepting
   third-party DCC.
5. **Flip in DC6.** Unread by the repo and unverified in the game (`guide#dc6-...`). Leave unused until a
   sample shows it matters.
6. **Repo DC6 header rule.** Repo rejects header mismatches; guide says game stores them only. Pick one for
   the Rust implementation, and document the choice.
7. **Companion ledger not reconciled.** `sprite-formats-dcc-dc6.verification.md` was not checked line by line.
   Its claims on repo parsers (for example `dc6_header.c` allowing 1,024 directions) are noted there
   but not verified here. `src/core/dc6_header.c` was not read.
8. **Data not in repo.** No `.dc6`/`.dcc` samples are in the reference tree, so worked examples and corpus
   counts cannot be re-run here.
