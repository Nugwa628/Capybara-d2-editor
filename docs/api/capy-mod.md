# `capy-mod` public API

Game-data editing for mods: pure functions over `d2_formats::txt::TxtDoc` and
`d2_formats::tbl::TblDoc` values passed in by the caller. No file I/O, no UI.
Every function leaves its inputs untouched and returns the edited docs plus a
human-readable change list. Saving to `capybara\saves`, `CHANGED.txt` and
history are the caller's job.

## Level tools

Modules: `level_tables`, `add_map`, `change_type`, `type_package`,
`entry_text`, `cube`, `custom_object`, `lvl_error`.

Behaviour reference: ds1-studio's `src/game/{addToGame,levelTables,
changeLevelType,typePackage,entryText,cubeRecipe,mapRecipes,customObject}.ts`
(re-implemented, not copied). Rules that hold for all three level tables:

* `Levels.Id`, `LvlPrest.Def` and `LvlTypes.Id` equal the row's index among
  data rows (blank lines and the `Expansion` separator do not count). Rows are
  only ever **appended** (before trailing blank lines); nothing is renumbered.
  An edit refuses (`ModError::Refused`) when a table already breaks this.
* Edits use `TxtDoc::set_cell` / `insert_row`, so every untouched row stays
  byte-identical. A table with no change compares equal to its input.
* Columns a table does not have are skipped (profile differences). All of this
  works on the `.txt` forms; a profile with only `.bin` tables (leveldefs.bin
  holds Levels' DrlgType, LevelType, sizes, Vis and Warp) must be decoded and
  rewritten by the caller.

```rust
// lvl_error
pub enum ModError { Refused(String), Format(d2_formats::FormatError) }
pub type Result<T, E = ModError> = std::result::Result<T, E>;

// level_tables
pub struct LevelTables { pub prest: TxtDoc, pub levels: TxtDoc, pub types: TxtDoc, pub lvl_sub: Option<TxtDoc> }
pub enum TableName { Levels, LvlPrest, LvlTypes }              // .file_name() -> "Levels.txt"...
pub struct FieldChange { table, row: String, column: String, from: String, to: String, why: Option<String> } // .summary()
pub const MAX_TILE_PATH: usize = 41; MAX_LEVEL_STRING = 39; MAX_LAYER = 100; DEFAULT_ENTRY_FILE = "A5L1";
pub fn level_act(id: u32) -> u8;                 // 109.. => 4 (Act 5)
pub fn tile_path_problem(rel: &str) -> Option<String>;
pub fn preset_rows_for(prest: &TxtDoc, map: &str) -> Vec<usize>;   // File1..6 match, any path form
pub fn ensure_type_slots(types: &mut TxtDoc, type_row: usize, dt1s: &[String]) -> Result<Slots>; // reuse or fill free File slots
pub fn mask_of(slots) -> u32;  pub fn mask_for(types, type_row, dt1s, old_mask) -> u32;
pub fn append_own_type(types: &mut TxtDoc, from_row, name, dt1s) -> Result<OwnType>; // copy of a type with just these DT1s
pub fn free_offset(levels, act, (w, h), except_row) -> (x, y);     // clear of other levels, inside automap range
pub fn safe_layer(levels) -> i64;  pub fn map_level_type(tables, map, fallback) -> (level_id, type_id);
```

### 1. Add a map to the game (`add_map`)

```rust
pub enum AddMode { New, Existing }
pub struct AddMapInput { mode, level_id, name, map_rel /* under data/global/tiles */, width, height,
    used_dt1s: Vec<String>, pop_count, pal_act: Option<u8>, new_type: bool }
pub struct AddMapOutcome { tables: LevelTables, changes: Vec<FieldChange>, warnings: Vec<String>,
    level_id, new_level_id: Option<u32>, type_id, own_type: bool, preset_def, touched: BTreeSet<TableName> }
pub fn add_map_to_game(tables: &LevelTables, input: &AddMapInput) -> Result<AddMapOutcome>;
```

* `New`: appends a level copied from `level_id` (Id = next record, Act 4 = Act 5,
  size = DS1 size - 1, `OffsetX/Y` from `free_offset`, DrlgType 2, Vis/Warp
  cleared, LevelName/LevelWarp = the name cut to 39 chars, EntryFile `A5L1`
  unless the template is an Act 5 level) and an LvlPrest row (Def = next
  record, `File1` = the map, `Dt1Mask` from the type's slots). Refused when the
  map is already in LvlPrest.
* A level type other levels use keeps its tile list: when the map needs DT1s it
  lacks (or `new_type`), the level gets a **type of its own** (copy of the
  template's Act/Expansion, File 1.. = the map's DT1s).
* `Existing`: the preset level now loads the map (first claiming LvlPrest
  row); `Dt1Mask` follows. **Idempotent**: a second call returns `changes`
  empty and `tables` equal.

### 2. Change a map's level type (`change_type`)

```rust
pub struct ChangeTypeInput { map_rel, level_id: Option<u32>, target_type: u32, copy: bool, used_dt1s, copy_name: Option<String> }
pub struct ChangeTypeOutcome { tables, changes, warnings, type_id, level_id, copied: bool }
pub struct LevelTypeChoice { id, name, act, files, users, copy_only: Option<String> }
pub fn map_level(tables, map_rel, want: Option<u32>) -> Result<MapLevel>;
pub fn level_type_choices(tables, map_rel, used_dt1s, level_id) -> Result<Vec<LevelTypeChoice>>;
pub fn change_level_type(tables, &ChangeTypeInput) -> Result<ChangeTypeOutcome>;
```

Refused for shared presets (LevelId 0), maze/outdoor levels, and a level that
already uses the target. A target that lacks the map's DT1s and is used by
other levels (or has no safe free slots) is only offered as a copy; `copy`
makes a new type of its own. The level's `Dt1Mask` becomes exactly the map's
DT1 slots. AutoMap.txt rows are not touched (a warning says so when a copy is
made); the automap module owns that.

### 3. Type packages (`type_package`)

```rust
pub struct TypePackage { type_id, type_name, type_row: PackageRow, dt1s: Vec<PackageDt1 {slot, path}>,
    maps: Vec<String>, levels: Vec<PackageRow>, presets: Vec<PackageRow> }   // serde Serialize/Deserialize
pub fn maps_of_type(tables, type_id) -> Vec<String>;
pub fn export_type_package(tables, type_id) -> Result<TypePackage>;
pub fn import_type_package(tables, &TypePackage) -> Result<ImportOutcome>; // { tables, changes, warnings, type_id, level_ids }
```

Zip packaging (DT1/DS1 bytes, README, `txt/` files) is the caller's job. Import
merges by name: type (reuse, else append with the next Id; missing DT1s go in
free slots, refusing if none), levels (reuse by Name and point at the type, else
append in Act 5 with links cleared), presets (update the row already listing
the map, else append). `Dt1Mask` bits are **remapped** from the package's slots
to the importer's by DT1 path. Importing twice changes nothing.

### 4. Entering text (`entry_text`)

```rust
pub fn set_area_name(levels: &TxtDoc, strings: &TblDoc, level_id, text, key: Option<&str>) -> Result<EntryTextOutcome>; // { levels, strings, key, changes }
pub fn set_entry_file(levels: &mut TxtDoc, level_id, name) -> Result<String>;   // data/local/ui/eng/expansion/<name>.dc6
pub fn suggest_entry_name(level_name, taken: impl Fn(&str) -> bool) -> String;
pub fn levels_using_entry_file(levels, name) -> Vec<u32>;
pub struct Font; impl Font { pub fn load(&Dc6, tbl: &[u8]) -> Result<Font>; pub fn glyph(&self, char) -> Option<Glyph> }
pub fn render_entry_text(height, glyph: impl Fn(char) -> Option<Glyph>, text) -> EntryImage;
pub fn entry_frames(&EntryImage) -> Vec<IndexedImage>;  // 256-px frames
pub fn entry_dc6(&EntryImage) -> Result<Dc6>;
```

`set_area_name` sets `LevelName` and `LevelWarp` to one string key (the level's
current key when no other level names it, else `capy_lvl_<id>`, or `key`) and
sets the text in the given `.tbl` (normally `patchstring.tbl`, path constant
`PATCH_STRINGS`). Idempotent. The loading-screen image functions rebuild the
line from a game font as the community's txt2dc6 does.

### 5. Cube recipes (`cube`)

```rust
pub fn plan_cube_item(tables: &CubeTables { misc, cube, levels: Option, strings: Option<TblDoc> }, input: &CubeItemInput) -> Result<CubeOutcome>;
pub fn same_recipe(cube, inputs) -> Option<String>;   pub fn normalize_input(&str) -> String;
pub fn template_risk(type1, type2) -> Option<TemplateRisk>;   // Blocked (elixirs) / ModType
pub fn map_item_level(misc, row, levels) -> Option<u32>;      // PD2-style: pSpell + len = level id
pub fn find_map_recipes(&RecipeTables, map_path) -> Vec<MapRecipe>;   // enabled recipes making a map's items
pub fn tagged_recipes(cube) / tagged_items(misc) / remove_rows(doc, rows)
```

Appends a copy of a template Misc row (own code in name/normcode/ubercode/
ultracode, `spawnable` 0) and a CubeMain row; refuses elixir templates,
unconfirmed mod item types, a code already in Misc, and a recipe whose inputs
another enabled recipe already has. A map-item template makes an item that opens
`map.level_id` with its name in the `.tbl` (key `capy_<code>`), optional new
level title, and output/lvl/ilvl copied from the mod's own map recipe. Rows
carry the tag `(Capybara)` (the reference editor's `(DS1 Studio)` is also
listed).

### 6. Custom PNG objects (`custom_object`)

```rust
pub fn game_rows(objects) -> Vec<usize>;   pub fn preset_row(presets: &[Vec<i32>], act, id) -> i32;
pub fn object_slots(presets, objects, act, uses: &HashMap<usize,u32>, groups: &HashSet<usize>) -> Vec<ObjectSlot>; // .free
pub fn free_token(objects, folders: &HashSet<String>) -> Result<String>;
pub fn custom_object_row(objects, row, &CustomObjectOptions) -> Result<ObjectRowOutcome>; // { objects, changes }
pub fn split_strip(w, h, pixels, count) -> Result<Vec<IndexedImage>>;   pub fn guess_frames(w, h, solid) -> u32;
pub fn custom_object_files(token, frames, feet, anchor_x) -> Result<ObjectFilePlan>; // { cof: CofPlan, cof_bytes, dc6_path, dc6: Vec<u8> }
```

The row becomes a plain one-mode decoration (marked `Capybara custom: <name>` in
`description - not loaded`; the reference editor's mark is recognised). The DC6
is encoded with `Dc6::from_frames` and the COF with `Cof::to_bytes`
(`cof_bytes`, written at `cof.path`; `CofPlan` is the readable form: path, box,
one TR layer, rate 256). Converting a PNG to palette indices and the
object-use scan over DS1s are the caller's.

### Tests

`tests/level_corpus.rs` (needs `CAPY_PRIVATE_DIR`; skipped when absent) adds a
map to vanilla and Median XL tables and resolves it through
`d2_assets::MapCatalog` to the right LvlType and DT1 list; checks unchanged rows
are byte-identical, idempotence, level-type changes, and type-package
export/import between the two corpora. `tests/other_tools.rs` holds the cube,
entering-text and custom-object cases translated from the reference editor's
vitest files.
## Tile tools (P11)

Pure functions over `d2_formats::dt1::Dt1` models (data in, edited model plus
change list out; no I/O, no UI). The caller reads and writes files and uses
`Dt1::parse` / `Dt1::to_bytes`. Modules `tiles_*`.

```rust
// tiles_flags: sub-tile collision flags (docs/collision-flags.md of ds1-studio)
pub const COLLISION_FLAGS: [CollisionFlag; 8];   // bit, name, advanced, help
pub const WALKABLE_MASK: u8 = 0x0D;
pub enum FlagOp { Add(u8), Remove(u8), Set(u8), MakeWalkable }
pub fn describe_collision(u8) -> String;  pub fn collision_hex(u8) -> String;
/// `subtiles` are y*5+x. Checked first (nothing changes on error); only changed bytes are returned.
pub fn edit_subtile_flags(&mut Dt1, tiles: &[usize], subtiles: &[usize], FlagOp) -> Result<Vec<FlagChange>, FlagEditError>;
pub fn set_tile_flags(&mut Dt1, tile: usize, [u8; 25]) -> Result<Vec<FlagChange>, FlagEditError>;

// tiles_dt1: manager operations
pub fn duplicate_dt1(&Dt1) -> Dt1;  pub fn copy_tiles(&Dt1, &[usize]) -> Result<Dt1, _>;
pub fn append_copies(&mut Dt1, &[usize]) -> Result<Vec<usize>, _>;  pub fn without_tile(&Dt1, usize) -> Result<Dt1, _>;
pub fn plan_rename(path: &str, new_stem: &str) -> Result<RenamePlan, _>;     // name rules + 41-char tile path limit
pub fn rename_references(cells: &mut [String], old_rel, new_rel) -> Vec<usize>; // LvlTypes File n / LvlPrest Filen cells
pub fn custom_name_problem(name, folder) -> Option<String>;  pub fn suggest_short_path(rel, max) -> String;
/// Rewrites only pixel literals of the chosen tiles (None = all); 0 stays 0 and nothing maps to 0.
pub fn recolor_tiles(&mut Dt1, Option<&[usize]>, &[u8; 256]) -> Result<Vec<TileChange>, _>;
pub fn hue_remap(&Palette, &HueOptions) -> [u8; 256];   // hue/saturation/brightness/tint, as dt1Edit.ts
pub fn swap_remap(&Palette, from, to, tolerance, allowed) -> [u8; 256];
/// Pixel painting: re-encodes through Block::with_pixels.
pub fn set_block_pixels(&mut Dt1, tile, block, &IndexedImage) -> Result<TileChange, _>;
pub fn edit_block_pixels(&mut Dt1, tile, block, impl FnOnce(&mut IndexedImage)) -> Result<TileChange, _>;

// tiles_settings: DT1 Tools .ini
pub fn read_settings(&Tile) -> TileSettings;  pub fn apply_settings(&mut Tile, &TileSettingsPatch) -> bool;
pub fn write_settings(&mut Dt1, &[(usize, TileSettingsPatch)]) -> Result<Vec<usize>, usize>;
pub fn dt1_to_ini(&Dt1) -> String;  pub fn parse_dt1_ini(&str) -> (Vec<IniBlock>, Option<usize>);
pub fn apply_ini(&mut Dt1, &[IniBlock]) -> (changed, missing);

// tiles_actsafe: make act-safe (capy_paint remap planner, TargetSet::SharedActColors)
pub fn shared_act_colors(&[Palette]) -> [bool; 256];      // Act 0 idea: same colour in every act
pub fn count_unsafe_pixels(&Dt1, Option<&[usize]>, &Safe) -> u64;
pub fn make_act_safe(&mut Dt1, home: &Palette, Option<&[usize]>, &ActSafeOptions) -> Result<ActSafeReport, _>;

// tiles_review: duplicates, custom DT1, own tiles
pub fn map_tile_usage(&Ds1) -> BTreeMap<TileId, u32>;  pub fn cells_using(&Ds1, TileId) -> Vec<(u32, u32)>;
pub fn duplicate_dt1s(&[(String, &Dt1)]) -> Vec<DuplicateDt1>;  pub fn clashing_pairs(..);  pub fn mixed_versions(..);
pub fn plan_custom_dt1(&[TilePick], sources, taken: &BTreeSet<TileId>) -> CustomDt1Plan;
pub fn build_custom_dt1(&CustomDt1Plan, remap_for: Option<&dyn Fn(usize) -> Option<[u8; 256]>>) -> Dt1;
pub fn own_tiles_path(map_path, Option<(type_name, listed)>, own_files) -> String;  pub fn append_tiles(..);

// tiles_usage: unused assets and archive plan
pub fn build_dt1_graph(&[MapRefs], &BTreeMap<String, Vec<TileId>>, protected) -> Vec<Dt1Asset>;
pub fn unused_report(&ReferenceGraph) -> UnusedReport;   // blocked (empty lists) when the scan had errors
pub fn plan_archive(paths, stamp) -> ArchivePlan;  pub fn plan_restore(&ArchiveManifest, exists) -> RestorePlan;
```

Behaviour:

- **Flags** index `Tile::subtile_flags` (`y * 5 + x`). Only changed bytes
  are written, so a flag edit changes only that tile's 25 flag bytes in the file.
- **Recolour** never touches control bytes; recolouring every tile equals
  `d2_formats::remap::remap_indices` on the file bytes. Blocks are per tile in
  the model, so recolouring a selection never changes an unselected tile.
- **Act-safe** keeps colours outside 225..=254 (or the caller's `Safe::Only`
  mask) and maps the rest to the nearest safe colour of `home` (RGB distance by
  default, as the community Act 0 conversions). Afterwards no pixel uses 225..=254.
- **Settings .ini**: 5 `floor_flag` rows are sub-tile rows `y = 0..5`; hex for
  direction/orientation/indices/frame/unknown, decimal for roof_y/sound/animated.
  Importing an unchanged `.ini` changes nothing.
- **Unused report** is conservative: errors block it, protected assets are listed
  apart. Archive paths are `capybara/archive/<stamp>/<original path>`; paths that
  escape or are already inside `capybara/` are refused; restore skips originals
  that exist again (conflicts). The app performs the moves.
- Not ported: `presetPackage` (map packages), `preparePresetLibrary` (copying a
  preset's tiles into the map's own-tiles DT1; use `plan_custom_dt1`), the
  game-table readers of `assetUsage.scanAssetUsage`.


## Automap, compatibility and crash logs

Pure functions; the caller passes parsed data and applies the results. Behaviour reference: ds1-studio `automap*.ts`, `compat.ts`, `crashLog.ts`.

### `automap`

- `AutomapTable::parse(&TxtDoc)` gives `rules: Vec<Rule>` (row, level, code, style, start, end, cels up to the first -1) and `levels`. `find_rule(level, orientation, main, sub)` is the game's first-match lookup; `level_for(type_name, act, type_id)` picks the `LevelName` for a level type; `unknown_levels()`, `reads_level_numbers()`.
- `pieces(&Ds1, &AutomapTable, level) -> Vec<Piece>`: each visible floor/wall tile with its rule row and cel.
- Edits on `&mut TxtDoc`, each returning `EditSummary { changes: Vec<Change>, summary }` (an empty `changes` means the table already had that content, so every edit is idempotent):
  `set_cel(doc, level, orientation, style, sub, cel, Scope::{Sequence,Style})` (cel < 0 clears by removing or splitting rules, never by writing -1; a wildcard rule is `AutomapError::Wildcard`), `set_cel_for_tiles`, `clear_tiles` (all or nothing), `suggest` / `with_suggestions` / `apply_suggestions`, `usual_cels`.
  `plan_clear(pieces, CellRect) -> ClearPlan { tiles, shared_elsewhere }`: tiles also used outside the area need a fresh tile id first (DT1/DS1 edit, done by the caller).
- Unchanged rows keep their bytes (edits go through `TxtDoc`).
- `automap::import`: `read_rows(bytes)`, `names_by_number(&TxtDoc)`, `merge_rows(&mut TxtDoc, &AutomapSource, &HashMap<String, Option<String>>)`; rows already present are skipped.
- `automap::safety`: `invalid_rows(&TxtDoc)` (Cel1 negative/non-integer), `remove_invalid_rows`, `validate_save(path, bytes)` (refuses a broken AutoMap.txt).
- `automap::style`: `AutomapStyle` (serde), `normalize(&serde_json::Value)`, `kind_of_code`, `part_of_code`, `is_water_tile`, `frame`, `cell_origin`. Painting is the renderer's job.

### `compat`

`check_map(&MapInput, &Tables, &Env) -> Vec<Finding>`, errors first. `MapInput` = map path (relative to `data/global/tiles`), `&Ds1`, the loaded libraries (`LoadedDt1 { path, found, tiles }`), candidate libraries for "add", and per-library usage counts. `Tables` holds the four `TxtDoc`s; `Env` holds optional callbacks (`dt1_exists`, `entry_image_exists`, `kept`, `new_level`, `is_waypoint`), the tiles' act and the compiled-bin list.
`Finding { severity, area, title, detail, cells, columns, fixes }`; `Finding::key()` ignores numbers (for "accepted" warnings). A `Fix` is `FixAction::Edits(Vec<Edit>)` or a UI flow (`OpenTable`, `OpenAutomapEditor`, `Register`, `ShortenPaths`, `Keep`). `Edit` variants state end states (`SetCell`, `ReorderRecords`, `RemoveInvalidAutomapRows`, `RemoveDt1s`/`AddDt1s`, `ClearCells`, `SetSpecial`, `DeleteObjectsOutsideMap`, `SetAct`), so applying a fix twice equals once: `Fix::apply(&mut Tables, Option<&mut Ds1>) -> Result<bool>` (false the second time).
`compat::verify_in_game` holds the table rules (record order, path length, Files, FillBlanks, town-only AutoMap 1, preset-to-level link, Layer, size = DS1 - 1, act, Pal, quest flag, EntryFile, world position and overlap). Not covered: duplicate/clashing/mixed-colour libraries, palette-act detection (pass `Env::tiles_act`).

**Render checks** (`compat::render_checks`, run when `Env::render = Some(RenderEnv { grid, is_waypoint })`; `grid` is `capy_render::walk::sub_tile_flags(ds1, tiles, Collision::Game { blank })`): arrival by portal (`links::arrival_problem`), warp links and ways out (`links::exit_problems`, with `Levels.txt` rows via `level_row`), roof hiding (`pops::pop_problems`, with LvlPrest `Pops`/`PopPad`), open map edges (`rooms::open_void`), objects on unwalkable ground and several objects on one sub-tile (fixes `Edit::MoveObjects { moves: Vec<(index, x, y)> }`, end-state so idempotent), and partly no-spawn rooms (`rooms::partly_no_spawn_rooms`, with LvlPrest `Logicals`). "Open table" fixes carry the row key (`FixAction::OpenTable { table, key }`) and the app opens the Tables window at it.

**`ShortenPaths`** (`compat::rename`): `suggest_renames(&[String]) -> Vec<Rename { from, to }>`, `rename_problems(&[Rename]) -> Vec<Option<String>>` (empty, still too long, wrong extension, two files sharing a path), `apply_renames(&mut Tables, &[Rename]) -> Vec<String>` (points every LvlTypes `File 1..32` and LvlPrest `File1..6` cell at the new path; the second application changes nothing). The app copies each file to its new path in the saves and saves both tables.

### `preset_package` (ds1-studio `presetPackage.ts`, `presetLibrary.ts`)

```rust
pub fn fill_sources(&mut Preset, source_of: &dyn Fn(TileId) -> Option<String>);   // tile_sources + dt1s
pub fn build_preset_package(&[Preset], locate: &dyn Fn(&str) -> Option<TileFile>) -> PresetPackage; // files [(zip path, bytes)], dt1s, missing
pub fn plan_preset_import(bytes, zip_entries: Option<&[(String, Vec<u8>)]>, existing: &[Preset], read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<PresetImportPlan>;
pub fn prepare_preset_library(&Preset, path, target: &dyn Fn(TileId) -> Vec<String>, read, keep: &BTreeSet<TileId>, taken: &BTreeSet<TileId>) -> Result<PreparedLibrary>; // { plan, dt1: Option<Dt1>, preset }
```

Package layout: `ds1studio-presets.json` (`{ format, version: 1, presets, dt1s }`), `presets/<file>.json`, `tiles/<path>` (the mod's own DT1s; the game's are left out). Import keeps a library already there unchanged, writes a free one, renames a clashing one (`name2.dt1`) and repoints the presets, skips unchanged presets, gives a taken id a new one, and lists libraries this game or mod lacks. `prepare_preset_library` copies only the tile numbers the map lacks (variants, animation frames and corner partners come along), renumbering clashes, and returns the preset with its cells and sources pointing at `path`. The zip is `capy_history::zip`.

### `crash_log`

`parse_crash_log(&str) -> Vec<Crash>` (time, `CrashKind`, what, module, offset, line, address, failed files) and `explain_crash(&Crash, Option<&LevelsWithEntry>) -> Explanation { title, detail, known, check, cause }`.
