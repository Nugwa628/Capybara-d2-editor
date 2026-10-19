# `capy-index` public API, schema and naming rules (owner: lead)

Background asset index (`capybara\cache\index.db`) and user data
(`capybara\user.db`). Depends on `d2-formats`, `d2-assets`. No Bevy.
SQLite via `rusqlite` (bundled, FTS5). Fuzzy ranking via `nucleo`.
Perceptual hash via `image_hasher`.

## Asset keys (stable across cache rebuilds)
- File assets: `<game path>` (e.g. `data\global\tiles\act1\town\towne1.ds1`).
- DT1 tiles: `<dt1 game path>#<orientation>/<main>/<sub>/<n>` where `n` is the
  0-based index of this tile among tiles with the same key in that DT1, in file
  order. Example: `data\global\tiles\act1\town\floor.dt1#0/5/2/0`.
- Keys never include the profile; `user.db` rows carry the profile id separately.

## Generated names (deterministic)
`<place> · <class> · <descriptor> · <main:02>/<sub:02>` for tiles, e.g.
`Act1 Town · Floor · grass · 05/02`.
- **place**: the LvlTypes `Name` of the lowest-id LvlType whose File slots load
  this DT1, if the table has names (vanilla `.txt`). Otherwise derived from the
  path under `data\global\tiles\`: drop the file extension, title-case each
  segment, join with spaces, and if the last segment repeats the directory drop it
  (`mxl\zone\whimsy\shire.dt1` → `Mxl Zone Whimsy Shire`; `act1\town\floor.dt1` → `Act1 Town Floor`).
- **class** from orientation: 0 Floor; 1–9 Wall (1 left, 2 right, 3 right-part
  corner, 4 left-part corner, 5 left end, 6 right end, 7 south corner, 8/9 door…
  use "Wall" for all, with `orientation` kept as a column); 10 Special; 11 Special;
  12 Wall; 13 Shadow; 14 Tree; 15 Roof; 16–17 Lower wall; 18–19 Lower wall.
- **descriptor**: the dominant colour name (see below), plus the texture class
  when it is not "flat" (`grass` is a colour+texture combination chosen from a
  small table: green+noisy → "grass", brown+noisy → "dirt", grey+edgy → "stone",
  blue+flat → "water", dark → "dark"; otherwise `<colour> <texture>`).
- Colour names (v2, lead decision 2026-10-19, replacing nearest-RGB, which
  named half of all tiles "black"): from the mean opaque colour in HSV.
  `dark` if luma < 25. Else if saturation < 0.15: `grey` (luma < 170) or
  `white`. Else by hue: red [345,15), orange [15,45), yellow [45,70),
  green [70,160), teal [160,185), cyan [185,200), blue [200,255),
  purple [255,290), magenta [290,320), pink [320,345); then red/orange/yellow
  with value < 0.55 become `brown`, and orange/yellow with saturation < 0.35
  and value >= 0.55 become `tan`.
- Texture class from edge density of the opaque pixels (Sobel magnitude mean):
  `flat` < 20, `noisy` < 60, `edgy` otherwise (recalibrated 2026-10-19 on the
  Median XL tile corpus: p10 = 22, p50 = 66, p90 = 110).
- Files: `<place> · <kind>` (`Act1 Town Towne1 · DS1`).
User aliases (user.db) are shown instead of, and searched alongside, generated names.

## Schema
`index.db` (schema_version 2; any mismatch → drop and rebuild):
```sql
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE profiles(id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, built_unix_ms INTEGER NOT NULL);
CREATE TABLE files(profile TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL,
  layer INTEGER NOT NULL, source TEXT NOT NULL, size INTEGER NOT NULL, hash BLOB NOT NULL,
  PRIMARY KEY(profile, path));
CREATE TABLE assets(id INTEGER PRIMARY KEY, profile TEXT NOT NULL, key TEXT NOT NULL,
  kind TEXT NOT NULL,              -- 'tile' | 'ds1' | 'dt1' | 'dc6' | 'dcc' | 'cof'
  path TEXT NOT NULL, act INTEGER, orientation INTEGER, main INTEGER, sub INTEGER,
  variant INTEGER, rarity INTEGER, width INTEGER, height INTEGER, class TEXT,
  gen_name TEXT NOT NULL, color TEXT, color_rgb INTEGER, texture TEXT, transparency REAL,
  phash INTEGER, phash_mirror INTEGER,
  directions INTEGER, frames INTEGER,  -- sprites (DC6/DCC/COF): direction count, frames per direction
  src TEXT NOT NULL,               -- winning source name (archive/folder)
  src_tag TEXT NOT NULL,           -- 'base' | 'mod' | 'saves'  (mod = any layer above the base MPQs)
  thumb BLOB,                      -- PNG, longest side <= 96 px
  UNIQUE(profile, key));
CREATE TABLE asset_lvltypes(asset_id INTEGER NOT NULL, lvltype INTEGER NOT NULL, slot INTEGER NOT NULL);
CREATE TABLE used_in(asset_id INTEGER NOT NULL, ds1_path TEXT NOT NULL, cells INTEGER NOT NULL);
CREATE VIRTUAL TABLE assets_fts USING fts5(gen_name, path, class, color, texture,
  content='assets', content_rowid='id', tokenize="unicode61 tokenchars '_\\'");
```
`user.db` (schema_version 2; migrations forward-only, never dropped; 1 → 2
adds `sprite_palettes`, see "Sprite palette memory"):
```sql
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE aliases(profile TEXT NOT NULL, key TEXT NOT NULL, alias TEXT NOT NULL, PRIMARY KEY(profile, key));
CREATE TABLE tags(profile TEXT NOT NULL, key TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY(profile, key, tag));
CREATE TABLE notes(profile TEXT NOT NULL, key TEXT NOT NULL, note TEXT NOT NULL, PRIMARY KEY(profile, key));
CREATE TABLE favorite_groups(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, position INTEGER NOT NULL);
CREATE TABLE favorites(group_id INTEGER NOT NULL, profile TEXT NOT NULL, key TEXT NOT NULL, position INTEGER NOT NULL,
  PRIMARY KEY(group_id, profile, key));
CREATE TABLE recents(profile TEXT NOT NULL, key TEXT NOT NULL, last_used_unix_ms INTEGER NOT NULL, uses INTEGER NOT NULL,
  PRIMARY KEY(profile, key));
CREATE TABLE saved_searches(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, query TEXT NOT NULL,
  position INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0);
-- schema 2
CREATE TABLE sprite_palettes(profile TEXT NOT NULL, key TEXT NOT NULL, palette TEXT NOT NULL,
  updated_unix_ms INTEGER NOT NULL, PRIMARY KEY(profile, key));
CREATE INDEX sprite_palettes_by_time ON sprite_palettes(profile, updated_unix_ms);
```

## Rust API
```rust
pub mod error;  pub use error::{IndexError, Result};
pub mod query;  pub use query::{Query, Filter, AssetKind, parse_query, QueryError};
pub mod naming; pub use naming::{generated_tile_name, place_from_path, color_name, texture_class};
pub mod index;  pub use index::{Index, IndexStatus, BuildProgress, BuildPhase, BuildReport, AssetRow, Hit};
pub mod user;   pub use user::{UserDb, FavoriteGroup, SavedSearch};

#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum IndexError {
    #[error("database {path}: {source}")] Sql { path: std::path::PathBuf, #[source] source: rusqlite::Error },
    #[error(transparent)] Asset(#[from] d2_assets::AssetError),
    #[error(transparent)] Format(#[from] d2_formats::FormatError),
    #[error("build cancelled")] Cancelled,
    #[error("{0}")] Other(String),
}
pub type Result<T, E = IndexError> = std::result::Result<T, E>;

// query.rs — `type: act: color: tag: idx: file: similar: used-in: mirrored: src:` + free text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind { Tile, Ds1, Dt1, Dc6, Dcc, Cof }
impl AssetKind {
    pub fn as_str(self) -> &'static str;               // the `kind` column: "tile", "ds1", ...
    pub fn parse(text: &str) -> Option<AssetKind>;     // inverse of as_str, ASCII case-insensitive
    pub fn label(self) -> &'static str;                // "Tile", "DS1", "DT1", "DC6", "DCC", "COF" (generated names)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    Type(AssetKind),                 // type:tile|floor|wall|roof|shadow|special|ds1|dt1|dc6|dcc|cof
    Class(String),                   //   (type:floor etc. become Type(Tile) + Class("floor"))
    Act(u8),                         // act:1..5 (stored 0-based)
    Color(String),                   // color:green
    Tag(String),                     // tag:"my tag" (quoted values allowed)
    Idx { main: u8, sub: Option<u8>, orientation: Option<u8> }, // idx:5  idx:5/2  idx:0:5/2 (orientation:main/sub)
    File(String),                    // file:floor.dt1 (substring of path, case-insensitive)
    AnyFile(Vec<String>),            // no token, programmatic only: path contains ANY of these substrings
                                     //   (each like File; pass `\` separators). [] matches nothing.
    Similar(String),                 // similar:<asset key>  (pHash Hamming distance <= 10, nearest first)
    UsedIn(String),                  // used-in:towne1 (substring of DS1 path)
    Mirrored,                        // mirrored:yes — has a horizontally mirrored twin (phash_mirror match)
    Src(String),                     // src:mod / src:base / src:saves / src:<archive substring>
    Not(Box<Filter>),                // -tag:foo
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query { pub text: String, pub filters: Vec<Filter> }
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("bad query at {position}: {detail}")]
pub struct QueryError { pub position: usize, pub detail: String }
pub fn parse_query(input: &str) -> Result<Query, QueryError>;   // unknown `word:` stays free text
// Multi-file scope (lib-wishes-2, 2026-10-19, for lead review): `Filter::AnyFile` rather than a
// `Query` field, so it composes with `Not` and the other filters and needs no
// new query plumbing. `Filter` is not `#[non_exhaustive]`; adding the variant
// only breaks exhaustive `match`es, and no crate outside capy-index has one.
// "This map's tile set" is one query: `type:tile` + `AnyFile(map DT1 paths)`;
// it returns exactly the union of the per-DT1 `file:` queries (real-data test
// over the Median XL whimsy DT1s in `tests/p3_queries.rs`).
// Ranking: every free-text token must match (AND). A token matches via an
// alias/name/path prefix, substring or segment hit, or a fuzzy match scoring
// at least 70% of the token's self-match score (no loose fuzzy wins).

// index.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexStatus { Missing, UpToDate, Stale { reason: String } } // Stale when any layer fingerprint differs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildPhase { Listing, Tables, Maps, Tiles, Sprites, Thumbnails, Writing }
#[derive(Clone, Debug, PartialEq)]
pub struct BuildProgress { pub phase: BuildPhase, pub done: u64, pub total: u64, pub current: String }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildReport { pub files: u64, pub assets: u64, pub tiles: u64, pub warnings: Vec<String>, pub millis: u64 }
#[derive(Clone, Debug, PartialEq)]
pub struct AssetRow {
    pub id: i64, pub key: String, pub kind: AssetKind, pub path: String,
    pub act: Option<u8>, pub orientation: Option<u8>, pub main: Option<u8>, pub sub: Option<u8>,
    pub variant: Option<u32>, pub rarity: Option<i32>, pub width: Option<u32>, pub height: Option<u32>,
    pub class: Option<String>, pub gen_name: String, pub color: Option<String>, pub texture: Option<String>,
    pub transparency: Option<f32>, pub src: String, pub src_tag: String,
    pub directions: Option<u32>, pub frames: Option<u32>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Hit { pub asset: AssetRow, pub score: f32, pub alias: Option<String> }

/// Send + Sync is not required; the app opens one Index per thread.
pub struct Index { /* path, Connection */ }
impl Index {
    pub fn open(cache_dir: &std::path::Path) -> Result<Index>;              // creates cache_dir/index.db
    pub fn db_path(&self) -> &std::path::Path;                              // where index.db lives
    /// Ignores the profile's saves layer (`saves\<id>`); saving a map never makes
    /// the profile stale. Saved DS1s are picked up by `refresh_saves`.
    pub fn status(&self, profile_id: &str, overlay: &d2_assets::Overlay) -> Result<IndexStatus>;
    /// Cheap incremental update: re-reads only the saves layer's DS1 files and
    /// updates their ds1 assets, src/src_tag and used_in rows. Milliseconds per map.
    pub fn refresh_saves(&mut self, profile: &d2_assets::Profile, overlay: &d2_assets::Overlay) -> Result<BuildReport>;
    /// Full rebuild of one profile (deletes its rows first) in the calling thread.
    /// Reports progress at least every 250 ms of work; checks `cancel` between items.
    pub fn build(&mut self, profile: &d2_assets::Profile, overlay: &d2_assets::Overlay,
                 progress: &mut dyn FnMut(&BuildProgress), cancel: &std::sync::atomic::AtomicBool) -> Result<BuildReport>;
    pub fn search(&self, profile_id: &str, query: &Query, user: &UserDb, limit: usize) -> Result<Vec<Hit>>;
    pub fn asset(&self, profile_id: &str, key: &str) -> Result<Option<AssetRow>>;
    pub fn thumbnail(&self, asset_id: i64) -> Result<Option<Vec<u8>>>;        // PNG bytes
    pub fn tiles_with_key(&self, profile_id: &str, orientation: u8, main: u8, sub: u8) -> Result<Vec<AssetRow>>;
    pub fn count(&self, profile_id: &str, kind: Option<AssetKind>) -> Result<u64>;
    /// LvlTypes rows that load a DT1 asset: `(lvltype id, slot)`, slot being the
    /// 0-based File index (bit `slot` of a LvlPrest `Dt1Mask`); sorted by lvltype, slot.
    /// Empty for assets that are not DT1 files or that no LvlType loads.
    pub fn lvltypes_of(&self, asset_id: i64) -> Result<Vec<(u32, u32)>>;
    /// DS1 paths that use a tile asset, with how many cells use it; most cells first,
    /// then by path. Empty for non-tile assets and unused tiles.
    pub fn used_in(&self, asset_id: i64) -> Result<Vec<(String, u32)>>;
}

// user.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FavoriteGroup { pub id: i64, pub name: String, pub position: i64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSearch { pub id: i64, pub name: String, pub query: String, pub position: i64, pub pinned: bool }
pub struct UserDb { /* Connection */ }
impl UserDb {
    pub fn open(path: &std::path::Path) -> Result<UserDb>;               // creates the file and folder, migrates
    pub fn schema_version(&self) -> Result<i64>;                         // the stored `meta` schema_version
    pub fn alias(&self, profile: &str, key: &str) -> Result<Option<String>>;
    pub fn set_alias(&self, profile: &str, key: &str, alias: Option<&str>) -> Result<()>;
    pub fn tags(&self, profile: &str, key: &str) -> Result<Vec<String>>;
    pub fn add_tag(&self, profile: &str, key: &str, tag: &str) -> Result<()>;
    pub fn remove_tag(&self, profile: &str, key: &str, tag: &str) -> Result<()>;
    pub fn keys_with_tag(&self, profile: &str, tag: &str) -> Result<Vec<String>>;
    pub fn note(&self, profile: &str, key: &str) -> Result<Option<String>>;
    pub fn set_note(&self, profile: &str, key: &str, note: Option<&str>) -> Result<()>;
    pub fn favorite_groups(&self) -> Result<Vec<FavoriteGroup>>;
    pub fn create_group(&self, name: &str) -> Result<FavoriteGroup>;
    pub fn rename_group(&self, id: i64, name: &str) -> Result<()>;
    pub fn delete_group(&self, id: i64) -> Result<()>;
    pub fn move_group(&self, id: i64, new_position: i64) -> Result<()>;
    pub fn favorites(&self, group_id: i64) -> Result<Vec<(String, String)>>;  // (profile, key) in order
    pub fn add_favorite(&self, group_id: i64, profile: &str, key: &str) -> Result<()>;
    pub fn remove_favorite(&self, group_id: i64, profile: &str, key: &str) -> Result<()>;
    pub fn move_favorite(&self, group_id: i64, profile: &str, key: &str, new_position: i64) -> Result<()>;
    pub fn record_use(&self, profile: &str, key: &str) -> Result<()>;
    pub fn recents(&self, profile: &str, limit: usize) -> Result<Vec<String>>;   // newest first
    pub fn use_count(&self, profile: &str, key: &str) -> Result<u32>;   // record_use calls kept in recents; 0 if never (or forgotten past 1000 recents)
    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>>;           // pinned first, then by position, then id
    pub fn save_search(&self, name: &str, query: &str) -> Result<SavedSearch>; // existing name: replaces the query, keeps position and pin
    pub fn rename_search(&self, id: i64, name: &str) -> Result<()>;     // trimmed, unique, non-empty; keeps query, position, pin
    pub fn move_search(&self, id: i64, new_position: i64) -> Result<()>; // clamped; others shift
    pub fn delete_search(&self, id: i64) -> Result<()>;
    pub fn set_search_pinned(&self, id: i64, pinned: bool) -> Result<()>;
}
```
Ordering (user.db): favorite groups, favorites and saved searches keep dense
positions `0..n` after every write; `move_*` removes the item and re-inserts it
at `new_position` (negative = 0, past the end = last). Saved-search positions
are one order over all searches; pinning only changes the listing order. Names
of groups and saved searches are trimmed and unique. An empty or taken name,
and renaming or moving an unknown id, is `IndexError::Other`; deleting or
pinning an unknown id does nothing. Recents are newest first and capped at
1000 per profile.

```rust
// naming.rs — pure functions behind generated names (see "Generated names")
pub fn generated_tile_name(place: &str, orientation: u8, descriptor: &str, main: u8, sub: u8) -> String;
pub fn generated_file_name(path: &str, kind: AssetKind) -> String;   // `<place> · <kind label>`
pub fn place_from_path(path: &str) -> String;
pub fn class_name(orientation: u8) -> &'static str;                  // "Floor", "Wall", "Special", ...
pub fn descriptor(color: &str, texture: &str, luma: f32) -> String;  // "grass", "dirt", "green edgy", "dark", ...
pub fn color_name(rgb: [u8; 3]) -> &'static str;
pub fn texture_class(edge_density: f32) -> &'static str;             // "flat" | "noisy" | "edgy" (NaN = flat)
pub fn luma(rgb: [u8; 3]) -> f32;                                    // Rec. 601
pub const FLAT_BELOW: f32 = 20.0;  pub const NOISY_BELOW: f32 = 60.0;  pub const DARK_BELOW: f32 = 25.0;
pub const GREY_BELOW_SATURATION: f32 = 0.15;  pub const WHITE_FROM_LUMA: f32 = 170.0;
pub const BROWN_BELOW_VALUE: f32 = 0.55;  pub const TAN_BELOW_SATURATION: f32 = 0.35;
```

## Sprite palette memory (`UserDb`, P7)

The palette a user last viewed, edited, imported or saved a sprite or tile
file with, so the Sprites tab reopens it with the same palette. Labels are
the palette folder names: `act1`..`act5`, `units`, `loading`, `menu0`, ...,
or `grey`. `key` is a game path (or a loose file path), matched
case-insensitively. A schema bump adds
`sprite_palettes(profile, key, palette, updated_unix_ms)` to existing
databases.

```rust
impl UserDb {
    pub fn sprite_palette(&self, profile: &str, key: &str) -> Result<Option<String>>;
    /// None forgets the entry.
    pub fn set_sprite_palette(&self, profile: &str, key: &str, palette: Option<&str>) -> Result<()>;
    /// The most recently remembered palette of any file in the same folder
    /// as `key` (its parent, case-insensitive), for a "used in this folder"
    /// hint when the file itself has none.
    pub fn folder_palette(&self, profile: &str, key: &str) -> Result<Option<String>>;
}
```
As implemented: keys are stored lower-cased (Unicode `to_lowercase`) with
`/` turned into `\`, so `Data/Global/UI/x.DC6` and `data\global\ui\x.dc6` are
one entry and both separators work in every call. Labels are trimmed and
stored as given; a blank label forgets like `None`. Every write is strictly
newer than the profile's earlier writes (as `record_use`), so "most recent"
is deterministic. `folder_palette` matches the key's direct parent only (not
subfolders), counts the file's own entry, and treats a bare file name as the
"no folder" folder.
