# `d2-assets` public API (owner: lead)

Game data access. Depends on `d2-formats`. Never writes, renames or locks anything
in a game folder. Changing anything here needs the lead's approval.

```rust
// ───────────── lib.rs ─────────────
pub mod error;   pub use error::{AssetError, Result};
pub mod path;    pub use path::GamePath;
pub mod mpq;     pub use mpq::{MpqArchive, MpqHeader};
pub mod source;  pub use source::{AssetSource, SourceKind, SourceFingerprint, MpqSource, FolderSource, CascSource};
pub mod overlay; pub use overlay::{Overlay, LayerId};
pub mod profile; pub use profile::{Profile, LayerSpec, GameVersion};
pub mod names;   pub use names::{KnownNames, HarvestStats};   // harvest(): wave 2
pub mod tables;  // load_excel(): wave 2

// ───────────── error.rs ─────────────
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AssetError {
    #[error("I/O error on {path}: {source}")]
    Io { path: std::path::PathBuf, #[source] source: std::io::Error },
    #[error("{archive}: {detail}")]
    Mpq { archive: std::path::PathBuf, detail: String },
    #[error("{archive}: cannot decompress {file} (method {method:#04x}): {detail}")]
    Decompress { archive: std::path::PathBuf, file: String, method: u8, detail: String },
    #[error("profile {profile}: {detail}")]
    Profile { profile: String, detail: String },
    #[error("{0} is not supported yet")]
    Unsupported(&'static str),
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
}
pub type Result<T, E = AssetError> = std::result::Result<T, E>;

// ───────────── path.rs ─────────────
/// Canonical game-relative path: ASCII-lowercase, `\` separators, no leading
/// separator. Example: `data\global\tiles\act1\town\floor.dt1`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct GamePath(String);
impl GamePath {
    pub fn new(path: &str) -> GamePath;
    /// For DS1 file lists and LvlTypes/LvlPrest `File` columns: strips `C:`,
    /// a leading `\d2\` and leading separators, maps `.tg1` to `.dt1`; a path
    /// that does not start with `data\` is taken under `data\global\tiles\`.
    /// None for empty or "0".
    pub fn from_tile_ref(raw: &str) -> Option<GamePath>;
    pub fn as_str(&self) -> &str;
    pub fn file_name(&self) -> &str;
    pub fn extension(&self) -> Option<&str>;
    /// `root` joined with the path's components (for loose folders and saves).
    pub fn to_native(&self, root: &std::path::Path) -> std::path::PathBuf;
}
impl std::fmt::Display for GamePath;
impl From<&str> for GamePath;

// ───────────── mpq.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MpqHeader {
    pub format_version: u16,
    pub archive_offset: u64,
    pub sector_size: u32,
    pub hash_table_entries: u32,
    pub block_table_entries: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MpqCoverage { pub total: usize, pub named: usize }

/// Read-only MPQ archive. `Send + Sync`; reads are positional, no lock is
/// held between calls, and the file is opened with full share mode.
pub struct MpqArchive { /* ... */ }
impl MpqArchive {
    pub fn open(path: &std::path::Path) -> Result<MpqArchive>;
    pub fn path(&self) -> &std::path::Path;
    pub fn header(&self) -> &MpqHeader;
    pub fn contains(&self, name: &str) -> bool;
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>>; // None when absent
    pub fn listfile(&self) -> Result<Option<Vec<String>>>;     // "(listfile)", split on ; \r \n
    pub fn file_count(&self) -> usize;                         // block entries with the EXISTS flag
    /// How many existing files the given names account for.
    pub fn coverage<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> MpqCoverage;
}
pub fn hash_string(name: &str, hash_type: u32) -> u32;

// ───────────── source.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceKind { Mpq, Folder, Casc }

/// Cache key for an archive or folder. Folders aggregate (file count, total
/// size, newest mtime).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SourceFingerprint { pub path: std::path::PathBuf, pub size: u64, pub modified_unix_ms: i64, pub entries: u64 }

pub trait AssetSource: Send + Sync {
    fn name(&self) -> &str;              // e.g. "medianxl-dGlsZXM.mpq", "saves\\mxl"
    fn kind(&self) -> SourceKind;
    fn contains(&self, path: &GamePath) -> bool;
    fn read(&self, path: &GamePath) -> Result<Option<Vec<u8>>>;
    /// Names the source can enumerate on its own (listfile, directory walk).
    fn list(&self) -> Result<Vec<GamePath>>;
    fn fingerprint(&self) -> Result<SourceFingerprint>;
}

pub struct MpqSource { /* archive, name */ }
impl MpqSource {
    pub fn open(path: &std::path::Path) -> Result<MpqSource>;
    pub fn archive(&self) -> &MpqArchive;
}
impl AssetSource for MpqSource;

/// Loose folder. A missing root is an empty source, not an error.
pub struct FolderSource { /* root, name */ }
impl FolderSource { pub fn new(root: &std::path::Path, name: impl Into<String>) -> FolderSource; }
impl AssetSource for FolderSource;

/// D2R CASC. Stub until a D2R install exists.
// TODO(casc): integration test against a real D2R install.
pub struct CascSource;
impl CascSource { pub fn open(root: &std::path::Path) -> Result<CascSource>; } // Err(Unsupported("CASC"))
impl AssetSource for CascSource;

// ───────────── overlay.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub usize); // 0 = highest priority

pub struct Overlay { /* layers: Vec<Box<dyn AssetSource>> */ }  // Send + Sync
impl Overlay {
    pub fn new(layers: Vec<Box<dyn AssetSource>>) -> Overlay;
    pub fn layers(&self) -> &[Box<dyn AssetSource>];
    pub fn resolve(&self, path: &GamePath) -> Option<LayerId>;
    pub fn resolve_all(&self, path: &GamePath) -> Vec<LayerId>; // top first
    pub fn read(&self, path: &GamePath) -> Result<Option<(LayerId, Vec<u8>)>>;
    pub fn read_bytes(&self, path: &GamePath) -> Result<Option<Vec<u8>>>;
    pub fn read_from(&self, layer: LayerId, path: &GamePath) -> Result<Option<Vec<u8>>>;
    pub fn fingerprints(&self) -> Result<Vec<SourceFingerprint>>;
}

// ───────────── profile.rs ─────────────
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GameVersion {
    #[serde(rename = "1.13c")] V113c,
    #[serde(rename = "1.14d")] V114d,
    #[serde(rename = "d2r")]   D2r,
}
impl GameVersion { pub fn excel(self) -> Option<d2_formats::excel::ExcelVersion>; }

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LayerSpec {
    /// `<saves_root>\<profile id>\`. Missing folder = empty layer.
    Saves,
    Folder { path: std::path::PathBuf },
    Mpq { path: std::path::PathBuf, #[serde(default)] optional: bool },
    /// Every file in `dir` matching `pattern` (`*` wildcard, case-insensitive),
    /// sorted by name. Zero matches is an error.
    MpqGlob { dir: std::path::PathBuf, pattern: String },
    Casc { path: std::path::PathBuf },
}

/// One `capybara\profiles\<id>.toml`. Layers are listed highest priority first.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub id: String,                 // [a-z0-9_-]+, also the saves/history folder name
    pub name: String,
    pub game_version: GameVersion,
    #[serde(rename = "layer")]
    pub layers: Vec<LayerSpec>,
}
impl Profile {
    pub fn from_toml(text: &str) -> Result<Profile>;
    pub fn to_toml(&self) -> String;
    /// saves → Patch_D2 → d2exp → d2data → d2char.
    pub fn vanilla(install_dir: &std::path::Path) -> Profile;    // id "vanilla", 1.14d
    /// saves → medianxl-*.mpq → patch_d2 → d2exp → d2data → d2char, all from install_dir.
    pub fn median_xl(install_dir: &std::path::Path) -> Profile;  // id "mxl", 1.13c
    pub fn saves_dir(&self, saves_root: &std::path::Path) -> std::path::PathBuf;
    pub fn open(&self, saves_root: &std::path::Path) -> Result<Overlay>;
    /// Every *.toml in `dir`, sorted by id. Bad files are errors naming the file.
    pub fn load_dir(dir: &std::path::Path) -> Result<Vec<Profile>>;
}

// ───────────── names.rs ─────────────
#[derive(Clone, Debug, Default)]
pub struct KnownNames { /* BTreeSet<GamePath> */ }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HarvestStats {
    pub from_listfiles: usize, pub from_seed: usize, pub from_tables: usize,
    pub from_ds1: usize, pub from_cof: usize,   // names each source contributed first
    pub rounds: usize,                          // passes, including the final empty one
    pub unresolved: usize,                      // distinct candidates that resolved nowhere (dropped)
    pub warnings: Vec<String>,                  // tables/DS1s that failed to parse (skipped)
}
impl KnownNames {
    pub fn new() -> KnownNames;
    /// Embedded well-known paths (assets/known-names/seed.txt via include_str!).
    pub fn seed() -> KnownNames;
    pub fn insert(&mut self, path: GamePath) -> bool;
    pub fn extend_from_listfile(&mut self, text: &str) -> usize;
    pub fn contains(&self, path: &GamePath) -> bool;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = &GamePath>;
    /// listfiles + seed + names referenced by the excel tables and by every
    /// DS1 found (COF: later), repeated until no new names appear. Keeps only
    /// names that resolve in the overlay.
    pub fn harvest(overlay: &Overlay, version: GameVersion) -> Result<(KnownNames, HarvestStats)>;
    pub fn load(path: &std::path::Path) -> Result<KnownNames>; // one name per line
    pub fn save(&self, path: &std::path::Path) -> Result<()>;
}

// ───────────── tables.rs (wave 2) ─────────────
/// 1.13c profiles read `<stem>.bin` first (the 1.13c loader's default), then
/// `<stem>.txt`; 1.14d profiles read `.txt` first, then `.bin`. Levels from
/// .bin needs both levels.bin and leveldefs.bin (`excel::decode_levels_bin`).
/// The form is chosen by priority across the whole overlay, not per layer.
/// A preferred form that exists but fails to decode is an error (no silent
/// fallback); an absent form falls back to the other.
pub fn load_excel(overlay: &Overlay, kind: d2_formats::excel::ExcelTableKind, version: GameVersion) -> Result<d2_formats::txt::TxtTable>;
```

## Installs and default profiles (`installs.rs`, release round)

Install detection and first-run profile seeding, shared by the GUI and the
CLI so either can be the first to run on a clean `capybara\` folder (moved
from `capy-app/src/installs.rs`; same file formats and results). Install
folders are only listed, never written; the registry is opened read-only
(`KEY_READ`, `windows-registry`, Windows only). The only files written are
profile TOMLs in the profiles folder the caller names, atomically
(`<file>.tmp` then rename), never over an existing file and never inside an
install.

```rust
pub mod installs;   // pub use installs::{Candidate, InstallKind, InstallSearch};

pub const DEFAULT_VANILLA_INSTALL: &str = r"C:\Program Files (x86)\Diablo II";
pub const DEFAULT_MXL_INSTALL: &str = r"C:\games\median-xl"; // an auto-detect candidate only
pub const REGISTRY_KEYS: [(RegistryRoot, &str); 2]; // HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment\Diablo II, HKCU\Software\Blizzard Entertainment\Diablo II
pub const REGISTRY_VALUE: &str = "InstallPath";
pub enum RegistryRoot { LocalMachine, CurrentUser }          // .label(): "HKLM" | "HKCU"

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InstallKind { Vanilla, MedianXl, Unknown }          // .label(), .base_id(): "vanilla" | "mxl"
/// File names directly in `dir`, case-insensitive: any `medianxl-*.mpq` is
/// MedianXl (it wins); `d2data.mpq` + `d2exp.mpq` is Vanilla; else Unknown
/// (also for a missing folder).
pub fn detect_kind(dir: &Path) -> InstallKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate { pub path: PathBuf, pub source: String /* "registry (HKLM)", ... */ }
/// Trims NULs, whitespace, quotes and a trailing separator; "C:" becomes "C:\";
/// empty or relative values are None.
pub fn install_path_from_value(value: &str) -> Option<PathBuf>;
/// The InstallPath values of REGISTRY_KEYS, read-only, deduplicated; empty off Windows.
pub fn registry_install_paths() -> Vec<Candidate>;
/// Case-insensitive, separator-style and trailing-separator insensitive.
pub fn same_path(a: &Path, b: &Path) -> bool;
/// ([default_vanilla, registry paths not equal to it...], [default_mxl]).
pub fn first_run_candidates(default_vanilla: &Path, default_mxl: &Path, registry: &[Candidate])
    -> (Vec<PathBuf>, Vec<PathBuf>);

/// Where first-run seeding looks. `system()` = the two defaults + the real
/// registry; tests build one by hand (no registry or game dependence).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallSearch { pub default_vanilla: PathBuf, pub default_mxl: PathBuf, pub registry: Vec<Candidate> }
impl InstallSearch {
    pub fn system() -> InstallSearch;
    pub fn candidates(&self) -> (Vec<PathBuf>, Vec<PathBuf>); // first_run_candidates
}

/// At least one `*.toml` file (any case) in `dir`; a missing folder has none.
pub fn has_profile_files(dir: &Path) -> bool;
/// Writes vanilla.toml (Profile::vanilla) for the first vanilla candidate that
/// is a folder — index 0 (the default location) unconditionally, later ones
/// (registry) only when detect_kind == Vanilla — and mxl.toml
/// (Profile::median_xl) for the first mxl candidate that is a folder. Skips
/// existing files and a profiles folder inside the chosen install; creates
/// the folder only when writing. Returns the files written.
pub fn seed_profiles(profiles_dir: &Path, vanilla: &[PathBuf], mxl: &[PathBuf]) -> Result<Vec<PathBuf>>;
/// seed_profiles(search().candidates()) when !has_profile_files(profiles_dir);
/// `search` is not called otherwise. The CLI runs this before every verb but help.
pub fn seed_default_profiles(profiles_dir: &Path, search: impl FnOnce() -> InstallSearch) -> Result<Vec<PathBuf>>;

/// The wanted id when it exists, else "vanilla" when it exists, else the smallest id.
pub fn startup_profile(profiles: &[Profile], wanted: Option<&str>) -> Option<String>;
/// base, else base-2, base-3, ... (taken compared case-insensitively).
pub fn unique_id(taken: &[String], base: &str) -> String;
/// Ids of profiles with an Mpq/Folder/Casc path in or directly under `dir`, or an MpqGlob on `dir`.
pub fn profiles_using(profiles: &[Profile], dir: &Path) -> Vec<String>;
/// `[a-z0-9_-]+` (the rule Profile::from_toml enforces).
pub fn valid_id(id: &str) -> bool;
/// Profile::median_xl for MedianXl, else Profile::vanilla; with id, the
/// trimmed name (unless blank) and version.
pub fn profile_for(kind: InstallKind, dir: &Path, id: &str, name: &str, version: GameVersion) -> Profile;
/// Writes profiles_dir\<id>.toml (creating the folder). AssetError::Profile for
/// an invalid id, an existing file, a profile that does not round-trip through
/// TOML, or a profiles folder inside install_dir (checked on the nearest
/// existing ancestor, so a folder not created yet is caught too).
pub fn write_profile(profiles_dir: &Path, install_dir: &Path, profile: &Profile) -> Result<PathBuf>;
```
