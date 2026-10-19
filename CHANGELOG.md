# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added
- **P3 complete.** Tiles panel asset browser (virtualized thumbnails, Ctrl+wheel
  size, map/profile/sprite scopes, set tile as brush), Ctrl+K command palette with
  filter tokens and inline query errors, recents, favorite groups with drag
  reorder, pinned saved searches, aliases, tags, notes, find-similar and used-in.
- README, user guide and CLI reference.
- **P6 complete: Sprites mode.** Top-level Map Editor / Sprites tabs; DC6/DCC/COF
  viewer (playback at AnimData speed, palettes, gamma, COF layers, box/anchor
  overlay); converter (image import with quantize/dither/transparent protection,
  frame/sheet/GIF/indexed-PNG/palette export, batch trees, save to game path);
  palette-locked sprite editor (pencil, shapes, fill, gradient, shading, outline,
  selections, frames, onion skin, DCC cell check); sprite preview box in the map
  editor; single-exe CLI (`CapybaraMapEditor.exe render ...`).
- **P2 complete.** Editing UI: box/lasso/tile-ID/identical selection, paint,
  erase, eyedropper, object move/delete, cross-map clipboard with conflict
  preview, move-to-layer with version upgrade, flags, resize, layer actions,
  undo/redo, dirty tracking, save to `capybara\saves` with CHANGED.txt and
  history snapshots, mark-as-packed; `CAPY_SCRIPT` automation hook.
- Objects and NPCs render in DS1Edit's order with shadows and blends; Median XL
  NPC tables decode (100% resolved).
- **P1 complete.** Textured Bevy map view (single draw call per map, premultiplied
  atlas, reference shadow blend, DS1Edit zoom grid) matching the CPU renderer on
  all 18 oracle maps; background asset index with progress bar and cache.
- `capy-render`: tile lookup and DS1Edit variant rule, reference draw order, CPU
  compositor, oracle comparison (18/18).
- `d2-assets`: map catalog (LvlPrest/Levels/LvlTypes, tile-coverage ranking,
  Levels.Pal palettes, legacy DS1Edit ini import) and object catalog
  (objects/NPCs to COF tokens, modes, parts, skins; Median XL monster tables).
- `d2-formats`: DC6/DCC encoders (all game sprites round-trip), monster-table
  `.bin` decoders, `dcc::cell_spans`.
- `capy-index`: SQLite index with FTS, thumbnails, pHash, used-in links, search
  with filter tokens (P3 query test 26/26), user.db, saves refresh.
- `capy-edit`: patches with undo/redo, selections, clipboard with conflict
  preview, layer moves, object and NPC-path ops (P2 oracle-load test 22/22).
- `capy-sprite`, `capy-paint`: sprite converter and palette-locked editing engine.
- `capy-cli`: headless `render`, `index`, `maps`, `profiles`.
- **P0 complete.** Acceptance (vanilla 1.14d + Median XL): every DS1 round-trips
  byte-identically (2,276 + 2,424), every DT1 parses (six unreferenced 4.1 files
  excepted), every known name resolves, `.bin` equals `.txt`, every Median XL
  LvlTypes/LvlPrest reference resolves.
- `d2-formats`: palettes (`pal.dat`, PL2), DT1 tiles, DC6, DCC, COF, AnimData.
- `d2-assets`: known-names harvest, `load_excel` (`.bin`/`.txt` by game version).
- `capy-history`: atomic saves with `CHANGED.txt` and mark-as-packed, deduplicated
  snapshot store with retention, gix milestones; crash-safe (kill test).
- `scripts/oracle_renders.py`: reference renders for the P1 oracle test.
- `d2-assets`: read-only MPQ reader (v1-v4 headers, encryption and FIX_KEY,
  implode, zlib, bzip2), loose-folder and stub CASC sources, an N-layer
  overlay, TOML game profiles (Vanilla, Median XL), a known-names container
  with an embedded seed list, and an `extract` example.
- `d2-formats`: DS1 reader/writer for every version (byte-identical round
  trip on all vanilla and Median XL maps), `.txt` tables, compiled excel
  `.bin` decoder (LvlTypes, LvlPrest, LvlWarp, Objects, ObjPreset, Levels),
  typed LvlTypes/LvlPrest/Levels views, `.tbl` string tables.
- `d2-assets` wave 2: `KnownNames::harvest` (listfiles, seed, LvlTypes/LvlPrest
  file columns and every DS1's tileset references, to a fixed point, keeping only
  names that resolve), `tables::load_excel` (`.bin`/`.txt` per game version),
  `GameVersion::excel`, and the P0 acceptance tests (DS1 round trip, names resolve,
  `.bin` vs `.txt`, Median XL references).
- `scripts/extract_fixtures.py`: independent Python MPQ extractor used to
  cross-check the Rust reader.
- `docs/specs/`: reference guides cross-checked against the reference source.
- Cargo workspace skeleton (`d2-formats`, `d2-assets`, `capy-index`, `capy-edit`,
  `capy-history`, `capy-app`), LICENSE (MIT) and NOTICE.
- `d2-formats` wave 1: `Grid`, `IndexedImage`, DS1 reader/writer (every version,
  byte-identical round trip on the full vanilla and Median XL corpora), `.txt`
  tables, excel `.bin` decoders (LvlTypes, LvlPrest, LvlWarp, Objects, ObjPreset,
  Levels), typed level tables and `.tbl` string tables.
