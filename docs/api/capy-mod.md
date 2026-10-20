# `capy-mod` public API

Game-data editing for mods. Other sections are added by other phases; this
file holds the sections below.

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
