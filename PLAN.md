# Capybara's D2 Editor (formerly Capybara Map Editor) — Plan

Status: **Release 1.2.0 published** (2026-10-20, github.com/Nugwa628/Capybara-d2-editor): 1.1.0 plus ds1-studio parity (P8–P13). Earlier: 1.1.0 (P7 palette tools). Deferred by the user: CLIP search, 16-direction sprite turning, COF/AnimData editors, D2R/CASC. Waiting on the user's DC6 examples to refine palette memory. Limits in 1.2.0 are listed under "P8–P13 acceptance".

## Decisions
- **Language:** Rust (stable), shipped as a single exe for x86_64 Windows. Linux/macOS are possible later but not a goal now.
- **UI:** Bevy draws the map as a live scene. bevy_egui and egui_dock provide the panels.
- **Game data:**
  - Classic D2/LoD MPQs and mod MPQs, with an overlay of any number of archives, including Median XL's split MPQs.
  - Loose mod folders.
  - D2R CASC sits behind the same trait; it is unimplemented and untested until a D2R install exists, so D2R is tested through extracted mod folders.
- **Game profiles** (TOML): *Vanilla* is the vanilla install's base MPQs. *Median XL* is the medianxl-* archives on top of Median XL's **own** base MPQs (never the vanilla ones). Users can add their own profiles.
- **Portable layout:** everything the editor writes lives next to the exe. Game install folders are **read-only and never written**. Copying or zipping the folder moves the whole setup.
  ```
  CapybarasD2Editor.exe
  capybara\
    config.toml                  settings
    saves\<profile>\data\global\tiles\...\*.ds1
                                 edited maps, mirroring game paths exactly
    saves\<profile>\CHANGED.txt  game-relative paths changed since the last "mark as packed"
    history\<profile>\           deduped snapshots + index.json (timeline)
    user.db                      tags, aliases, favorites, recents, saved searches
    cache\                       index.db + thumbnails (safe to delete; rebuilt automatically)
    profiles\<id>.toml           one game profile per file (built-ins written on first run)
  ```
- **Output:** saves are loose `.ds1` files. **The user packs them into the MPQ by hand.** To make that easy:
  - drag the `saves\<profile>\data` folder into an MPQ editor and the paths match;
  - `CHANGED.txt` lists exactly which files need packing;
  - the editor has "Open saves folder" and "Mark as packed" buttons, which clear CHANGED.txt.
  - The editor never writes MPQs.
- **Load order:** files in `saves\<profile>\` override the game archives, so maps you've edited open as your edited version.
- **Versioning:** hybrid.
  - Deduped snapshots in `capybara\history\<profile>\`.
  - An optional "Milestone" that makes a `gix` commit, with the repo in `capybara\saves\`.
- **Reference:** `reference/d2-ds1-edit` (MIT, gitignored) is a spec and test oracle. It is not ported line by line, and it is credited in NOTICE.

## Verified facts (2026-10-19, read-only probes)
### Machine
- rustc/cargo 1.99 stable `x86_64-pc-windows-msvc`. VS 2026 (v18) Community with MSVC 14.51. Python 3.14.
- Git's `link.exe` is on PATH. rustc normally finds MSVC's linker anyway, but if linking fails, check this first.

### Vanilla D2: `C:\Program Files (x86)\Diablo II`, patch 1.14d
- `d2data.mpq` has a plain listfile: 10,841 names, including 1,771 ds1, 206 dt1 and 83 txt.
- `d2exp.mpq` has an encrypted listfile compressed with **PKWARE implode (0x08)**.
- `Patch_D2.mpq` has **no listfile**, so its file names must come from elsewhere.

### Median XL: `C:\games\median-xl`, runs on 1.13c with D2Sigma.dll
- 56 MPQs named `medianxl-<base64>[-N].mpq`, plus its own modified copies of the base MPQs (`d2data.mpq` here is not identical to vanilla).
- The base64 parts decode to: animdata, bintbltxt, gui, itemsgfx, object, overlays, local, monzzz, maps, missls, sfx, tiles.
- Every listfile is encrypted with FIX_KEY (flags 0x80030200), and standard MPQ decryption reads them.
- 8,528 files in total, with **no path overlaps between the medianxl MPQs**. Their order relative to each other doesn't matter; together they only need to rank above the base MPQs.
- Maps:
  - 574 ds1 files: overrides of vanilla paths plus new ones under `data\global\tiles\mxl\{zone,quest,other,generic}\…`.
  - 129 dt1 files, all in `medianxl-dGlsZXM.mpq`.
- **Data tables ship only as compiled `.bin`** (`lvltypes.bin`, `lvlprest.bin`, `lvlwarp.bin`, `objects.bin`, `objpreset.bin`, `automap.bin`, …). There are no `.txt` versions, so we need a `.bin` excel decoder with the 1.13c record layouts.

### Reference editor (DS1Edit 2.1.1 source)
- **Specs to use:** `docs/guides/{ds1-map-format,dt1-tile-format,sprite-formats-dcc-dc6,cof-pipeline-1.13c,animdata-d2,palettes-and-colour,excel-tables-and-data-loading,tbl-string-tables}.md`. Each has a `.verification.md` companion.
  - **Do not trust** `docs/internal/ds1-validation.md` or `ds1-decoding.md`; their fixed layer counts are wrong.
- **MPQ reader:**
  - Hard-limited to 4 base MPQs and 1 mod folder (`structs.h:24-25`), so it **cannot load Median XL's extra MPQs**.
  - Accepts v1 headers only. Handles implode and zlib, but not bzip2.
  - Excel tables are read as `.txt` only, so **it cannot render Median XL maps**, which ship `.bin` tables only.
- **DS1:**
  - Reads any version through a ladder of version checks (ds1.c:535-1195). Truncated files must be tolerated.
  - Always saves as **v18** and renames the old file to `name-NNN.ds1` as a backup.
  - Stores editor state as `DS1EDIT_WRKSPC_*` strings inside the file list. We must keep these.
- **Tile rules:**
  - A tile is keyed by (orientation, main, sub), where `main = (prop3>>4)|((prop4&3)<<4)`. Floors use orientation 0, shadows 13.
  - Orientations 18 and 19 fall back to each other, then to sub 0.
  - Variant choice: the highest rarity wins. If the rarity sum is 0, the last block of the first DT1 is used, so DT1 load order matters.
  - The DT1 list comes from LvlTypes `File1..32`, masked by LvlPrest `Dt1Mask`.
- **Render order** is in `preview.c:2574`:
  1. Lower walls, then floors, then DT1 shadows.
  2. Object shadows.
  3. Objects with orderflag 1.
  4. Upper walls together with objects of orderflag 0/2.
  5. Roofs.
  6. Special tiles (orientation 10/11).
- **Blend modes** (`preview.c:291-380`, keyed on COF `trans_b`): 25/50/75% alpha, additive, multiply, and (ONE, INV_SRC_COLOR).
- **Gamma** is `255*(i/255)^(1/g)`. The palette is chosen per act, and LvlTypes row 35 needs the Act 4 palette.
- **Features it has:**
  - Tile-only undo with no redo.
  - Copy/paste for tiles and objects.
  - No way to move tiles between layers.
  - Two tile pickers: the old modal one has no mouse-wheel support at all; the newer one has the wheel and a recently-used list but no search.
- **Headless oracle:** `ds1edit <file.ds1> <LvlTypesId> <LvlPrestDef> --zoom=1:4 --headless out.png`. It renders deterministically. Compare terrain only, with a tolerance, because object frames use `rand()` and gamma comes from the ini (default 1.30).
- A prebuilt `ds1edit.exe` already exists at `C:\Users\ZF\Desktop\sugma\ds1edit-v2.1.1-windows-x86\DS1Edit-2.1.1-win32\`. **Copy that folder** to a scratch location before use; never edit the user's ini there.
- Embeddable editor data under MIT: `assets/editor/obj.txt` (DS1 object → COF token) and `ds1edit.dt1` (marker tiles).

### Our MPQ reader (2026-10-19, measured)
- Reads every listed file in all 11 vanilla and 68 Median XL archives with 0 failures (~69,000 files, 4.1 GB); every length matches the block table.
- Cross-checked against an independent pure-Python extractor (`scripts/extract_fixtures.py`): the 15,122 files both extracted are byte-identical.
- No archive in either install uses bzip2 (covered by synthetic tests only). Vanilla uses PKWARE via the compression mask; Median XL also uses the IMPLODE block flag.
- `Patch_D2.mpq` holds 209 files; names from the other listfiles cover 114. Vanilla has no `objpreset` table in any archive; Median XL ships `objpreset.bin`.
- Private corpus: `testdata\private\extracted\{vanilla,mxl}\` (6,458 and 8,681 files, with `_manifest.tsv`).

### DS1 and excel results (2026-10-19, measured)
- All 2,276 vanilla and 2,424 Median XL `.ds1` round-trip byte-identically, 0 exceptions. Vanilla: 42 files carry trailing bytes (41 x 4 bytes, `swamp2.ds1` 48); 2 are short (`trees.ds1` 12, `expansion\siege\xtransition.ds1` 4). Median XL: 39 trailing, the same 2 short.
- **The `.bin` layouts are confirmed** against vanilla `.txt` on every decoded cell: LvlTypes 0x788, LvlPrest 0x1B0, LvlWarp 0x30, Objects 0x1C0 (157 columns), Levels 0x220 **plus `leveldefs.bin` 0x9C**. 1.13c and 1.14d layouts are identical. Median XL decodes: 52 LvlTypes, 1,212 LvlPrest, 280 Levels, 589 Objects, and `objpreset.bin` (750 x i32, the `[act*150+id]` object remap vanilla compiles into the DLL).
- Vanilla's live excel tables come from `Patch_D2.mpq` in **both** `.txt` and `.bin` form, and they decode identically.
- `.bin` files do not store `Name` (LvlTypes, LvlPrest, LvlWarp) or `Beta`; display names come from Levels `LevelName` and string tables.

### Graphics formats (2026-10-19, measured)
- DT1: 655 files, 42,718 tiles, 926,942 blocks parse and render (both profiles). Six vanilla act1 files (`barracks`, `gargtrap`, `catacombs`, `cathedrl`, `court`, `outdoor1`) are an undocumented version 4.1 and are rejected; no LvlTypes slot names them (asserted in the P0 acceptance test).
- DC6: 4,065 files, 61,068 frames. DCC: 47,939 files, 7.1 M frames, 0 failures, every frame inside its direction box. COF: 8,817 files; `amblxbow.cof` (both profiles) is 72 bytes of noise, 3 Median XL COFs are short and zero-filled. AnimData: vanilla 3,558 records, Median XL 6,057.
- **Palette byte order settled:** `pal.dat` is BGR on disk; the PL2 base palette is RGB + pad (= `pal.dat` reversed) in every pair of both installs. The reference reads PL2 correctly; the guide's description is inverted. Act palettes: index 0 black, 255 white.
- COF priority tables hold component ids, not layer indices.
- DCC decode keeps per-cell state; the reference's shared-bitmap model differs on 0.15% of sampled frames (1-9 edge pixels, stale-pixel artefacts on the reference side). Confirm against the oracle when objects render.

### Oracle (2026-10-19)
- `testdata\private\oracle-renders\`: 18 vanilla maps at `--zoom=1:4`, gamma 1.30, each rendered twice, all byte-identical. Includes LvlTypes 35 (`act5_lava`).
- **There is no terrain-only switch** in the reference CLI or ini; the headless render always includes objects and object shadows. The P1 comparison therefore masks a box around every DS1 object (see `capy-render` tests) instead of rendering terrain-only.
- Headless exit code is 0 even on failure; success is detected from stdout and the PNG.

### Renderer vs oracle (2026-10-19, measured)
- **User visual check passed (2026-10-19):** the 5 Median XL renders (scosglen town, uldy tri_town4, talrasha, heaven charismatic_base, dunc bonehexerx) look correct.
- `capy-render` matches all 18 oracle maps: canvas sizes equal, best shift (0,0), `within_tolerance(16)` 0.9915–1.0000 with objects masked; the five object-free maps are pixel-exact. All 8 Median XL test maps render with 0 missing tiles (`testdata\private\mxl-renders\`).
- Rules measured against the oracle (they refine the first draft of `docs/api/capy-render.md`):
  - Pass 1 is a single per-cell loop: lower walls, floors, shadows for each cell (`preview.c:3688`). Within a cell, layers of one family sort by `prop1` (orientation 10 = 255).
  - Orientation 3 also draws the matching **orientation 4** tile (`preview.c:1131-1160`).
  - Only orientations 18/19 fall back: to the swapped orientation, then the swapped orientation with sub 0 (`ds1.c:153-193`).
  - Special tiles (10/11) ignore the hidden bit; animated floors show frame 0.
  - Zoom scales each tile with point sampling at source column `N*k + N/2`; it is not composite-then-shrink.
  - DT1 shadows: 50% mode, `out = d*127/255 + s*(128/255)^2`.
- **Variant choice default = DS1Edit's rule (`VariantRule::ReferenceQuirk`, `misc.c:156-187`)**, lead decision. In vanilla every multi-candidate group is a rarity tie (rarities are almost all 1); the documented "first highest" rule matched only 8/18 oracle maps, DS1Edit's matched 18/18. The real game picks randomly among nonzero-rarity blocks, so neither rule is "the game"; matching the editor modders know wins. `Documented` stays available as an option.
- **Palette = Levels `Pal` when the map's Level is known**, else the reference rule (LvlTypes act, overridden when all File entries sit under one `ActN\`, which makes LvlTypes 35 use Act 4). Median XL needs `Pal` (e.g. Level 241: LvlTypes act 5, `Pal` 2 = jungle); for vanilla test maps both agree.

### P1 acceptance (2026-10-19)
- Oracle: the CPU renderer matches all 18 vanilla oracle maps (canvas equal, shift 0,0, within_tolerance(16) >= 0.9915 with objects masked; object-free maps pixel-exact).
- The app's GPU renderer matches the CPU renderer on all 18 maps (within_tolerance(16) = 1.0000; differences at tolerance 0 are 1-level shadow rounding).
- Median XL: 8 maps render with 0 missing tiles; the user checked 5 by eye and confirmed them (2026-10-19).
- Index: background build with progress bar and cancel; second launch reports up to date with no rebuild (0.01 s); the unit test proves a size/mtime change makes only that profile stale.
- Zoom 1:1-1:16 on DS1Edit's integer zoom grid, pan, layer toggles (floors, walls, shadow, roofs, special, tag).

### P2 acceptance (2026-10-19)
- Property tests: undo-all restores, redo-all reproduces, undo∘redo is identity at every step (thousands of random sequences incl. layer moves, paste, resize, version upgrades, objects, NPC paths); `NeedsVersion` is always the least sufficient version.
- Edited maps load in the oracle and render as written: 22/22 maps (incl. v3/v12/v13/v15) at within_tolerance(16) >= 0.991.
- App: scripted end-to-end edit (select, Wall 1 → Wall 3, paint, cross-region paste, save) produces a correct file in `capybara\saves\<profile>\`, CHANGED.txt lists it once, and a history snapshot exists.

### P6 acceptance (2026-10-19)
- Encoders: every DC6 (4,065) and DCC (47,939) in both installs re-encodes to identical frames; DCC output averages 1.3% larger than Blizzard's.
- Converter: PNG export -> import (with sprite.json) -> encode -> decode is exact; sheet round trip; batch conversion of trees both ways with cancel.
- Editor: palette lock (no opaque index 0, every pixel a palette index) and undo/redo laws hold under randomized operation sequences; DCC 4x4-cell limit check uses the decoder's own cell partition and is proven by an encode/decode round trip.
- App: scripted end-to-end test exports a real DCC, re-imports and saves it to a game path (identical frames, CHANGED.txt, snapshot), edits it with outline and gradient (still palette-locked), and runs the single-exe CLI render.

### P3 acceptance (2026-10-19)
- Index search: 26/26 table-driven queries over the Median XL index (index, generated name, colour, tag, alias, source file, used-in, similar, mirrored, src, type) return the expected asset in the top 5.
- App: scripted end-to-end test on Median XL searches `color:green type:floor file:whimsyfloor.dt1`, uses the hit as the paint brush, paints, and checks recents, favorite, alias and the saved cell; alias search returns it first.

### P4 acceptance (2026-10-19)
- Crash safety: killing the process during saves (40 random kills) never corrupts the target; CHANGED.txt lists the path exactly once.
- Snapshot store: dedupe, checkpoints survive pruning, corrupt blobs detected; gix milestones commit/no-op/log.
- App: scripted end-to-end test paints, autosaves, checkpoints, diffs (48 changed floor cells), restores (BeforeRestore recorded, one undo step), saves (file equals the checkpoint), commits a milestone (second commit is a no-op) and prunes (checkpoint survives); autosave/checkpoint/restore never touch CHANGED.txt; the game install folder is unchanged (file list, sizes, mtimes).

### P5 acceptance (2026-10-19)
- GPU map view draws objects and NPCs from `draw_list_with_objects` in DS1Edit order, with blend runs (Normal, Alpha, Darken shadows, Additive, Multiply, Screen); gpu_parity with objects at 1:4 on all 18 oracle maps: tol16 1.0000, shift 0,0, canvas equal (towne1 42, lutw 67, docktown3 84 drawing objects).
- Animation at 25 Hz (FrameDelta or AnimData speed; meshes rebuilt only for changed frames); placement picker over `placeable(act)`; properties via `set_object`; NPC path tool (add/insert/move/delete, actions 1-5) with NeedsVersion upgrade flow; DS1Edit ini import into per-profile overrides.
- App: scripted e2e on vanilla towne1 imports an ini, places a torch and Gheed, adds a 3-point path, saves, re-parses the DS1 (objects, anchor, points, actions), checks config.toml and an unchanged install, and compares the saved map GPU vs CPU `render_with_objects` (tol16 1.0000).
- Fixed: the textured map sat half a cell (80 px) right of every overlay; run meshes and the map-only camera now shift by `WORLD_SHIFT_X = -80` (parity unchanged).

### Robustness audit (2026-10-19, `docs/robustness-audit.md`)
- 351 hazard sites reviewed, 76 reachable ones fixed (overflow on file values, allocations sized by unbacked counts, unbounded loops and recursion); no unsafe, exit, panic or unwrap outside tests.
- Fuzz tests on every parser entry point; `PROPTEST_CASES=5000` release run: 800 passed, 0 failed. Both oracles (render 74 rows, edit oracle) identical to the pre-audit baselines.
- New limits, far above shipped data: txt padding (4 cells/byte + 1 Mi), tbl strings (8x file size + 64 KiB), DT1/DC6 headers count against the read budget, milestone trees 64 levels, sprite batch scans 256 levels.

### P7 acceptance (2026-10-19)
- Library: `d2_formats::remap` on the full corpora: 643/643 readable DT1 and 4065/4065 DC6 give byte-identical output for the identity table and change only pixel bytes for an act5->act1 table (83.2 M bytes in 511 DT1, 6.5 M in 483 DC6); 47,939/47,939 DCC decode to the mapped frames. `capy_paint::remap` property-tested; act5->act1 default plan: 29 indices re-matched, mean 0.40, worst 10.5.
- App: `palette_e2e` converts `expansion\siege\ground.dt1` Act 5 -> Act 1 with one override (output equals `remap_indices(original, table)`, only pixel bytes differ, CHANGED.txt, install untouched) and `ui\bigmenu\radiobutton.dc6` to all-acts shared colours (no index 225-254 left); palette badges survive a restart. Fact found: in-game UI, item and skill-icon DC6s use no index in 225-254; only menu/front-end art does.
- Fixed on the way: concurrent saves (batch worker + main thread) could drop a CHANGED.txt entry; `SavesDir` now serialises CHANGED.txt updates (test fails without the lock).
- Lead decisions on the app's open questions: "Use <remembered>" in the save warning switches palette and continues the save; a conversion to "All acts" is remembered as `act1` (it displays identically).

### P8–P13 acceptance (2026-10-20, branch `feat/ds1-studio-parity`)
- **Whole branch:** `cargo fmt --check` clean, clippy 0 warnings, `CAPY_GPU_TESTS=1 cargo test --workspace` 1,311 passed, 0 failed (full corpora extracted on 2026-10-20, including DC6/DCC).
- **Writers:**
  - DT1: 619/631 round-trip byte-identically; the other 12 are v4.1 files the reader rejects. All 895,820 blocks re-encode to the same pixels.
  - `.txt`: 344/344 round-trip; a single-cell edit changes only that cell.
  - `.tbl`: 40/40 string tables round-trip and rebuild byte-for-byte; the 32 font `.tbl` files are a different format.
- **UI:**
  - Screenshot at 1600×900 matches ds1-studio's `overview.png` section by section. Walkable area on townn1 is 593, the same value ds1-studio shows.
  - `shell_e2e` clicks 86 ribbon buttons and presses 70 default keys.
  - `studio_e2e` (P9/P10) and `tools_e2e` (P11–P13) cover every feature that writes files.
- **Gaps closed before release (2026-10-20):** the Tiles panel tidy, a COF encoder (8,815/8,817 COFs round-trip byte-identically; the other 2 are `amblxbow.cof`, which is not a COF), editable Automap and Level light modes (real colour multiply), re-roll from other libraries, the stack-paint brush, walk painting and the no-spawn planner, the remaining compatibility rules and the `ShortenPaths` fix, the missing MCP tools, and preset packages.
- **Release gate (1.2.0):** fmt clean, clippy 0 warnings, `CAPY_FULL_CORPUS=1 CAPY_GPU_TESTS=1 cargo test --workspace` 1,352 passed, 0 failed.
- **Known limits in 1.2.0:**
  - Tables and Level light can't be edited where a profile has only `.bin` tables (the `.bin` decoder reads a column subset, so there is no `.bin` writer).
  - ds1-studio's "set exactly" walk-paint mode and `render_map` overlays in MCP aren't ported.
  - DS1Edit oracle tests skip because `reference/d2-ds1-edit` is absent in this clone.

### Crates (crates.io, 2026-10-19)
| Crate | Version | Note |
|---|---|---|
| bevy | 0.20.0 | **pinned** |
| bevy_egui | 0.43.1 | **pinned**; requires egui 0.36 |
| egui | 0.36.2 | **pinned** (not 0.37: see Deviations) |
| egui_dock | 0.21.1 | **pinned**; requires egui 0.36. `cargo tree -d` shows one egui, bevy, winit and wgpu; a bevy+egui+egui_dock probe compiles |
| gix | 0.89 | |
| rusqlite | 0.40 | Bundled SQLite, FTS5 |
| image_hasher | 3.1 | |
| nucleo | 0.5 | |
| pklib / explode | 0.2 / 0.1 | |
| mpq | 0.8 | Read-only |
| wow-mpq | 0.7 | |

- Use an MPQ crate only if it passes the archive tests below. Otherwise write an MPQ reader of about 600 lines, since the format is small. It must handle:
  - v1 headers, and v2+ headers without failing;
  - encrypted files and FIX_KEY;
  - implode, zlib and bzip2 (Huffman and ADPCM only matter for wav files, which we skip);
  - every Median XL archive.

## Workspace layout
```
crates/
  d2-formats/    ds1 dt1 dc6 dcc cof pl2 animdata txt excel-bin tbl  (pure, no Bevy)
  d2-assets/     AssetSource trait: Mpq, Folder, Casc(stub); N-layer overlay resolver; known-names
  capy-index/    crawler -> SQLite+FTS5, thumbnails, generated names, pHash, tags/favorites/recents
  capy-edit/     document model, commands, undo/redo, selection, clipboard, layer ops
  capy-history/  snapshot store + gix milestones
  capy-app/      Bevy + egui binary (the only crate that depends on Bevy)
docs/specs/      cross-checked format notes (reference guides + our findings)
reference/       upstream clone (gitignored)
testdata/        synthetic fixtures (committed); testdata/private/ = real game files (gitignored)
```

## Phases (each needs its tests to pass before the next one starts)
- **P0 Formats and assets.**
  - Cross-check the reference guides. Build `d2-formats` and `d2-assets`, including the `.bin` excel decoder for 1.13c and 1.14d and the known-names list.
  - **Test:**
    - Every ds1 in both installs round-trips byte-identical when unchanged; exceptions such as truncated files are listed and justified.
    - Every dt1 parses. There is no dt1 writer.
    - Every known name resolves.
    - The `.bin` decoder matches `.txt` wherever a table exists in both forms. Every DT1 and DS1 named in Median XL's LvlTypes and LvlPrest resolves.
- **P1 Viewer and index.**
  - Bevy renders tiles in reference order with blend modes and act palettes. Mouse-wheel zoom from 1:1 to 1:16, pan, and layer toggles.
  - `capy-index` runs in the background with a progress bar.
  - **Test:**
    - At least 5 vanilla maps match the reference headless renders, terrain only and within tolerance.
    - At least 5 Median XL maps render without missing tiles. Their screenshots are sent to the user to check by eye, because the reference can't render Median XL.
    - The second launch loads the index from cache with no re-crawl. A unit test checks that changing an archive's size or modified time triggers a rebuild. No timing benchmarks.
- **P2 Editing.**
  - A command pattern with unlimited undo and redo.
  - Box, lasso and select-by-tile-ID selection, plus "select identical".
  - Copy, cut and paste across maps, with a preview of conflicts.
  - Move or batch-move a selection between layers (for example wall1 → wall3). A layer is added automatically if the map doesn't have it.
  - **Test:** property tests show that undoing a redo returns the original, and that edit-then-save files load in the reference editor.
- **P3 Tile browser and search.** See the asset index section.
  - **Test:** a table-driven test of about 20 queries over the Median XL index (by index, generated name, color, tag, source file). Each must return the expected asset in the top 5.
- **P4 History.**
  - Snapshots on save and by autosave timer, named checkpoints, a timeline, a visual tile diff, restore, a retention limit, and gix milestones.
  - Saves are atomic (write a temp file, then rename). Each save also updates `CHANGED.txt`.
  - **Test:** killing the process during a save never corrupts the target file.
- **P6 Sprites mode** (added at the user's request, 2026-10-19).
  - A top-level mode tab switches between **Map Editor** and **Sprites**. Sprites mode works without any map open. A dockable sprite viewer is also available inside the map editor.
  - Viewer: DC6/DCC/COF, direction/frame picker, animation at AnimData speed, palette and gamma picker, COF layer compositing with draw order and blend modes, bounding-box and anchor overlay.
  - Converter (feature list from SixDice v0.63's README/CHANGES only; its license forbids reverse engineering, so its jar is never opened, decompiled or run for this work):
    - DC6 and DCC **encoders** in `d2-formats` (DC6 runs capped at 127; no state leaking between DCC frames);
    - image import (PNG/BMP/GIF/PCX) with palette quantization, optional Floyd-Steinberg dithering and transparent protection (opaque pixels never map to index 0);
    - frame split/join (256x256), trim borders, centre/batch offsets, add/remove frames and directions;
    - export of frames + `sprite.json` (exact offsets for re-import) and of sprite sheets;
    - batch conversion of directory trees in both directions, from the GUI and the exe's CLI.
  - Output: "Save to game path" writes to `capybara\saves\<profile>\<game path>` and appends CHANGED.txt; "Export to…" and batch conversion write to a folder the user picks.
  - **Sprite editor** (user scope, 2026-10-19): a lightweight, palette-locked "mini-GIMP" (`capy-paint` engine, UI in `capy-app`). Every pixel stays a real palette index. Tools:
    - pencil and eraser with brush sizes; line, rectangle and ellipse; fill; replace colour; picker;
    - **gradient** (linear/radial, two colours or a ramp, Bayer or Floyd-Steinberg dither), **shading brush** (steps along the pixel's own colour ramp), **outline** (fixed or auto-darker colour, inside/outside, 1-8 px), stray-pixel clean-up;
    - selections (rectangle, ellipse, lasso, magic wand; grow/shrink/invert) with move, cut, copy, paste (incl. RGBA images from the clipboard, palette-mapped), flip, rotate 90;
    - frames (insert, duplicate, delete, reorder), mirror a direction, offsets, canvas resize, onion skin, unlimited undo;
    - DCC 4x4-cell colour-limit check with auto-reduce;
    - export of palette (.gpl/.act/.pal), indexed PNG for GIMP/Photoshop round trips, animated GIF.
  - **Later (noted by the user):** "turn" a sprite into all 16 directions inside the app to build a complete DCC from one view. Also later: COF and AnimData metadata editors, D2R `.sprite`, image-to-DT1 tile maker (see `docs/research/sprite-tools-survey.md`).
  - **Test:** every DC6 and DCC in both corpora re-encodes to the same frames (decode, encode, decode); a PNG import/export round trip preserves pixels and offsets; batch conversion of a sample tree both ways.
- **P5 Extras.** Objects and NPC paths, animated DCC/DC6, a headless render CLI, and an importer for the legacy `assets/*.ini` files. (No MPQ writing; the user packs by hand.)
  - Stretch goal: offline CLIP text-to-image tile search. **Deferred by the user (2026-10-19)**; feasibility and plan in `docs/research/clip-search.md` (opt-in model download into `capybara\cache\clip\`, gated by a one-day quality check).

- **P7 Palette tools (user request, 2026-10-19, from modder feedback).** Problems: tiles copied between acts show wrong colours (the act palettes differ only at indices 225-254; measured), and UI art made under the wrong palette glitches in game; users forget which palette a file belongs to.
  - **Convert to palette** (manual only, never automatic): pick a source palette and a target (Act 1-5, units, loading, menu0-4, ..., or "All acts (shared colours only)" = skip 225-254), see a plan of every used colour (source swatch -> target swatch, error, pixel count), adjust it (distance OkLab/redmean/RGB, allowed targets, keep exact matches, lightness weight, brightness/contrast/saturation/hue, per-colour overrides from a candidate list or the palette grid), compare before/after, then Convert. Works on DC6, DCC and DT1, single files or a batch with one shared setting; output to the game path (saves, CHANGED.txt, snapshot) or Save As. Named presets in config.toml.
  - **DT1 and DC6 are patched in place** (only pixel bytes change; everything else byte-identical), DCC is decoded/mapped/re-encoded. This revises the earlier "DT1 is read-only, no writer ever" decision: still no general DT1 writer, only index remapping.
  - **Palette memory**: the Sprites tab remembers the palette per file (user.db), falls back to the folder's last palette, and shows it as a badge in the Viewer, Editor and Converter; save and import show the palette and warn when it differs from the file's remembered one; sprite.json records it. To be refined with the user's DC6 examples.
  - Planner merged (2026-10-19): act5->act1 default plan keeps every index outside 225-254, re-matches 29, mean error 0.40, worst 10.5 (index 239, a saturated cyan Act 1 lacks). UI decisions (lead): defaults OkLab, keep exact on, lightness weight 1.0 as an advanced slider 0.5-3, Adjust sliders +-0.3 default 0; when Adjust is non-zero and keep exact is on, show a hint with a one-click "turn off keep exact" (never change it silently); overrides outside the allowed target set are honoured but flagged in the plan table; entries above error 5 marked visible, above 10 obvious.
  - Contracts: `docs/api/d2-formats.md` (remap), `capy-paint.md` (remap planner), `capy-index.md` (palette memory). Acceptance: act5 -> act1 DT1 conversion with overrides round-trips (only pixel bytes differ, output indices within the allowed set except explicit overrides, identity table = identical file) on the full corpora; DCC conversions decode to the mapped frames; app e2e converts a real act5 DT1 and a DC6, saves, reopens, and the palette is remembered across restarts.

## ds1-studio parity (P8–P13, user request 2026-10-20, done; see "P8–P13 acceptance")
Goal: give end users ds1-studio's UI and layout for map editing, then add every ds1-studio feature Capybara lacks.
- **Reference:** ds1-studio v0.2.63, commit `1d01dcd` (MIT, "Copyright (c) 2026 RoofooEvazan"). React + TypeScript + WebGL2 in Tauri 2; all parsing and editing is TypeScript.
  - Nothing is copied. As with DS1Edit, it is a spec and behaviour reference. Each feature is rebuilt in Rust and its vitest cases (`test/*.test.ts`) are translated into Rust tests.
  - Keep a clone in `reference/ds1-studio` (gitignored like DS1Edit).
  - **Credit (user, 2026-10-20: "give all credit"):** NOTICE section 3 (with its MIT text) and a link near the top of README to the original, `github.com/RoofooEvazan/ds1-studio`, and to the fork the user linked, `github.com/D2R-Gimli/ds1-studio`.
- **Unchanged decisions:** the game folder stays read-only, nothing writes MPQs, and every output goes to `capybara\saves\<profile>\<game path>` with a `CHANGED.txt` entry and a history snapshot. ds1-studio writes into the mod folder with a `.bak`; we don't.
  - Maps keep their own DS1 version (ds1-studio always writes v18).
- **Kept from Capybara:** history and snapshots, the indexed tile search, Sprites mode, palette tools, Median XL support.
- **How each feature is ported:**
  1. Read the ds1-studio module and write its behaviour into `docs/specs/` or `docs/api/`.
  2. Put the logic in a pure crate (`d2-formats`, `capy-edit` or a new one), with no UI.
  3. Translate the matching vitest cases into Rust tests.
  4. Build the egui UI through the P8 command registry.
  5. Add an app e2e test (`CAPY_SCRIPT`/`CAPY_SCREENSHOT`), then update `docs/user-guide.md` and `CHANGELOG.md`.
- Every edit is a `capy-edit` patch, so undo and redo work. Every new writer must round-trip byte-identically on the full vanilla and Median XL corpora before anything uses it.

- **Order and agents (user, 2026-10-20: "whatever order completes fastest; Haiku and Sonnet where Opus wouldn't be better").**
  - capy-app is the bottleneck (one warm worktree, long compiles), so its UI work runs one phase at a time. Everything outside capy-app runs in parallel.
  - **Wave A, in parallel:**
    - P8 shell in capy-app (Opus).
    - DT1 writer (Opus).
    - `.txt` and `.tbl` writers (Sonnet).
    - capy-edit patches for P9 (Sonnet).
  - No separate spec-writing pass (saves tokens): each implementer reads the ds1-studio module it ports (`reference/ds1-studio`) and records only non-obvious behaviour in `docs/specs/`.
  - **Wave B:** capy-app UI in order P9 → P10 → P11 → P12 → P13. Each phase's pure-crate work runs one step ahead, in parallel with the UI phase before it.
  - **Opus:** the P8 shell and command registry, the DT1 writer, Bevy render changes (animated tiles, overlays), tag-group editing, collision and walkability semantics, add-to-game.
  - **Sonnet:** most feature ports, dialogs and panels, the `.txt`/`.tbl` writers, the table editor, automap, packages, MCP.
  - **Haiku:** porting the theme and keybinding tables, translating simple vitest cases, docs and CHANGELOG, scripted screenshot and e2e runs, regenerating the license files.
  - Every phase's acceptance is still checked by the lead before it is marked done.

- **P8 ds1-studio UI and layout** (Map Editor only; Sprites mode keeps its dock layout).
  - **Themes:** five base colours per theme (bg, panel, text, accent, line), with the other shades derived, mapped to egui `Visuals`.
    - Port the 9 built-in themes from `src/ui/themes.ts`. "DS1 Studio" is the default. Custom themes can be imported and exported as share codes.
    - Segoe UI loaded at runtime with egui's font as fallback; the Lucide icon font (ISC) embedded.
  - **Command registry:** each action has an id, label, icon, default key and an "enabled when" rule. The ribbon, right-click menus, Ctrl+K command finder and shortcuts are all built from it.
    - Default keys follow `src/ui/keybindings.ts` (user, 2026-10-20). There is no Capybara preset; a Shortcuts dialog rebinds any key, and bindings are saved in `config.toml`. CHANGELOG lists the old → new keys.
  - **Layout:** replace `egui_dock` in map mode with fixed panels.
    - A 44 px top bar: app name, ribbon tabs, command box, file name and game path, and a Sprites mode switch.
    - Left 260 px and right 320 px columns, both resizable and collapsible to 30 px. Shift+F folds both ("just the map").
    - The map in the centre and a 26 px status bar.
  - **Ribbon:** Home, View, Map and Help tabs; Game and Diagnostics are added once P12 exists.
    - Home groups: File, Edit, Tools, Paint/erase, Active layer.
    - Lasso, select-by-tile-ID and select-identical move into a Select ▾ dropdown.
  - **Left column:** a folder tree of maps with a filter box and counts.
  - **Centre:** the Show bar of layer chips (Shift-click or Shift+number to solo), a collapsible DT1 wall-categories row, a minimap and the walkable-area badge.
  - **Right column:** Tiles and Presets tabs.
    - The Tiles tab has category chips, a DT1 dropdown, a filter box and labelled thumbnails. Capybara's search tokens work in the filter box.
    - Below the tabs: collapsible Cell, History, Tags & groups and Layers sections.
    - The Log and Objects panels become floating windows; Objects also appears in the right column while the Objects tool is active.
  - **Status bar:** a hint for the current tool, then tool and layer, cell, zoom, and map size, version, act and palette.
  - **Test:**
    - A 1600×900 screenshot next to ds1-studio's `docs/media/overview.png`, compared section by section and confirmed by the user.
    - An e2e test that clicks every ribbon button and presses every default key.
    - A screenshot e2e for each theme.
    - All existing e2e tests pass.

- **P9 Map editing parity.**
  - Paint or erase a rectangle (U) or a connected area (L).
  - Random-mix painting (Ctrl+click several tiles).
  - Find & replace tiles (Ctrl+H).
  - Floor re-roll (`floorReroll.ts`, `floorLibrary.ts`) and tile reassignment (`reassignTiles.ts`).
  - New map; resize by dragging the map edge.
  - Shift+wheel through stacked tiles; double-click selects every copy of a tile; Shift+O switches between tiles and objects.
  - Cell section: raw bytes and cell flags. Sub-tile collision painting waits for P11.
  - Tag group editing in `capy-edit` (today only the count is shown).
  - Object checks (`objectChecks.ts`).
  - PNG export of the map or a selection, and copying the view to the clipboard (PrintScreen).
  - **Test:**
    - Proptests show that each new patch undoes to the original.
    - Every DS1 still round-trips byte-identically, including maps with edited tag groups.
    - Translated vitest cases pass.
    - Edited maps open in DS1Edit (the edit oracle).

- **P10 Views and overlays.** View modes (Tiles, Objects, Walkability, Automap, Level light, Roof hiding) with Tab/Shift+Tab and F1–F6.
  - Animated floor tiles, lava and water (`waterAnimation.ts`; today only frame 0 is drawn).
  - Game-view framing at a chosen resolution (Z), the 8×8 rooms grid (R), zoom to 100% (Home).
  - Walkability view and painting (`walkArea.ts`, `walkEdit.ts`). Check whether walk painting writes DS1 cell flags or DT1 sub-tile flags; the DT1 part waits for P11.
  - Spawn regions (`spawnRegions.ts`).
  - Warps, with a jump to the target level (`warps.ts`, `exits.ts`, `arrival.ts`).
  - Roof and wall hiding view (`pops.ts`).
  - Level light view. Editing it may need the P12 table writer; check where `LevelLightEditor` stores it.
  - The Automap view becomes editable in P12.
  - **Test:** for each overlay, screenshots on a fixed set of vanilla maps compared with ds1-studio, and checked by the user on Median XL maps.

- **P11 DT1 writer and tile tools.**
  - **General DT1 writer in `d2-formats`.** This revises the "no general DT1 writer" decision in P7.
    - **Gate:** every DT1 in both corpora round-trips byte-identically, and the fuzz and proptest suites pass.
  - Sub-tile collision-flag painting, using `docs/collision-flags.md` from ds1-studio as the reference for what each bit means.
  - DT1 manager (`Dt1Manager.tsx`, `Dt1Tree.tsx`): duplicate, rename, recolour (`dt1Edit.ts`).
  - Pixel painting on tiles, reusing `capy-paint`.
  - Tile settings, as a DT1 Tools `.ini`.
  - Make act-safe, building on the P7 remap code and the Act 0 palette.
  - Duplicate DT1s, custom DT1s and "own tiles" review (`duplicateDt1s.ts`, `customDt1.ts`, `ownTiles.ts`, `dt1Review.ts`).
  - Unused-asset report, with archiving and restoring inside `capybara\` (`assetUsage.ts`).
  - Presets library, preset builder (20×20) and suggested presets (`presets.ts`, `presetLibrary.ts`, `PresetBuilder.tsx`). These fill the Presets tab.
  - **Test:**
    - A recoloured or painted DT1 changes only the intended bytes and loads in DS1Edit.
    - A duplicated DT1 renders the same as its source.
    - Preset save, place and undo work.

- **P12 Game data.** Every table and string edit is saved to `capybara\saves` and recorded in `CHANGED.txt`; history snapshots include it.
  - Lossless `.txt` writer, plus the table editor with column help (`txt.ts`, `txtTable.ts`, `TableEditor.tsx`, `src/data/columnHelp.ts`). Unchanged rows stay byte-identical.
  - `.tbl` writer (for the entering text).
  - Add a map to the game (Levels, LvlPrest, LvlTypes); change level type; type packages (`addToGame.ts`, `changeLevelType.ts`, `typePackage.ts`).
  - Automap editor: style, clear, import, safety checks (`automap*.ts`).
  - Cube recipes (`cubeRecipe.ts`, `mapRecipes.ts`).
  - Entering text (`entryText.ts`).
  - Custom PNG objects (`customObject.ts`).
  - Level light editing, if P10 found it lives in a table.
  - Compatibility checker with one-click fixes (`compat.ts`); crash-log reader (`crashLog.ts`).
  - Ribbon: add the Game and Diagnostics tabs.
  - **Test:**
    - Every `.txt` and `.tbl` in both corpora round-trips byte-identically.
    - A map added to the game resolves through the `d2-assets` presets catalog.
    - The compatibility checker's fixes are idempotent.

- **P13 Packages and automation.**
  - Map packages (`.zip`): export, and merge-import into `capybara\saves` (`mapPackage.ts`).
  - Import a DS1 together with the DT1s it needs.
  - `capy mcp` subcommand, built on `capy-edit` and `capy-render`, with ds1-studio's tool names (`list_maps`, `open_map`, `new_map`, `paint`, `set_cell`, `copy_area`, `find_replace`, `reroll`, `resize_map`, `set_cell_flags`, `add_object`, `edit_objects`, `undo`, `redo`, `check_map`, `render_map`, `save_map`).
  - **Test:**
    - A package export/import round trip gives identical files.
    - The MCP tool calls are scripted and the resulting map is compared with the same edits made in the GUI.

- **P14 Distribution: dropped by the user (2026-10-20).** Kept for reference only.
  - **Self-updater** (about 1–2 days).
    - The app checks GitHub releases, downloads the zip, verifies a signature, swaps the exe (`self-replace`) and restarts.
    - The first network access in an otherwise offline app, so it should be opt-in. Also needs a signing key, and has some antivirus false-positive risk.
    - "Check only, then link to the download" is about half a day.
  - **Linux build** (about 3–5 days, plus testing each release on two OSes).
    - The Rust stack (Bevy, egui, rfd, arboard) is cross-platform.
    - The work is: case-sensitive paths for loose mod folders and saves; install detection for Wine, Lutris and Steam prefixes instead of the registry; cfg-gating `winresource`; a Linux release script (AppImage or tar.gz); running the GPU e2e suites under Linux (WSL2 is enough).
    - Decisions currently say Linux is "not a goal now".

## Asset index (`capy-index`, built in P1, used by P3)
1. **Crawl:**
   - Build the file list from three sources:
     - the archives' own listfiles;
     - a known-names list built from both installs' listfiles;
     - names referenced by LvlTypes, LvlPrest, objects, COF and DS1 files.
   - This covers `Patch_D2.mpq`, which has no listfile.
   - Record each file's source archive, priority, size, hash and which layer wins.
2. **Decode** per tile:
   - DT1: orientation, main/sub index, rarity, size and flags. Also the parent DT1, the LvlTypes rows that load it, and the Dt1Mask bit.
   - DC6/DCC: frames and directions.
   - DS1: which DT1s and objects it references, giving "used in" back-links.
3. **Names for unnamed tiles:**
   - Generated from context: `Act1 Wilderness · Floor · grass · 05/02`.
   - Plus visual descriptors: dominant palette color, a texture class estimated from edge density, and how much of the tile is transparent.
   - pHash for "find similar", detecting mirrored copies and grouping variants.
   - User aliases, tags and notes are stored separately, so re-indexing never wipes them.
4. **Storage:**
   - `capybara\cache\index.db` holds one SQLite file per machine: the asset rows, FTS5 search, and thumbnails stored as small PNG blobs. Storing them as blobs means no atlas code.
   - `capybara\user.db` holds the user's data (tags, aliases, favorites, recents, saved searches), keyed by stable asset keys such as path + orientation/main/sub, so it survives a rebuild of the cache.
   - Rebuild rule, chosen to keep the code small: if any archive in a profile changes size or modified time, rebuild that whole profile's index in the background. Incremental per-archive re-indexing waits until rebuilds actually feel slow.
5. **Ctrl+K global search:**
   - Fuzzy matching (nucleo) plus filter tokens: `type: act: color: tag: idx:5/2 file: similar: used-in: mirrored: src:mod`.
   - Thumbnails update live as you type. Enter picks the result as the brush; Shift+Enter shows it in the browser.
6. **Recents** are recorded automatically. **Favorites** are stars in named, reorderable groups. **Saved searches** work as smart folders, so custom tiles can be pinned to the top.

## Format decisions (lead)
- **COF layer priority uses the file direction**, the same index as the sprite frames. The reference viewer indexes priority by the raw user direction while sprites go through the dir8/dir16 remap (`cof.c:103-110`, `preview.c:2095`), which looks like a bug; its compose path (`compose_render.c:470`) is consistent and is what we follow. See `docs/specs/cof.md`.
- **DCC pixel-buffer fill pops decoded colours in reverse order, with zero fill** (`dcc.c:989-1001`). This is the standard DCC algorithm, not a quirk.
- **Loaders are as lenient as the game but always bounds-checked**: no signature-strictness the game lacks, and no crash or over-allocation on bad data. No `.dcc`→`.dc6` fallback on decode failure (the reference does this, the game does not); a failed sprite is reported as missing.
- **AnimData lookup** ignores case and a trailing `.cof`.
- **Open, to settle against the oracle in P1** (from `docs/specs/palette.md`, `dt1.md`):
  - PL2/pal byte order: the reference reads R from byte 0 (`palette.c:27-29`) while the guide says byte 0 is B. Settle by rendering one vanilla map both ways against the oracle.
  - DT1 variant choice: the reference never compares the group head's rarity (`misc.c:156-187`), so on ties or a higher-rarity head it can pick a different variant than "first highest rarity wins". We implement the documented rule and record any oracle mismatches it causes.
  - COF blend row names (`structs.h:92-97`) are inverted against the guide's percentages; confirm with one composited sprite.
- **`.tbl` strings** decode as Latin-1 for classic profiles (UTF-8 is a D2R concern); a missing key returns `None`.

## API contracts
- `docs/api/*.md` (d2-formats, d2-assets, capy-edit, capy-history, capy-index) are the public APIs, owned by the lead, including the index schema and naming rules in `capy-index.md`. Agents implement against them and ask before changing them.

## Deviations (with reasons)
- **egui 0.36.2 instead of 0.37.** bevy_egui 0.43.1 and egui_dock 0.21.1 (the newest releases) both depend on egui 0.36.2; a direct egui 0.37 would put two eguis in the tree. Code uses `bevy_egui::egui`.
- **`capybara\profiles\<id>.toml`** added to the portable layout: the prompt asks for one small TOML file per profile that users can add, so profiles live in their own folder instead of inside `config.toml`.
- **`Levels` added to the `.bin` decoder's tables** (subset of columns: Id, Name, Act, Pal, LevelType, DrlgType). LvlPrest only names a LevelId; the LvlType and palette for a preset come from Levels. Without it every Median XL map would need a manual LvlType pick.
- **`d2-formats` is built in two sequential waves** (wave 1: ds1, txt, excel, tables, tbl; wave 2: palette, dt1, dc6, dcc, cof, animdata) because two agents may not edit one crate at once. `FormatError` was written by the lead before wave 1 so `d2-assets` can depend on it in parallel.
- **Real-file format tests read an extracted corpus** at `testdata\private\extracted\<profile>\` (made by a `d2-assets` example), because `d2-formats` cannot depend on `d2-assets`. The P0 acceptance tests that need the overlay live in `d2-assets` tests.
- **Known-names seed** lives at `assets/known-names/seed.txt` (outside any crate so it can be written while `d2-assets` is being built) and is embedded with `include_str!`.
- `.gitattributes` forces LF for text and marks game formats binary, because `core.autocrlf` is on and would corrupt binary fixtures.
- **Levels decodes from two files** (`levels.bin` + `leveldefs.bin`), via the added `excel::decode_levels_bin`: LevelType and DrlgType live in `leveldefs.bin`.
- **Excel source preference per game version**: 1.13c reads `.bin` first, 1.14d reads `.txt` first (each falls back to the other). Vanilla ships identical data in both forms, so this only matters for mods.
- **`LvlType.act` and `Level.act` are both 0-based**, like the DS1 act field (LvlTypes.txt stores 1-based acts).
- **DS1 parse limits**: the header and cell layers must be complete; zero-padding past EOF is tolerated only in the object/group/NPC blocks, up to 256 bytes (the shipped short files need 12 and 4). `to_bytes` refuses any model the file's version cannot store, rather than silently dropping data.
- **Phases overlap at the user's request (2026-10-19: "need this done fast").** While P0 wave 2 finishes, `capy-history` (P4), `capy-edit` (P2), `capy-index` (P1/P3 groundwork) and the `capy-app` shell (P1) are built in parallel against the API contracts in `docs/api/`. Each phase's acceptance tests must still pass before that phase is declared done, and each phase still ends with a summary to the user.
- **Writes outside `capybara\` (user-approved, 2026-10-19):** the sprite converter's "Export to…" and batch conversion write to a folder the user picks in a dialog. Everything else still stays under `capybara\`.
- **Agent models (user, 2026-10-19):** new agents run on Opus 5.5 with high effort instead of the original Sonnet/Haiku split. Agents already running were not restarted. Compile time remains the main bottleneck, so non-app agents test with `--exclude capy-app` and UI agents reuse the warm app worktree.
- **obj.txt licensing (user decision, 2026-10-19):** DS1Edit's `obj.txt` originates in Paul Siramy's unlicensed win_ds1edit; it stays embedded with attribution to both authors, the same basis DS1Edit ships it on. NOTICE states the provenance. Third-party notices: `THIRD-PARTY-LICENSES.md` (generated by `scripts/third_party_licenses.py`; ship it next to the exe; MPL-2.0 nucleo source pointer and OFL/UFL font terms included).
- Repo-local git identity `ZF <112085691+Nugwa628@users.noreply.github.com>` was set because none existed.

## Pending small fixes (lead decisions, not yet implemented)
- **Release:** `C:\games\median-xl` is only an auto-detect candidate on this machine; docs should not present it as the standard path.
- **Release round: DONE (2026-10-19).** Rename to Capybara's D2 Editor, capybara icon and version resource, loading screen ported from the user's MXL Capybara Editor (door animation from the game files, capybara, gold title), crash log and message box, CLI profile seeding via `d2_assets::installs`, `scripts/build-release.ps1` (static CRT, path checks, zip), capy-render `frames_of_info` and owned `ObjectSprites` adopted by the app, README, user guide and checklist current. Final pass on main 5b3f01b: 1062 tests with the full corpora, render oracle 87, edit oracle, all 8 GPU suites; release exe smoke-tested from the zip.
  - Backlog: an interrupted index build restarts from scratch; a `capybara\log.txt` for Bevy's log in release; a `d2-formats` font-table reader (the splash parses `.tbl` fonts in the app); the splash keeps the old program's timing (the door plays once, then frames 8/9 flicker).
- Release: one exe only, no separate console `capy.exe` (user, 2026-10-19). The GUI-subsystem exe attaches to the parent console; from cmd the prompt can return before CLI output finishes (documented; redirection and exit codes work).
- `capy-render`: a mod colormap that maps a visible index to 0 makes that pixel transparent here; DS1Edit keeps it opaque. No vanilla row does this. Match DS1Edit later.
- `capy-render`: DS1Edit re-derives DCC indices with `palette_find_closest` (first duplicate colour wins, `cof.c:149-170`), causing ~3 pixel differences on Coldcrow. Not reproduced; low value.

## Known gaps
- `Patch_D2.mpq`: 77 of 209 files (vanilla) and 53 of Median XL's copy remain unnamed after the harvest; every map, tile set and table the game references is named. A COF-derived name pass (objects/monsters) can recover sprite names later.
- `scripts/` tools (Python extractor, oracle renders) are dev-only and not shipped.

## Open questions
- ds1-studio parity (2026-10-20):
  - Answered 2026-10-20: ds1-studio keys are the default and rebindable, with no Capybara preset; full credit to RoofooEvazan with a README link; fastest order using the fewest tokens, with Haiku and Sonnet where Opus isn't needed; no self-updater and no Linux build.
- Answered on 2026-10-19:
  - Saves go next to the exe and are packed into the MPQ by hand.
  - Performance targets are smoke tests, not benchmarks, to save time and code.
