# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added
- **Edit DT1 tiles in the sprite editor.** **Open in editor** now works for a
  `.dt1` in the Viewer: every tile is a frame with all the paint tools,
  selections, palette, undo and onion skin. Pixels outside the tile's blocks
  are locked. Saving re-encodes only the changed blocks; everything else in
  the file is byte-identical.

## [1.2.1] - 2026-10-20

### Added
- **Add to this map.** Right-click a tile marked "!" in the Tiles panel (or
  Ctrl+click several) and choose **Add to this map...**. After a preview, the
  tiles are copied into the level type's own DT1, renumbered where they clash
  with any DT1 of the type, and the DT1 is registered in the first free
  LvlTypes File slot and the map's LvlPrest Dt1Mask, so the game really loads
  it. Saved with CHANGED.txt entries and a history snapshot; Ctrl+Z undoes the
  map's file list. On Median XL (1.13c) the compiled `lvltypes.bin` and
  `lvlprest.bin` are patched in place (only the File slot and Dt1Mask), so no
  recompiling is needed.
- **Export GIF window** (Viewer, Converter and Editor): choose a frame range
  (thumbnail strip or typed), the current, all or ticked directions (one GIF
  each, or one in sequence), speed (starts at the Viewer's), loop or play
  once, scale 1-4x, a transparent or solid background, with a live preview.
  COF objects and monsters export composed with blend modes, or as one part.
- `capy-paint`: `animated_gif_with` and `GifOptions` (loop, scale,
  background), `delay_for_fps`, `rgba_to_indexed`, `nearest_index`.
  `d2-formats`: `excel::patch_bin_field` (change one field of a compiled
  table, every other byte unchanged).
- Script commands `sprite-gif-open`, `sprite-gif-set`, `sprite-gif-export`.

### Changed
- The Map Editor's status bar shows the palette act the map is drawn with
  beside the DS1's act (`Act 1 · Act 2 palette`).
- The Tiles panel's "!" warning explains that the tile's DT1 is not loaded by
  the map's level type, so it would be invisible in game, and points to
  **Add to this map**.

### Fixed
- **Re-roll floors** and **Presets > Copy tiles to DT1** now register their DT1
  in LvlTypes and the map's Dt1Mask. In 1.2.0 they only changed the DS1's file
  list, which the game ignores.
- On 1.13c profiles the level-table tools (Add to game, Change level type...)
  refuse to save when the `.txt` has a different number of rows than the
  compiled `.bin` the game reads, instead of writing out-of-date tables.
- A COF (object or monster) shown in the Viewer could not be exported as a
  GIF at all; the Converter's export buttons stayed disabled. It now exports
  the composite.

## [1.2.0] - 2026-10-20

The Map Editor now has the interface, layout and map-editing tools of
[DS1 Studio](https://github.com/RoofooEvazan/ds1-studio) by RoofooEvazan,
rebuilt in Rust on Capybara's engine. All credit for that design goes to
its author (see NOTICE).

### Changed
- **The Tiles panel is tidied like DS1 Studio's:** category chips, the DT1
  drop-down, the filter box and then the thumbnails. The scope buttons,
  thumbnail size, Saved searches, Recent and Favorites moved into a
  **Search options** section (closed at start); the query language still
  works in the filter box.
- **The Map Editor looks and works like DS1 Studio** (Sprites mode is
  unchanged and is reached with the **Sprites** button in the top bar): a
  ribbon (Home, View, Map, Game, Diagnostics, Help) replaces the menus and the tool strip; the
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
  Backspace (show all), Ctrl+1-7 (active layer), U (rectangle), L (fill
  area), Ctrl+H (find & replace), Tab / Shift+Tab and F1-F6 (view modes),
  W / A / H / Shift+L (walkability, automap, roof hiding, level light
  views), Shift+H (roofs as if inside), R (8x8 rooms), Z (game view), 9 /
  Shift+9 (lower walls), PrintScreen (copy the view). DS1 Studio's keys
  for features that come later (Shift+Delete, P) are reserved.

### Added
- **Automap and Level light view modes are editable.** In Automap, pick
  Paint, Pick or Clear in the panel on the map and click cells: the tile
  gets the picture (cel) in AutoMap.txt, as in the Automap editor, with a
  live preview coloured by picture and a **Save AutoMap.txt** button. In
  Level light, edit the map's Levels.txt Intensity, Red, Green and Blue and
  save them; the preview is now a real colour multiply in the tile shader
  instead of a darkening approximation.
- **Re-roll floors can use floors from other DT1 libraries** of the level
  type (or of every level type): the picked floors are copied under free
  numbers into `<map>_custom.dt1`, which is saved next to the map and listed
  in it.
- **Stack painting (Alt with the brush):** Alt+Paint, or Alt+Rectangle,
  puts each cell on the next free wall or floor layer instead of replacing
  the tile, adding a layer (up to 4 walls, 2 floors) when needed.
- **Walkability tools:** paint whole cells (the DS1 unwalkable flag) or
  single sub-tiles (blockers and tile copies in a per-map `<map>_walk.dt1`),
  block or clear, with undo; and the **No-spawn area** planner for the
  selection or the whole map.
- **Map editing (DS1 Studio parity, P9):** Rectangle (U) and Fill area (L)
  paint and erase next to Freehand; random-mix painting (Ctrl+click several
  tiles in Tiles: Paint picks one per cell); Find & replace tiles (Ctrl+H);
  Re-roll floors and Reassign tiles (Map tab); New map (saved at once to the
  saves folder); resize by dragging a map edge with the Select tool;
  Shift+wheel steps through the tiles stacked in a cell; double-click
  selects every copy of a tile; the Cell section shows each layer's raw
  dword and bytes with Hidden / Unwalkable / Block jump checkboxes; Tags &
  groups edits object groups (add, move, resize, delete; undoable); the
  Objects window lists object checks (outside the map, duplicates, unknown
  ids); Export ▾ writes the map or the selection as PNG, and PrintScreen
  copies the map view to the clipboard.
- **Views and overlays (P10):** view modes Tiles, Objects, Walkability,
  Automap, Level light and Roof hiding, cycled with
  Tab / Shift+Tab or F1-F6; the walkability overlay and a walkable-area
  badge (DS1 Studio's number and colours); spawn regions, warps (click a
  label to open the target level's map) and roof-hiding areas; the 8x8
  game rooms grid (R); Game view framing at a chosen screen size (Z);
  water, lava and other animated floors play in the map (Animate,
  Shift+N, now covers tiles too).
- **Tiles column like DS1 Studio:** Floors / Walls / Shadows / Objects /
  Roofs / Special chips, a DT1 drop-down for the map's libraries and the
  filter box (Capybara's query language still works; the chips and the
  drop-down edit its `type:` and `file:` tokens). Under the Show bar,
  Upper walls / Lower walls chips and a DT1 wall-categories row decide
  which group each library's walls belong to.
- The minimap shows the map itself (drawn small in the background) instead
  of a schematic.
- **Tile libraries** (Map tab): a DT1 manager (open, duplicate, rename,
  recolour by hue / saturation / brightness / tint), tile settings with DT1
  Tools `.ini` export and import, collision-flag painting on a tile's 5x5
  sub-tiles, pixel painting on a tile block, Make act-safe, and a review of
  duplicate, clashing and mixed-colour libraries with "collect the placed
  tiles into an own DT1".
- **Presets** (right column tab): the preset library, Save selection as
  preset, Place (preview, then click; undoable), a 20x20 preset builder and
  Suggest.
- **Game tab**: Add to game, Change level type, Type packages, Entering text,
  Cube recipe, Custom object (writes the DC6, the COF and the objects.txt row), an Automap editor
  and a **Tables** spreadsheet editor with column help. Tables the game only
  has as compiled `.bin` cannot be opened yet. Edited files go to your saves
  with a CHANGED.txt entry and a history snapshot, like maps.
- **Diagnostics tab**: Compatibility check with one-click fixes, Crash log
  reader, Unused DT1s with archiving inside `capybara\archive`, Restore
  assets.
- **Map packages**: Home > Export > Map package (.zip), Import > Map package...
  (merge into the saves, table rows applied through the type-package merge)
  and Import > DS1 with its DT1s...
- **Themes**: DS1 Studio's nine themes (DS1 Studio is the default), custom
  themes from share codes (DS1 Studio / PD2 Filter Forge format), in
  View > Theme and Preferences. Segoe UI is used when Windows has it; icons
  come from the Lucide icon font (ISC).
- Right-click menu on the map (Cut, Copy, Paste, Delete, Select all,
  Deselect, Move to layer, flags, Fit), and Help > About.

- **COF writer.** `Cof::to_bytes` writes every COF of the vanilla and Median
  XL corpora back byte-for-byte. Custom object now writes the COF next to the
  DC6 and the objects.txt row (all three land in your saves with CHANGED.txt
  entries).
- **Compatibility check, more rules**: players arriving by portal, warp
  links and ways out, roof hiding (Pops), walkable edges that border void,
  NPCs on unwalkable ground and objects stacked on one sub-tile (one-click
  move fixes), and rooms that are only partly no-spawn. **Rename the long
  paths** now works: each file is copied to its shorter path and LvlTypes /
  LvlPrest are updated. "Open" fixes open the Tables window at the row.
- **MCP server**: `CapybarasD2Editor.exe mcp --profile <id>` lets AI tools
  open, edit, check, render and save maps over stdio, with DS1 Studio's tool
  names (`list_maps`, `open_map`, `new_map`, `paint`, `set_cell`,
  `copy_area`, `find_replace`, `reroll`, `resize_map`, `set_cell_flags`,
  `add_object`, `edit_objects`, `undo`, `redo`, `check_map`, `render_map`,
  `save_map`, `regions`, `no_spawn_area`, `list_tiles`, `list_placeable`,
  `add_layer`, `clear`, `history`, plus `map_info`, `get_cells`,
  `list_objects`). Saves go to your saves folder with CHANGED.txt entries,
  like the editor. See `docs/cli.md`.
- **Preset packages**: Export... saves the selected preset (or a category, or
  all) as a .zip with the mod's own tile libraries they use; Import... adds
  them (a clashing library is kept and the import renamed). "Copy tiles to
  DT1" copies the tiles a preset needs and the open map lacks into the map's
  own DT1. Presets now record which DT1 each tile comes from.

### Fixed
- Tile labels, list rows and the Log's INFO tag used fixed light greys that
  were hard to read on the light themes (Parchment, Snow); they follow the
  theme now.

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
