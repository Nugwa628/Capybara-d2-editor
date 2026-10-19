# Robustness audit: library crates

The project brief says nothing in the library crates may call `exit` or panic on malformed game data; they must return errors instead. It also says no `unsafe` except in MPQ decompression hot paths, with a safety comment on each use. This audit covers every library crate:

- `d2-formats`
- `d2-assets`
- `capy-index`
- `capy-edit`
- `capy-history`
- `capy-render`
- `capy-sprite`
- `capy-paint`
- `capy-cli`

`capy-app` is out of scope.

## Method

The audit read all non-test code and looked for these patterns:

| pattern | what it covers |
|---|---|
| unwrap/expect | `unwrap` and `expect` calls |
| indexing | `[i]`, `[a..b]`, `split_at`, `copy_from_slice` |
| truncating cast | an `as` cast on a size or offset |
| overflow arithmetic | `+ - * <<` on file or user values |
| file-sized allocation | `with_capacity`, `vec![_; n]` or `reserve` sized from the file |
| panic macro/assert | panic macros and `assert!` |
| division by data | division or remainder by a file value |
| recursion | recursion whose depth the data controls |
| exit | `exit` calls |
| unsafe | `unsafe` blocks and functions |
| unbounded loop | CPU or iteration count set by a claimed size, e.g. a quadratic scan or a loop over a declared size the buffers don't back |

A finding is **reachable** when a value from a game file, a project or history file on disk, a database, a clipboard, a folder tree, or a user argument to the public API can trigger it. "Caller" in the reachable column means a public function takes an argument whose fields can disagree. One example is an image whose `width × height` doesn't match its buffer.

Two rules held for every fix:

- **Valid input behaves exactly as before.** The fixes use `get`, checked or saturating arithmetic, or i64 maths that only differs on out-of-range values. Allocation budgets sit well above anything the vanilla or Median XL corpora contain.
- **The public API is unchanged.** Errors use existing variants: `FormatError::Invalid`/`NotRepresentable`, `AssetError::Mpq`, `HistoryError::Index`/`Milestone`, and `SpriteError::Image`/`Invalid`.

Line numbers refer to the code after the fixes, on branch `lane/robustness`.

## Summary

- **No `unsafe`.** The workspace sets `unsafe_code = "deny"`, and no crate has an exception, including the MPQ decompression code.
- **No `process::exit`, `todo!`, `unreachable!` or `panic!` in library code.** The `capy-cli` example binary returns an `ExitCode`.
- **No `unwrap`, `expect` or `assert!` outside `#[cfg(test)]`.** Clippy's `unwrap_used` and `panic` lints are already on in the workspace.
- **The real hazards were of three kinds:**
  - integer overflow on file values, which panics in debug builds and wraps silently in release;
  - allocations or loops sized by a count the file claims but does not back;
  - one unbounded recursion over a folder tree.

| pattern | found | reachable | fixed |
|---|---:|---:|---:|
| overflow arithmetic | 143 | 45 | 45 |
| indexing | 106 | 0 | 0 (7 switched to `get` anyway) |
| file-sized allocation | 45 | 17 | 17 |
| truncating cast | 32 | 4 | 4 |
| division by data | 12 | 0 | 0 |
| unbounded loop (CPU) | 8 | 8 | 8 |
| recursion | 4 | 2 | 2 |
| unwrap/expect | 1 | 0 | 0 |
| panic macro/assert | 0 | 0 | 0 |
| exit | 0 | 0 | 0 |
| unsafe | 0 | 0 | 0 |
| **total** | **351** | **76** | **76** |

A row that groups several sites (for example "×7") counts each site.

**Reachable findings per crate:**

| crate | reachable |
|---|---:|
| d2-formats | 10 |
| d2-assets | 2 |
| capy-index | 1 |
| capy-edit | 1 |
| capy-history | 3 |
| capy-render, capy-sprite, capy-paint | 59 |
| capy-cli | 0 |

Most of the 59 in the render, sprite and paint crates are i32 screen-coordinate overflows on file offsets: debug panics and silent wraps in release.

**The most serious reachable findings, with what triggers them:**

| crate | finding | trigger |
|---|---|---|
| d2-formats | `tbl` string amplification: many nodes sharing one long string | a 400 KB file asks for about 2.6 GB |
| d2-formats | `txt` row padding | 2 MB of input pads to 10^12 cells |
| d2-formats | DT1 shared sub-block headers | about 1e6 blocks from 116 KB |
| d2-formats | DC6 shared frame headers | |
| d2-assets | ObjPreset `Act*150+Id` and MonPreset `Act-1` overflow | |
| capy-history | snapshot `size` from index.json used as a `Vec` capacity | capacity-overflow panic or OOM abort |
| capy-history | `next_id = u64::MAX` | |
| capy-history | unbounded milestone tree recursion | |
| capy-sprite | PNG/BMP/GIF/PCX decoders allocate the declared size before reading data | a tiny file claims a 4 GB image |
| capy-render | `RgbaImage::from_png` | same as above |
| capy-render | DT1 `i32::MIN` heights and offsets | |
| capy-render | render loops over a map size its grids don't back | |
| capy-edit | O(N²) anchor check when pasting a large clipboard | the editor freezes |

## Findings

Columns: crate | file:line | pattern | reachable from data? | fix. Rows marked "—" in the fix column needed no change.

### d2-formats: ds1, dt1, dc6, dcc, image, grid, blend

| crate | file:line | pattern | reachable from data? | fix |
|---|---|---|---|---|
| d2-formats | ds1.rs:636-650 | unbounded loop (CPU: paths × objects owner scan) | **yes**: both counts come from the file, bounded only by its length, so ~1e9+ comparisons on a few-MB file | objects counted per (x,y) in a HashMap first, which makes the check linear; same warnings |
| d2-formats | ds1.rs:917-924 | overflow arithmetic + file-sized allocation (`to_bytes` capacity `cells*4*layers`) | no: `check_fits` ties every grid to width×height; hardened anyway because the fields are public | checked_mul/add, falls back to capacity 0 (hint only) |
| d2-formats | ds1.rs:268-351 (Reader) | overflow arithmetic / indexing | no: `need()` checks first, everything goes through `get` | — |
| d2-formats | ds1.rs:414-549 (files/objects/groups/points reserve) | file-sized allocation | no: each is preceded by `r.need(n*rec)` | — |
| d2-formats | ds1.rs:437-471 (layer dwords w×h) | file-sized allocation | no: `cells*4` is checked against the remaining bytes before allocating | — |
| d2-formats | ds1.rs:569,655-678,710,1036 | overflow arithmetic / truncating cast ×4 | no: guarded comparison, i64 maths, clamp or `check_fits` | — |
| d2-formats | ds1.rs:686-770 (Ds1::new, add_*_layer) | file-sized allocation (Grid::new) | no: dims are caller-given, or come from a parsed model whose cells were in the file | — |
| d2-formats | grid.rs:17-21,84-95 | file-sized allocation (`vec![_; w*h]`, saturating) | no: caller-controlled (documented); parsers use `from_vec` | — |
| d2-formats | grid.rs:48 | overflow arithmetic (index) | no: x<w, y<h | — |
| d2-formats | image.rs:18-25 | file-sized allocation (IndexedImage::new) | no: every decoder caps dims first (DT1 2048², DC6 4096²/16 Mi, DCC 2048² + 32 Mi/file) | — |
| d2-formats | image.rs:33-36 | file-sized allocation (`to_rgba` capacity; `usize::MAX` → capacity-overflow panic) | no from data; **yes from caller** (public fields) | capacity hint is `min(len, pixels.len())*4`; output unchanged |
| d2-formats | image.rs:40 | indexing `colors[u8]` | no: u8 into a 256-entry array | — |
| d2-formats | blend.rs:67,73 | overflow arithmetic (u16) | no: max is 255·255+127 | — |
| d2-formats | dt1.rs:161-197 | overflow arithmetic / indexing / allocation (header, tile count) | no: count ≤ (len−table)/96 is checked first | — |
| d2-formats | dt1.rs:227-268 | file-sized allocation (**shared sub-block header amplification**) | **yes**: every tile can point at the same N zero-length headers, giving tiles×N `Block`s (~1e6 from a 116 KB file) because zero-length data used no budget | each header now charges 20 bytes plus its data against the len+1 MiB budget; the full corpus still parses |
| d2-formats | dt1.rs:316,328-350 | indexing / overflow arithmetic (iso split, RLE) | no: `split_at_checked`, `get`, sums bounded by data length | — |
| d2-formats | dt1.rs:355-395 (compose) | truncating cast / overflow arithmetic / allocation | no: extent ≤ 2048 checked, try_from, `get_mut` blit | — |
| d2-formats | dc6.rs:82-104 | overflow arithmetic / truncating cast (`len-24`, `total`) | no: length checked; `total*4 ≤ len` | `total` via `try_from` (cosmetic) |
| d2-formats | dc6.rs:104-136 | file-sized allocation (**shared frame-header amplification**, `with_capacity(total)`) | **yes**: 12× input, and zero-length frames sharing one header cost no budget | each header charges 32 bytes plus its data against the budget; capacity capped at budget/32; corpus still parses |
| d2-formats | dc6.rs:110-125 | overflow arithmetic (`off+o`, `off+32`) | no: after `slice_at` succeeded | — |
| d2-formats | dc6.rs:172-231 (render) | file-sized allocation / overflow arithmetic | no: ≤ 4096 per side and 16 Mi px; row in i64; `get_mut` | — |
| d2-formats | dc6.rs:253-301 (from_image) | overflow arithmetic / allocation | no from data (decoded images are capped); caller-built images cost their declared size | — (infallible public signature) |
| d2-formats | dc6.rs:340-397 (to_bytes) | truncating cast ×5 | no: total < 4 GiB checked first | — |
| d2-formats | dcc.rs:134-173 (Bits) | overflow arithmetic / shifts | no: `checked_add` against size, `get`, shifts < 64 | — |
| d2-formats | dcc.rs:200-257 | indexing / allocation (direction offsets) | no: ≤ 32 dirs and ≤ 1024 frames enforced, `get(start..end)` | — |
| d2-formats | dcc.rs:295-315 (`cell_spans`, public) | overflow arithmetic + allocation | **yes from caller input** (capy-paint passes image bounds): `i64::MIN` operands overflow; `frame_len` near i64::MAX gives a capacity-overflow panic | returns an empty Vec when abs(len) > 2^20 or rel+len overflows; unchanged inside that range |
| d2-formats | dcc.rs:323-354 (axis_cells) | overflow arithmetic / allocation | no: internal callers stay inside the 2048 box | doc note |
| d2-formats | dcc.rs:410-573 (read_direction) | overflow arithmetic / allocation / casts ×8 | no: i64 maths; box ≤ 2048 and pixel budget checked before any allocation | — |
| d2-formats | dcc.rs:576-688 (stage 1) | indexing / allocation / CPU | no: `get`, entries capped at 1 Mi, each cell consumes ≥ 1 bit | — |
| d2-formats | dcc.rs:699-775 (stage 2) | indexing / casts ×4 | no: `get_mut`, cells ≤ 5×5 | — |
| d2-formats | dcc.rs:807,816-821 | overflow arithmetic / truncating cast | no: pos ≤ size; dims ≤ 2048 | — |
| d2-formats | dcc/encode.rs:137-251 | overflow arithmetic / casts / allocation | no: box ≤ 2048 checked, `checked_mul`, buffer length validated | — |
| d2-formats | dcc/encode.rs:405-428 (to_bytes header) | truncating cast ×3 (silent wrap → corrupt file) | not a panic; reachable only with > 4 GiB in memory | `try_from`; dir-count byte and DC6-size hint saturate; direction offset > 4 GiB returns `NotRepresentable` |
| d2-formats | dcc/encode.rs:66-101,443-867 | shifts / indexing ×10 | no: take ≤ 8, n < 32, fixed-array indices | — |

### d2-formats: cof, palette, animdata, txt, tbl, tables, excel, bytes

| crate | file:line | pattern | reachable from data? | fix |
|---|---|---|---|---|
| d2-formats | tbl.rs:134-158 | file-sized allocation (string amplification) | **yes**: many active nodes can point their key/text offsets at one long NUL-terminated string; each node copies it, so a 400 KB file can ask for about 2.6 GB, plus O(nodes×len) scanning | running total of key+text bytes capped at 8×file length + 64 KiB, over it returns `Invalid{what:"string data"}`; shipped tables reach at most 0.78× (every vanilla and mxl lng tbl measured; no offsets are shared) |
| d2-formats | tbl.rs:162-165 | overflow arithmetic | yes in principle: `slot + node_count - home` in u32 overflows when node_count > 2^31, which needs a file over 36 GB | computed in u64, same result |
| d2-formats | txt.rs:42-60 | file-sized allocation (row padding) | **yes**: a header of 1M tabs followed by 1M blank lines (about 2 MB) pads to 10^12 `String`s and aborts on OOM | padding cells capped at 4×input bytes + 2^20, over it returns `Invalid{format:"txt",what:"row widths"}`; the corpus maximum is 2,333 cells |
| d2-formats | tbl.rs:118-141 | indexing / overflow arithmetic | no: id_count is u16, and the node range is checked in u64 against the length first | — |
| d2-formats | tbl.rs:145-146 | truncating cast (u32 as usize) | no: lossless on 32/64-bit targets, and the offset goes through `get(off..)` | — |
| d2-formats | tbl.rs:165 | division by data (`% n`) | no: n > 0 inside `for slot in 0..node_count` | — |
| d2-formats | cof.rs:151-153 | overflow arithmetic | no: the three counts are u8, so at most 28+255×9; the subtraction only runs on the error path, where records_end > len | — |
| d2-formats | cof.rs:173-183 | file-sized allocation / overflow arithmetic | no: want ≤ 255³+255, and `missing ≤ 4096` is checked before allocating | — |
| d2-formats | cof.rs:185-186 | truncating cast | no: the values came from u8 | — |
| d2-formats | cof.rs:210 | overflow arithmetic | no: row ≤ 65,535 and width = layers.len() | — |
| d2-formats | animdata.rs:60,99 | overflow arithmetic | no: `pos+4` only after a successful 4-byte read; `pos += block.len()` stays within the slice | — |
| d2-formats | animdata.rs:67-75 | file-sized allocation | no: `checked_mul` plus `slice_at` must succeed first | — |
| d2-formats | animdata.rs:90 | indexing (copy_from_slice) | no: `get(16..160)` is always 144 bytes | — |
| d2-formats | palette.rs:86,185 | overflow arithmetic | no: guarded by `min(768)` and the `< MIN_PL2_LEN` check | — |
| d2-formats | palette.rs:113,166-170 | indexing ×2 | no: u8 index into `[u8;256]`, constant offsets, and `slice_at` gives exactly 256 bytes | — |
| d2-formats | excel.rs:274,876 | file-sized allocation | no: `record_count` requires len == 4+count×size exactly | — |
| d2-formats | excel.rs:299,319 | overflow arithmetic (shift) | no: width ≤ 4 and bit < 29 are layout constants | — |
| d2-formats | excel.rs:264-265,366,881 | overflow arithmetic / truncating cast | no: layout constants; count fits in u32 | — |
| d2-formats | excel.rs:275 | division by data (`chunks_exact`) | no: rec is a non-zero constant | — |
| d2-formats | tables.rs:183,213,288 | overflow arithmetic / truncating cast / shift | no: `saturating_sub`; the range is checked before `as u32`; `slot < 32` is checked before the shift | — |
| d2-formats | txt.rs split_lines, excel decode | file-sized allocation (linear) | no: proportional to input only (about 24-48 B per txt byte, about 12× per excel record), not to a header count | left as is |
| d2-formats | bytes.rs:3-29 | indexing / overflow arithmetic | no: `checked_add` and `get` | — |

### d2-assets

| crate | file:line | pattern | reachable from data? | fix |
|---|---|---|---|---|
| d2-assets | objects.rs:409-415 | overflow arithmetic | **yes**: `Act * 150 + Id` on i64 values parsed from ObjPreset cells (e.g. Act = i64::MAX); reproduced by the new fuzz test as "attempt to multiply with overflow" | `checked_mul`/`checked_add`; on overflow the row gets no slot (index -1) |
| d2-assets | objects.rs:532 | overflow arithmetic | **yes**: MonPreset `Act` = i64::MIN makes `a - 1` overflow | `a.checked_sub(1)?` |
| d2-assets | presets.rs:222-223,252,265-266 | overflow arithmetic (u32 cell counts, `sum`) | no in practice: needs more than 4G DS1 cells (a DS1 over 16 GiB) | saturating add / `fold` (same results) |
| d2-assets | mpq/mod.rs:140-147 | overflow arithmetic (`offset + done`) | no: a read near `u64::MAX` returns 0 bytes first | `checked_add` → UnexpectedEof I/O error |
| d2-assets | mpq/mod.rs:170 | overflow arithmetic (`file_len - archive_offset`) | no: `find_header` guarantees offset + 4 ≤ len | `saturating_sub` |
| d2-assets | mpq/mod.rs:219-221 | truncating cast (`size as usize`) before allocation | no on 64-bit; a table over 4 GiB would truncate on 32-bit | `usize::try_from`, else `AssetError::Mpq` |
| d2-assets | mpq/mod.rs:425 | overflow arithmetic (`start + from`) | no: start ≤ 2^48 + len, from is u32 | `saturating_add` |
| d2-assets | source.rs:218 | overflow arithmetic (sum of file sizes) | no: needs a folder over 16 EiB | saturating `fold` |
| d2-assets | mpq/mod.rs:176,178 | file-sized allocation / truncating cast | no: capped at `HEADER_MAX` (0xD0); header size is range-checked | — |
| d2-assets | mpq/mod.rs:192 | overflow arithmetic (`512 << shift`) | no: shift > 16 rejected first | — |
| d2-assets | mpq/mod.rs:200-224 | file-sized allocation (hash/block/hi-block tables) | no: `checked_mul`/`checked_add` and `end ≤ file_len` before `vec!` | — |
| d2-assets | mpq/mod.rs:268 | overflow arithmetic (`i * 2`) | no: i < entries ≤ len/16 | — |
| d2-assets | mpq/mod.rs:408-412,429 | indexing ×4 | no: constant indexes into fixed arrays | — |
| d2-assets | mpq/mod.rs:492-523 | file-sized allocation (`vec![0; csize]`) | no: file_size ≤ 1 GiB, csize ≤ 2 GiB, csize·65536+4096 ≥ file_size, extent ≤ len are checked first | — |
| d2-assets | mpq/mod.rs:537-601 | truncating cast / overflow arithmetic / division ×7 | no: file_size ≤ 1 GiB; sector_size ≥ 512; i < sectors; `from ≤ to` checked | — |
| d2-assets | mpq/mod.rs:554 | file-sized allocation (`with_capacity`) | no: capped at 64 MiB, capacity only | — |
| d2-assets | mpq/mod.rs:572 | truncating cast (u32 → usize) | no: lossless on 32/64-bit | — |
| d2-assets | mpq/mod.rs:635 | unwrap/expect (`unwrap_or`) | no: `rsplit` always yields one item | — |
| d2-assets | mpq/mod.rs:640-672 (`find_header`) | overflow arithmetic ×3 | no: u64 sums of an offset, a u32 and a 64K chunk | — |
| d2-assets | mpq/crypto.rs:21-26 | indexing / division | no: index ≤ 0x4FF < 0x500; constant divisor | — |
| d2-assets | mpq/crypto.rs:50,66,69 | indexing / shift | no: `get`; constant shifts; wrapping ops already explicit | — |
| d2-assets | mpq/decompress.rs:38,66,83 | indexing / file-sized allocation | no: `split_first`; capacity ≤ 1 MiB; output limited by `take(expected + 1)` | — |
| d2-assets | objects.rs:406 | overflow arithmetic | no: constants | — |
| d2-assets | objects.rs:544,638,719 | overflow arithmetic ×3 | no: row counter; `index > 0` guard; `base ≤ id` | — |
| d2-assets | objects.rs:849,879,895,1265,1286 | overflow arithmetic ×5 | no: act is clamped to 0..=4 in `resolve`/`placeable` | — |
| d2-assets | objects.rs:955-1167 | overflow arithmetic / division ×6 | no: i64 sums of i32 values; act ≤ 4; constant divisor | — |
| d2-assets | objects.rs:1082 | truncating cast (u32 → usize) | no: lossless; lookup via `get` | — |
| d2-assets | presets.rs:125,166,347,366-367,454,804 | overflow arithmetic / indexing ×6 | no: line counter; `split_at` on a `find` boundary; `get`; digit in '1'..='5'; `try_from`; bounded | — |
| d2-assets | path.rs:46-47 | indexing (`bytes[0]`, `bytes[1]`, `&s[2..]`) | no: `len >= 2` guard; byte 1 is ASCII ':' so 2 is a char boundary | — |
| d2-assets | profile.rs:93-102 (glob) | overflow arithmetic ×4 | no: bounded by pattern/text lengths | — |
| d2-assets | names.rs:82,269,314; source.rs:45,114,220 | overflow arithmetic / truncating cast ×6 | no: counters; `try_from`; usize → u64 | — |
| d2-assets | whole crate | panic macro/assert, exit, unsafe, recursion | none in non-test code; `FolderSource::walk` uses an explicit stack | — |

### capy-index, capy-edit, capy-history

| crate | file:line | pattern | reachable from data? | fix |
|---|---|---|---|---|
| capy-history | store.rs:182-188 | file-sized allocation | **yes**: `Vec::with_capacity(snap.size)` with `size` from index.json; `u64::MAX` panics (capacity overflow), `1<<40` aborts | capacity and `take()` bound capped at min(size, compressed_len×1032+64, 64 MiB) (deflate's maximum ratio is ~1032:1) |
| capy-history | store.rs:267 | overflow arithmetic | **yes**: `next_id += 1` when index.json has `next_id = u64::MAX` (debug panic, release wraps to 0) | `checked_add` before the blob is stored → `HistoryError::Index`; rollback restores the old id |
| capy-history | milestone.rs:125,143 | recursion | **yes**: `write_tree` recursed once per folder level of the saves folder on disk, without limit | depth capped at `MAX_TREE_DEPTH = 64` → `HistoryError::Milestone` (saves are ~7 levels deep) |
| capy-history | store.rs:376 | indexing (`split_at`) | no: the hash is validated as 64 ASCII hex on open, or computed by us | hardened with a char-boundary check |
| capy-history | store.rs:307,360; atomic.rs:69 | overflow arithmetic ×3 | no: snapshot counts, sums of real file sizes, retry counter ≤ 8 | — |
| capy-edit | document/objects.rs:146-175 | indexing ×2 + quadratic loop | indexing no (`0..len`); **the O(N²) loop was reachable**: pasting a JSON clipboard with N objects froze the editor | objects and paths grouped per sub-tile in one pass (linear); identical output, no indexing |
| capy-edit | document/paths.rs:111,120,193,227 | indexing ×3 / `Vec::insert` | no: `pos` from `position()`; `at <= len` checked | — |
| capy-edit | patch.rs:398-487 | indexing (`Vec` insert/remove) ×8 | no: each guarded by the index checks just before it | — |
| capy-edit | selection.rs:265 | indexing (`skip(n-1)`) | no: n ≥ 3 checked | — |
| capy-edit | document/clip.rs:120,124,217 | overflow arithmetic ×3 | no: x1 ≥ x0 and x1 < width; `x ≥ rect.x0` by construction | — |
| capy-edit | document/clip.rs:250-330 | overflow arithmetic | no: i64 on i32/u32 inputs, then `try_from` | — |
| capy-edit | selection.rs:217; patch.rs:242; edits.rs:281 | overflow arithmetic ×3 | no: zero width checked; k < n; u64 product | — |
| capy-edit | document.rs:96,110,154,209 | overflow arithmetic ×4 | no: needs 2^32 or 2^64 UI calls | — |
| capy-edit | selection.rs:250-253; edits.rs:351 | truncating cast ×5 | no: f64→u32 `as` saturates and input is checked finite; `&0xFF` then `get` | — |
| capy-edit | patch.rs:36-40; objects.rs:255 | file-sized allocation ×2 | no: a parsed DS1's size is backed by grids already read; resize capped at `MAX_CELLS`; indices checked | — |
| capy-edit | clipboard.rs:184 | recursion (JSON nesting) | no: serde_json's recursion limit is 128 | — |
| capy-index | tiles.rs:430 | overflow arithmetic | **yes**: `w*h*4` on file-derived tile sizes overflowed usize (e.g. u32::MAX², debug panic) | `checked_mul` → `None` |
| capy-index | build.rs:281 | overflow arithmetic | no in practice: needs > 2^32 cells with one key in one DS1 | `saturating_add` |
| capy-index | tiles.rs:188-263,448; build.rs:757,771 | overflow arithmetic ×7 | no: bounded by file size; i64 on i32/u32; u64 sums | — |
| capy-index | tiles.rs:442-465 | indexing ×5 | no: after the length check; luma plane sized (w+2)(h+2) | — |
| capy-index | tiles.rs:518-525 | indexing ×2, truncating cast ×3 | no: tw, th ≤ 32 and `x*tw/w < tw` | — |
| capy-index | tiles.rs:573-574 | truncating cast ×2 | no: values ≤ 96 | — |
| capy-index | tiles.rs:435 | file-sized allocation | no: bounded by the supplied RGBA buffer length | — |
| capy-index | tiles.rs:473,478,521-522 | division by data ×4 | no: `opaque > 0` and w, h ≥ 1 checked | — |
| capy-index | build.rs:307 | division by data ×2 | no: width 0 skipped | — |
| capy-index | build.rs:218,565,972 | indexing ×3 | no: internal `LayerId` / index-map invariants | switched to `get()` anyway |
| capy-index | naming.rs:199 | indexing ×2 | no: `len >= 2` checked | — |
| capy-index | query.rs:279-345 | indexing (str slicing) ×12 | no: every offset is a char boundary (advance by `len_utf8` or an ASCII delimiter); proptest confirms | — |
| capy-index | par.rs:46 | indexing | no: `items.get` | — |
| capy-index | index.rs:102-141,345-376; build.rs; user.rs; search.rs:288 | SQLite reads (truncating cast) | no: `opt_int` uses `try_from`; typed `get` returns a rusqlite error for NULL/out-of-range/wrong type | — (DB-corruption fuzz confirms) |

### capy-render, capy-sprite, capy-paint, capy-cli

| crate | file:line | pattern | reachable from data? | fix |
|---|---|---|---|---|
| capy-sprite | image_io.rs:46-77 | file-sized allocation | **yes**: PNG/BMP/GIF decoders allocate the declared size before reading data; no `image::Limits` were set | header probe (`into_dimensions`) + `Limits` (side ≤ 16384) + `check_size` before decoding |
| capy-sprite | image_io.rs:172 | file-sized allocation | **yes**: `with_capacity(planes*per_line*height)` reached 4.3 GB from a 128-byte PCX header | capacity capped at `bytes.len()*32` (an RLE pair yields ≤ 63 bytes) |
| capy-sprite | image_io.rs:151-160 | overflow arithmetic ×3 | no: u16 fields checked against MAX_SIDE first | — |
| capy-sprite | image_io.rs:113,213,233 | indexing ×3 | no: fixed-size slices, u8 palette index | — |
| capy-sprite | image_io.rs:241 | overflow arithmetic (shift) | no: bpp ∈ {1,2,4} | — |
| capy-sprite | model.rs:205-220 | overflow arithmetic | **yes, caller frames** whose fields disagree with the buffer (`(y*w+x)*4`) | checked `byte_index` |
| capy-sprite | model.rs:113-125 | file-sized allocation / unbounded loop | **yes, caller frames**: encoders allocate or loop over declared sizes (2 s per empty 0×2^30 DC6 frame) | `check_shape` rejects frames claiming more pixels than their buffers hold or a side > 2^24 |
| capy-sprite | model.rs:101-102; cof.rs:158; quantize.rs:141 | indexing ×4 | no: `[_;256]` indexed by u8 | — |
| capy-sprite | model.rs:178 `RgbaFrame::new` | file-sized allocation | no: size is a caller argument; internal callers bounded | left |
| capy-sprite | cof.rs:138,147,150 | truncating cast / overflow arithmetic ×3 | no: union ≤ 2^26 checked first | — |
| capy-sprite | ops.rs:46-80 `split_frame` | overflow arithmetic + file-sized allocation | **yes** (user tile size, caller frame): `x += tile_w` overflowed | `checked_add`; frames with lying sizes returned unchanged |
| capy-sprite | ops.rs:103 | file-sized allocation | **yes**: `vec![0; columns]` from a user u32 | clamped to `frames.len()` (identical output) |
| capy-sprite | ops.rs:97,135 | file-sized allocation / unbounded loop | **yes, caller frames** | lying frames are an error; zero-width frames skipped |
| capy-sprite | ops.rs:167 `trim_borders` | unbounded loop | **yes, caller frames** | scan bounded by buffer length |
| capy-sprite | ops.rs:196-197 `center_offset` | truncating cast / overflow arithmetic | **yes**: height > 2^31 wraps to i32::MIN, negation panics | `saturating_sub_unsigned` |
| capy-sprite | quantize.rs:97-104 | file-sized allocation | **yes, caller frames** (a 4e9×0 frame allocated a 48 GB error buffer) | lying frames > 2^26 px give an empty image; zero area returns early |
| capy-sprite | quantize.rs:169 `to_rgba` | file-sized allocation | **yes, caller frames** | same guard |
| capy-sprite | transfer.rs:431-475 `export_sheet` | overflow arithmetic ×4 | **yes**: DC6/DCC offsets (`max_x - min_x` in i32, empty frames' offsets) | i64 spans, saturating maths, empty frames skipped, zero cells rejected |
| capy-sprite | transfer.rs:388,543 | file-sized allocation | no: capped counts / `height % directions == 0` | empty-sheet guard added at :521 |
| capy-sprite | transfer.rs:100-112 | truncating cast ×3 | no: counts of in-memory vectors | — |
| capy-sprite | batch.rs:142-157 `scan` | recursion | **yes**: user folder tree depth | `MAX_DEPTH` 256 → error |
| capy-render | tileset.rs:112-146 `TileGeometry::of` | overflow arithmetic ×3 | **yes**: DT1 i32 height (`-i32::MIN`, `+32`) | i64, saturated; a height outside i32 gives an empty tile, as release builds did |
| capy-render | tileset.rs:180-187 `bitmap_top` | overflow arithmetic | **yes**: zero_line can be i32::MIN | saturating |
| capy-render | tileset.rs:450-490 `build_image` | overflow arithmetic / indexing | no: i64 maths, `get` | — |
| capy-render | compose.rs:122 RgbaImage get/set | overflow arithmetic | **yes, caller images** with lying fields | checked index |
| capy-render | compose.rs:154-158 `from_png` | file-sized allocation | **yes**: a 50-byte PNG could ask for 40 GB | capped at `MAX_CANVAS_BYTES` (1 GiB) |
| capy-render | compose.rs:210-245 `cell_corner` | overflow arithmetic | **yes**: huge maps (the `i32::MAX/4` fallback itself overflowed) | i64, saturated to i32 |
| capy-render | compose.rs:267; order.rs:200 | unbounded loop | **yes**: a `Ds1` whose size fields exceed its grids (1e6 × 1e6) | loops bounded by the largest layer grid (`cell_extent`) |
| capy-render | compose.rs:276-285,388-395,453-456 | overflow arithmetic ×6 | **yes** | i64 / saturating; the existing > 1 GB canvas error stays |
| capy-render | compose.rs:502-515 `draw_object` | overflow arithmetic ×3 | **yes**: Objects.txt offsets, empty frames' offsets | saturating |
| capy-render | compose.rs:571-590 `blit_pixels` | unbounded loop + overflow arithmetic ×4 | **yes**: loop ran over the declared image size; offsets come from files | rows bounded by `alpha.len()/w` (identical for consistent images); saturating |
| capy-render | compose.rs:594,603; objects.rs:386 | indexing ×3 | no: `get` / `[_;256]` by u8 | — |
| capy-render | compose.rs:640 `downscale`, `filled` | overflow arithmetic / allocation | no: ≤ width; size is a caller argument | left |
| capy-render | compare.rs:138-157 | overflow arithmetic / unbounded loop | **yes, user `max_shift`** (i32::MAX panicked on `cost`, or looped 2^64 times) | clamped to the images' reach (same best shift); saturating cost |
| capy-render | objects.rs:193,203,233 | overflow arithmetic ×3 | **yes**: DCC/DC6 and Objects.txt offsets, empty layers at any offset | i64; empty layers skipped; saturating |
| capy-render | objects.rs:262-266 `object_anchor` | overflow arithmetic ×2 | **yes**: DS1 object x/y (public fn) | saturating |
| capy-render | objects.rs:250,658 | division by data / overflow | no: `count.max(1)`, guarded `!= 0` | — |
| capy-render | objects.rs sort_key | truncating cast | no: `rem_euclid(5)` | — |
| capy-render | order.rs:402-414 | overflow arithmetic ×3 | **yes** | saturating |
| capy-render | order.rs:534-539; palette.rs:30-37 | indexing ×6 | no: loop bounds; length ≥ 5 checked | — |
| capy-paint | mask.rs:99 `rect` | truncating cast + overflow arithmetic | **yes, user size ≥ 2^31** (`MIN - 1` panicked) | `try_from` and clip |
| capy-paint | mask.rs:190 grow/shrink | truncating cast | **yes**: radius could become i32::MIN, then `-r` panicked | radius clamped |
| capy-paint | mask.rs:203 | overflow arithmetic | **yes, caller** | saturating |
| capy-paint | mask.rs:125 lasso/bounds | indexing / division ×3 | no: division only when yi ≠ yj | — |
| capy-paint | mask.rs `none`/`all` | file-sized allocation | no: size is a caller argument | left |
| capy-paint | check.rs:95-110 `value` | overflow arithmetic / negative cast | **yes, caller frames** | checked `index()` |
| capy-paint | check.rs:241-250 `reduce_cell` | indexing ×2 | no: session frames are consistent | `get` anyway |
| capy-paint | session.rs:466-468 `commit_image` | indexing | no: internal invariant | alpha-length check added to `same_shape` |
| capy-paint | session.rs:384,473,487 | overflow arithmetic / indexing ×3 | no: invariants | — |
| capy-paint | export.rs:128,180 | unbounded loop ×2 | **yes, caller frames** (empty frame claiming 4e9 rows) | empty frames skipped |
| capy-paint | export.rs:138,191 | indexing ×2 | no: inside the union box | `get_mut` anyway |
| capy-paint | tools/* (`at()`, frames.rs, effects.rs, select.rs) | indexing ×9 | no: in-frame checks, MAX_SIDE 8192, equal frame counts, rgba length checked | — |
| capy-cli | args.rs:76,87,145; render_cmd.rs:43,173 | indexing / overflow arithmetic ×5 | no: `first()`, loop bounds, `position`, `inside ≤ len` | — |
| capy-cli | overrides.rs `toml::from_str` | recursion | no: the toml crate's recursion limit returns an error for 20,000-deep nesting | test added |
| capy-render, capy-sprite, capy-paint, capy-cli | — | exit, unwrap/expect, panic macro/assert, unsafe | none in library code (`examples/capy.rs` returns `ExitCode`, not `process::exit`) | — |

## Fuzz-style tests

Every parser entry point has property or mutation tests asserting that no input panics. Each test feeds:

- random bytes;
- truncated and bit-flipped synthetic valid files;
- hostile header values (counts, offsets and sizes at 0, ±1 and the integer extremes);
- private-corpus seeds where they help. These tests skip when `testdata/private` is absent.

Committed seeds are all synthetic.

| entry point | tests (file) | inputs |
|---|---|---|
| `Ds1::parse` | `fuzz_random_bytes_up_to_4k`, `fuzz_plausible_header_random_body`, `fuzz_mutated_synthetic_files`, `fuzz_truncated_synthetic_files`, `fuzz_hostile_dimensions_and_counts`, `fuzz_npc_paths_against_many_objects_stay_linear`, `fuzz_extreme_models_error_instead_of_panicking`, `fuzz_corpus_ds1_mutations` (`d2-formats/src/ds1/tests/fuzz.rs`) | every gate version and tag type; round trip `to_bytes`; re-encode at 10 versions |
| `Dt1::parse` | `fuzz_random_bytes_up_to_8k`, `fuzz_mutated_synthetic_files`, `fuzz_truncated_synthetic_files`, `fuzz_header_fields_take_nasty_values`, `fuzz_shared_block_headers_cannot_amplify`, `fuzz_corpus_dt1_mutations` (`dt1/tests/fuzz.rs`) | renders every tile, also with block formats swapped |
| `Dc6::parse` | `fuzz_random_bytes_up_to_8k`, `fuzz_mutated_synthetic_files`, `fuzz_truncated_synthetic_files`, `fuzz_frame_table_into_random_body`, `fuzz_rendering_extreme_frame_fields`, `fuzz_shared_frame_headers_cannot_amplify`, `fuzz_encoder_on_extreme_models`, `fuzz_corpus_dc6_mutations` (`dc6/tests/fuzz.rs`) | render, then encode → parse round trip |
| `Dcc::parse` | `fuzz_random_bytes_up_to_8k`, `fuzz_mutated_synthetic_files`, `fuzz_truncated_synthetic_files`, `fuzz_cell_spans_any_i64`, `fuzz_encoder_on_odd_frames`, `fuzz_encoder_rejects_extreme_frames`, `fuzz_cell_spans_extremes_do_not_overflow_or_allocate`, `fuzz_corpus_dcc_mutations` (`dcc/tests/fuzz.rs`), plus the existing `truncation_and_bit_flips_never_panic` and `corpus_dcc_mutations_never_panic` | re-encode small decodes |
| `Cof::parse` | `cof_mutated_valid_never_panics`, `cof_any_header_with_long_tail_never_panics`, `corpus_cof_mutations_never_panic` (`d2-formats/src/robustness.rs`) | also runs `draw_order` and `component_layer` |
| `Pl2::parse`, `Palette::parse_dat` | `palette_dat_random_and_truncated_never_panic`, `pl2_truncated_and_mutated_never_panics`, `corpus_palette_mutations_never_panic` (`robustness.rs`) | every cut length of a full PL2 |
| `AnimData::parse` | `animdata_mutated_valid_never_panics`, `animdata_plausible_counts_never_panic`, `corpus_animdata_mutations_never_panic` (`robustness.rs`) | |
| `TxtTable::parse` | `txt_random_bytes_long_never_panic`, `txt_mutated_valid_never_panics`, `txt_padding_bomb_is_rejected`, `corpus_txt_mutations_never_panic`, `typed_tables_on_weird_cells_never_panic` (`robustness.rs`) | the typed table readers run on weird cells |
| `excel::decode_bin` (every kind and version), `decode_levels_bin`, `detect_version` | `excel_random_bytes_longer_never_panic`, `excel_counts_against_body_never_panic`, `excel_mutated_valid_never_panics`, `corpus_excel_bin_mutations_never_panic` (`robustness.rs`) | |
| `StringTable::parse` | `tbl_mutated_valid_never_panics`, `tbl_offsets_pointing_anywhere_never_panic`, `tbl_shared_string_bomb_is_rejected`, `corpus_tbl_mutations_never_panic` (`robustness.rs`) | |
| all d2-formats tables | `truncations_of_valid_files_never_panic` (`robustness.rs`) | every prefix of every synthetic seed |
| `MpqArchive::open`/`read` | `mutated_archives_never_panic`, `random_block_entries_never_panic`, `random_bytes_never_panic`, `extreme_field_values_never_panic`, `absurd_claims_in_a_tiny_archive_are_errors` (`d2-assets/tests/fuzz_mpq.rs`) | v0/v1/v3 seeds with zlib, bzip2, implode, encryption and sector tables; open, then `listfile`, `read`, `compression_mask`, `block_info` and `coverage` |
| `parse_legacy_ini`, `Profile::from_toml`, `GamePath` | `d2-assets/tests/fuzz_text.rs`, including `legacy_ini_private_corpus_mutations` | |
| object and map catalogs | `object_catalog_extreme_cells_never_panic`, `map_catalog_extreme_cells_never_panic`, `private_corpus_tables_with_out_of_range_ids` (`d2-assets/tests/fuzz_tables.rs`) | |
| `parse_query` | `parse_query_never_panics_on_any_text`, `parse_query_never_panics_on_any_chars`, `parse_query_never_panics_on_token_soup` (`capy-index/src/query.rs`) | |
| index.db, user.db, tile pixels | `damaged_rows_in_index_and_user_db_never_panic`, `damaged_database_files_never_panic` (`capy-index/src/tests/robustness.rs`), `pixel_pipeline_never_panics` (`tiles.rs`) | |
| `Clipboard::from_json` and paste | `clipboard_from_any_text`, `clipboard_from_damaged_json_bytes`, `clipboard_from_damaged_json_values`, `arbitrary_clipboards_paste_safely`, `extreme_arguments_on_synthetic_maps`, `edits_on_weird_but_accepted_maps`, `edits_on_damaged_corpus_maps` (`capy-edit/tests/robustness.rs`) | |
| history store | `corrupt_index_json_bytes`, `corrupt_index_json_values`, `huge_sizes_and_ids_in_index_are_errors`, `corrupt_blobs_are_reported_corrupt`, `corrupt_milestone_repository`, `deeply_nested_saves_folder_is_an_error` (`capy-history/tests/corrupt.rs`) | |
| `capy_sprite::import_image` (PNG/BMP/GIF/PCX) | `import_random_bytes_never_panic`, `import_random_bodies_never_panic`, `import_mutated_images_never_panic`, `huge_declared_sizes_are_errors`, `import_mutated_corpus_frames_never_panic` (`capy-sprite/src/tests/fuzz_tests.rs`) | plus the frame operations on frames whose sizes lie |
| render | `render_survives_hostile_tiles_and_maps`, `from_png_survives_mutations`, `lying_map_size_does_not_hang`, `mutated_corpus_tiles_render_without_panics` and others (`capy-render/src/compose/fuzz_tests.rs`) | |
| paint, CLI | `capy-paint/tests/fuzz.rs`; `parse_takes_any_arguments`, `overrides_take_any_text`, `deeply_nested_toml_is_not_a_stack_overflow` (`capy-cli/src/fuzz_tests.rs`) | |

**Knobs:**

- Every proptest follows `PROPTEST_CASES`. The default is 256, or a smaller per-test default for slow tests; nothing hard-codes a higher count.
- Hand-rolled corpus and mutation loops read `CAPY_FUZZ_ITERS` and print their iteration counts.

### Raised-count run

The command was `PROPTEST_CASES=5000 CARGO_BUILD_JOBS=4 cargo test --release --no-fail-fast --workspace --exclude capy-app`, run on 2026-10-19. It took 8 minutes including the build.

**Result: 800 tests passed, 0 failed.** One test is ignored; it was already ignored before this work. Every proptest ran 5000 cases.

Counts the tests printed:

| test | count |
|---|---|
| `mutated_archives_never_panic` | 5000 cases: 3145 opened, 15,935 reads OK, 2,659 read errors |
| `random_block_entries_never_panic` | 5000 cases: 4251 opened, 21,286 reads OK, 1,917 read errors |
| `random_bytes_never_panic` (MPQ) | 5000 cases: 159 opened |
| `extreme_field_values_never_panic` | 8296 cases: 7668 opened, 65,731 reads OK, 3,886 read errors |
| `legacy_ini_private_corpus_mutations` | 40 files × 5000 mutations |
| `object_catalog_extreme_cells_never_panic` | 5000 cases |
| `map_catalog_extreme_cells_never_panic` | 5000 cases |

Corpus mutation loops, at the default `CAPY_FUZZ_ITERS`:

| format | seeds × mutations |
|---|---|
| DS1 | 48 × 40 |
| DT1 | 32 × 24 |
| DC6 | 48 × 40 |
| DCC | 32 × 16 |
| COF | 40 × 50 |
| animdata | 2 × 40 |
| pal.dat | 6 × 30 |
| pal.pl2 | 2 × 10 |
| txt | 12 × 20 |
| tbl | 4 × 10 |
| excel bin | 16 × 10 |
| sprite import | 960 mutated images (40 per file and kind) |
| render | 30 mutated tile sets |
| capy-edit | 40 damaged maps (22 parsed and were edited) |

The capy-history corruption loops ran 200 iterations each, and the capy-index database loops ran 30.

**Pre-existing test bug found by this run (not a panic in library code).** At 5000 cases, `capy-edit/tests/properties.rs::needs_version_is_the_least_sufficient_version` failed for seed 17807363844412858865, and it fails the same way on main's code.

- `paste` documents that it checks NPC-path anchor conflicts before the file version. So a paste refused for an anchor can still have a preview that names a needed version.
- The property asserted `None` in that case.
- The property now accepts that case; commit `test(capy-edit): allow anchor refusals …`.

Shake-out runs during the work, in debug with `PROPTEST_CASES=2000`, `CAPY_FUZZ_ITERS` 300-2000 and up to 20,000 MPQ cases, found the reachable cases fixed above. No panics remain.

## Oracle results unchanged

Both release oracle tests were run before any change and again after the last one:

- `CAPY_RENDER_TESTS=1 cargo test --release -p capy-render`
- `cargo test --release -p capy-edit --test oracle_load`

After dropping timings, the printed tables are identical: 74 map rows, plus the per-map object and coverage lines. The only differences are the new fuzz tests in the test list.

## Open items

- **Limits that can reject legitimate input.** These are new caps, all far above any shipped file:
  - `TxtTable::parse` can now return `Err`, for more than 4 padding cells per input byte plus 1 Mi. Its callers already handled the error.
  - `StringTable::parse` rejects more than 8× the file size plus 64 KiB in strings.
  - DT1 and DC6 now charge each header read against the existing len + 1 MiB budget. A file whose tiles or frames legitimately share headers in huge numbers would be rejected; none exists in either corpus.
  - `capy-history` caps a milestone tree at 64 folder levels; `capy-sprite` batch scans stop at 256.
- **Caller-controlled constructors stay infallible.** `Grid::new`, `IndexedImage::new`, `RgbaFrame::new`, `RgbaImage::filled`, `Mask::none` and `Dc6Frame::from_image` allocate whatever size the caller passes. File data never reaches them unbounded.
- **`export_sheet` limit.** It still allows sheets up to 2^28 pixels (1 GiB RGBA), so a tiny DC6 with widely spread frame offsets can ask for a 1 GiB sheet. `import_image` accepts up to 2^28 pixels with each side at most 16384, the same as before.
- **`Dc6::from_frames` loop.** It still loops over the full height of a zero-width frame. capy-sprite blocks this by rejecting sides over 2^24 before encoding.
- **`proptest-regressions/` files.** They were added for capy-render and capy-sprite. They hold seeds that failed before the fixes.
