# `capy-history` public API (owner: lead)

Everything the editor writes about maps: loose saves with `CHANGED.txt`,
deduplicated snapshots, and optional gix milestones. Depends on `d2-assets`
(for `GamePath`) only. No Bevy. All writes are atomic.

```rust
// ───────────── lib.rs ─────────────
pub mod error;     pub use error::{HistoryError, Result};
pub mod atomic;    pub use atomic::atomic_write;
pub mod saves;     pub use saves::SavesDir;
pub mod store;     pub use store::{SnapshotStore, Snapshot, SnapshotId, SnapshotKind, ContentHash, Retention, PruneReport};
pub mod milestone; pub use milestone::{Milestones, MilestoneInfo};   // feature "milestones" (default on), gix

// ───────────── error.rs ─────────────
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoryError {
    #[error("I/O error on {path}: {source}")]
    Io { path: std::path::PathBuf, #[source] source: std::io::Error },
    #[error("{path} is not writable: {detail}")]
    NotWritable { path: std::path::PathBuf, detail: String },
    #[error("invalid game path {0:?}")]
    BadGamePath(String),
    #[error("snapshot {id} is corrupt: {detail}")]
    Corrupt { id: u64, detail: String },
    #[error("unknown snapshot {0}")]
    UnknownSnapshot(u64),
    #[error("history index: {0}")]
    Index(String),
    #[error("milestone: {0}")]
    Milestone(String),
}
pub type Result<T, E = HistoryError> = std::result::Result<T, E>;

// ───────────── atomic.rs ─────────────
/// Write `bytes` to a temp file in the same directory, flush + sync it, then
/// rename over `path` (replace-existing). Creates parent directories. A crash
/// at any point leaves either the old file or the new file, never a mix.
/// Stale temp files from a crash are named `.<file>.capytmp-*` and are
/// ignored/cleaned by the next write to the same file.
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()>;

// ───────────── saves.rs ─────────────
/// `capybara\saves\<profile>\`: edited maps mirroring game paths, plus CHANGED.txt.
pub struct SavesDir { /* root */ }
impl SavesDir {
    pub fn new(saves_root: &std::path::Path, profile_id: &str) -> SavesDir;
    pub fn root(&self) -> &std::path::Path;
    /// `root` + the game path's components (lowercase, as GamePath stores them).
    pub fn path_for(&self, game_path: &d2_assets::GamePath) -> Result<std::path::PathBuf>; // BadGamePath for `..`, drive or absolute components
    /// Creates `root` if needed and proves it is writable (writes and deletes a probe file).
    pub fn check_writable(&self) -> Result<()>;
    /// atomic_write to path_for(game_path), then add the game path to CHANGED.txt
    /// if it is not already listed (CHANGED.txt is rewritten atomically).
    pub fn save(&self, game_path: &d2_assets::GamePath, bytes: &[u8]) -> Result<std::path::PathBuf>;
    /// CHANGED.txt entries in insertion order (one canonical game path per line, CRLF).
    pub fn changed(&self) -> Result<Vec<d2_assets::GamePath>>;
    /// "Mark as packed": clears CHANGED.txt (atomically; file left empty).
    pub fn mark_packed(&self) -> Result<()>;
}

// ───────────── store.rs ─────────────
/// `capybara\history\<profile>\`: `index.json` (timeline, atomic) + `objects\<2 hex>\<rest>.z`
/// (zlib-compressed blobs named by content hash; each written once).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct SnapshotId(pub u64);
/// blake3 of the uncompressed bytes, hex.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ContentHash(pub String);
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SnapshotKind { Save, Autosave, Checkpoint { name: String }, BeforeRestore }
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub game_path: d2_assets::GamePath,
    pub hash: ContentHash,
    pub size: u64,
    pub created_unix_ms: i64,
    pub kind: SnapshotKind,
    pub label: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retention {
    /// Per game path, keep this many newest Save/Autosave/BeforeRestore snapshots.
    pub keep_last: usize,
    /// Also drop those older than this. Checkpoints are never pruned.
    pub max_age_days: Option<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruneReport { pub snapshots_removed: usize, pub blobs_removed: usize, pub bytes_freed: u64 }

pub struct SnapshotStore { /* root, snapshots: Vec<Snapshot>, next id */ }
impl SnapshotStore {
    /// Opens or creates the store. A missing index.json is an empty store; a corrupt one is an error.
    pub fn open(history_root: &std::path::Path, profile_id: &str) -> Result<SnapshotStore>;
    /// Stores the blob if new and appends a snapshot. If the newest snapshot of
    /// this game path has the same hash and `kind` is Save or Autosave, nothing
    /// is added and that snapshot is returned (dedupe).
    pub fn record(&mut self, game_path: &d2_assets::GamePath, bytes: &[u8], kind: SnapshotKind) -> Result<Snapshot>;
    pub fn all(&self) -> &[Snapshot];                                          // oldest first
    pub fn timeline(&self, game_path: &d2_assets::GamePath) -> Vec<&Snapshot>; // oldest first
    pub fn get(&self, id: SnapshotId) -> Option<&Snapshot>;
    /// Decompresses and verifies the hash (Corrupt on mismatch).
    pub fn read(&self, id: SnapshotId) -> Result<Vec<u8>>;
    pub fn set_label(&mut self, id: SnapshotId, label: Option<String>) -> Result<()>;
    /// Turns any snapshot into a named checkpoint (or renames one).
    pub fn make_checkpoint(&mut self, id: SnapshotId, name: String) -> Result<()>;
    pub fn delete(&mut self, id: SnapshotId) -> Result<()>;                   // blob GC happens in prune
    pub fn prune(&mut self, policy: &Retention) -> Result<PruneReport>;        // also removes unreferenced blobs
}

// ───────────── milestone.rs (feature "milestones") ─────────────
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MilestoneInfo { pub id: String, pub message: String, pub time_unix: i64 }
/// A git repository at `capybara\saves\` (all profiles), managed with gix.
pub struct Milestones { /* repo */ }
impl Milestones {
    pub fn open_or_init(saves_root: &std::path::Path) -> Result<Milestones>;
    /// Commits the full current content of `saves_root` (every file, including
    /// deletions since the last milestone). Author "Capybara's D2 Editor".
    /// Returns the commit id. No-op (returns the previous id) if nothing changed.
    pub fn commit(&self, message: &str) -> Result<String>;
    pub fn log(&self, limit: usize) -> Result<Vec<MilestoneInfo>>;            // newest first
}
```
