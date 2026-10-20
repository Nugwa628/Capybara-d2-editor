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
pub fn custom_object_files(token, frames, feet, anchor_x) -> Result<ObjectFilePlan>; // { cof: CofPlan, dc6_path, dc6: Vec<u8> }
```

The row becomes a plain one-mode decoration (marked `Capybara custom: <name>` in
`description - not loaded`; the reference editor's mark is recognised). The DC6
is encoded with `Dc6::from_frames`. **Deferred:** `d2-formats` has no COF
encoder, so the COF is returned as a `CofPlan` (path, box, one TR layer, rate
256) for the caller to encode once it exists. Converting a PNG to palette
indices and the object-use scan over DS1s are the caller's.

### Tests

`tests/level_corpus.rs` (needs `CAPY_PRIVATE_DIR`; skipped when absent) adds a
map to vanilla and Median XL tables and resolves it through
`d2_assets::MapCatalog` to the right LvlType and DT1 list; checks unchanged rows
are byte-identical, idempotence, level-type changes, and type-package
export/import between the two corpora. `tests/other_tools.rs` holds the cube,
entering-text and custom-object cases translated from the reference editor's
vitest files.
