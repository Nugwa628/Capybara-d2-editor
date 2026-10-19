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
`user.db` (schema_version 1; migrations forward-only, never dropped):
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    Type(AssetKind),                 // type:tile|floor|wall|roof|shadow|special|ds1|dt1|dc6|dcc|cof
    Class(String),                   //   (type:floor etc. become Type(Tile) + Class("floor"))
    Act(u8),                         // act:1..5 (stored 0-based)
    Color(String),                   // color:green
    Tag(String),                     // tag:"my tag" (quoted values allowed)
    Idx { main: u8, sub: Option<u8>, orientation: Option<u8> }, // idx:5  idx:5/2  idx:0:5/2 (orientation:main/sub)
    File(String),                    // file:floor.dt1 (substring of path, case-insensitive)
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
}

// user.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FavoriteGroup { pub id: i64, pub name: String, pub position: i64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSearch { pub id: i64, pub name: String, pub query: String, pub position: i64, pub pinned: bool }
pub struct UserDb { /* Connection */ }
impl UserDb {
    pub fn open(path: &std::path::Path) -> Result<UserDb>;
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
    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>>;
    pub fn save_search(&self, name: &str, query: &str) -> Result<SavedSearch>;
    pub fn delete_search(&self, id: i64) -> Result<()>;
    pub fn set_search_pinned(&self, id: i64, pinned: bool) -> Result<()>;
}
```
