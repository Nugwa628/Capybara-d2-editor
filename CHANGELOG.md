# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Changed
- **The Map Editor looks and works like DS1 Studio** (Sprites mode is
  unchanged and is reached with the **Sprites** button in the top bar): a
  ribbon (Home, View, Map, Help) replaces the menus and the tool strip; the
  map list sits on the left grouped by folder with counts; the right column
  holds the Tiles browser with collapsible Cell, History, Tags & groups,
  Layers and Map sections; a Show bar of layer chips (Shift-click shows one
  layer only) and a minimap sit on the map; the status bar describes the
  current tool. Side columns resize and fold (Shift+F folds both). The Log,
  Objects and Sprite preview panels are floating windows; Objects also fills
  the right column while the Objects tool is active. The old dock layout of
  the Map Editor (`layout.json`) is no longer used.
- **Ctrl+K is now the command finder** (every command by name). Searching
  every asset is its last row, and the same query language works in the
  Tiles search box. In Sprites mode Ctrl+K still searches every asset.
- **Keys follow DS1 Studio** and can be changed in **Help > Shortcuts**
  (saved in `config.toml`). Old -> new:

  | Action | Old | New |
  |---|---|---|
  | Select box | M | V |
  | Lasso | L | Q |
  | Select by tile ID | T | T |
  | Select identical | S | S |
  | Paint | P | B |
  | Erase | E | E |
  | Eyedropper (Pick) | I | I |
  | Objects tool | O | O (also F2) |
  | Search every asset | Ctrl+K | Ctrl+K, then the last row |
  | Object markers | (menu) | M |
  | Object sprites | (Layers panel) | N |
  | Animate objects | (menu) | Shift+N |
  | Fit map | (menu) | F |

  Unchanged: Ctrl+S, Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z, Ctrl+C, Ctrl+X, Ctrl+V,
  Ctrl+A, Ctrl+D, Delete, Esc, Enter, G. New: Shift+O (toggle the Objects
  tool), F1 (leave it), Home (100%), = and - (zoom), K (minimap), Shift+F
  (just the map), 1-0 (show/hide layers), Shift+1-0 (show one layer only),
  Backspace (show all), Ctrl+1-7 (active layer). DS1 Studio's keys for
  features that come later (U, L, Ctrl+H, Shift+Delete, Tab, W, A, R, H,
  Z, P, F3-F6, ...) are reserved.

### Added
- **Themes**: DS1 Studio's nine themes (DS1 Studio is the default), custom
  themes from share codes (DS1 Studio / PD2 Filter Forge format), in
  View > Theme and Preferences. Segoe UI is used when Windows has it; icons
  come from the Lucide icon font (ISC).
- Right-click menu on the map (Cut, Copy, Paste, Delete, Select all,
  Deselect, Move to layer, flags, Fit), and Help > About.

## [1.1.0] - 2026-10-20

### Added
- **Convert to palette** (Sprites tab, and right-click a tile set in the Tiles
  browser): convert DT1 tiles and DC6/DCC sprites from one palette to another,
  one file or a batch. Shows every used colour with its match and error,
  lets you override any colour, change the matching settings and compare
  before/after; nothing changes until you press Convert. "All acts (shared
  colours only)" makes art that looks the same in every act. DT1 and DC6 keep
  every byte except the colours.
- The Sprites tab opens DT1 tile sets.
- **Palette memory:** each file remembers its palette; a badge shows whether
  the palette is remembered, suggested by the folder or not remembered, and
  saving or importing with a different palette asks first.

### Fixed
- Saving from a batch while saving a map could drop an entry from
  `CHANGED.txt`.

## [1.0.0] - 2026-10-19

### Changed
- Renamed to **Capybara's D2 Editor**; the exe is `CapybarasD2Editor.exe`
  (the data folder stays `capybara\`).

### Added
- Capybara program icon and version information in the exe; window and
  taskbar icon.
- Loading screen: Diablo II's door-opening animation from your game files
  with the capybara in the doorway and the program name on the plate
  (Settings > Start-up > Show loading screen; click or Esc skips it).
- Crash reporting: `capybara\crash.log` and an error box instead of a silent
  exit (for example when no GPU driver is available).
- The command line works on a fresh folder: it finds the installs and writes
  the default profiles itself.
- `scripts/build-release.ps1`: static-CRT release build (no Visual C++
  runtime needed), checks for leaked build paths, release zip with licenses
  and docs.
- **P5 complete.** Objects and NPCs drawn in the map view (DS1Edit order and
  blends, matches the CPU renderer on all 18 oracle maps), animation at 25 Hz
  (View > Animate objects), object placement picker with animated preview,
  object properties, NPC path editing (add, insert, move, delete, actions),
  File > Import DS1Edit ini mappings.
- Robustness audit of the library crates: fuzz tests on every parser; bounded
  allocations, loops and recursion on hostile files (`docs/robustness-audit.md`).
- **P4 complete.** History panel: autosave (dirty maps only, never touches saves),
  per-map timeline, named checkpoints, visual tile diff overlay and read-only
  snapshot view, undoable restore, retention with prune, gix milestones.
  Settings dialog (gamma, default layers, autosave, retention, profiles) and
  "Add game install..." with registry detection; first run prefers vanilla.
- CLI: renders draw objects; overrides, version labels, exit codes, help aliases.
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

### Fixed
- The map was drawn half a cell right of its overlays (hover, selection, markers).
- A map could stay at "Loading" after a context refresh that changed nothing.
- Pasting many objects no longer freezes on the anchor-conflict check.
