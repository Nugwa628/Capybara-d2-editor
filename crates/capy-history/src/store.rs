//! `capybara\history\<profile>\`: `index.json` (timeline, written atomically) plus
//! `objects\<2 hex>\<rest>.z` (zlib-compressed blobs named by content hash, each
//! written once).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use d2_assets::GamePath;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::error::{HistoryError, Result};

const INDEX_FILE: &str = "index.json";
const OBJECTS_DIR: &str = "objects";
const INDEX_VERSION: u32 = 1;
const MS_PER_DAY: i64 = 86_400_000;
/// Upper bound on what deflate can expand one input byte to.
const MAX_DEFLATE_RATIO: u64 = 1032;
/// Most bytes reserved up front when decompressing a blob (it grows past this
/// on demand), so a damaged `size` cannot reserve an absurd buffer.
const MAX_PREALLOC: u64 = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnapshotId(pub u64);

/// blake3 of the uncompressed bytes, hex.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SnapshotKind {
    Save,
    Autosave,
    Checkpoint { name: String },
    BeforeRestore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub game_path: GamePath,
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
pub struct PruneReport {
    pub snapshots_removed: usize,
    pub blobs_removed: usize,
    pub bytes_freed: u64,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    next_id: u64,
    snapshots: Vec<Snapshot>,
}

pub struct SnapshotStore {
    dir: PathBuf,
    snapshots: Vec<Snapshot>,
    next_id: u64,
}

impl SnapshotStore {
    /// Opens or creates the store under `history_root/<profile_id>`. A missing
    /// `index.json` is an empty store; a corrupt one is an error.
    pub fn open(history_root: &Path, profile_id: &str) -> Result<SnapshotStore> {
        let dir = history_root.join(profile_id);
        let index_path = dir.join(INDEX_FILE);
        let bytes = match fs::read(&index_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SnapshotStore {
                    dir,
                    snapshots: Vec::new(),
                    next_id: 1,
                });
            }
            Err(e) => return Err(HistoryError::io(&index_path, e)),
        };
        let index: IndexFile = serde_json::from_slice(&bytes)
            .map_err(|e| HistoryError::Index(format!("{}: {e}", index_path.display())))?;
        if index.version != INDEX_VERSION {
            return Err(HistoryError::Index(format!(
                "{}: unsupported version {}",
                index_path.display(),
                index.version
            )));
        }
        let mut seen = HashSet::new();
        let mut max_id = 0;
        for s in &index.snapshots {
            let h = s.hash.0.as_bytes();
            if h.len() != 64 || !h.iter().all(u8::is_ascii_hexdigit) {
                return Err(HistoryError::Index(format!(
                    "{}: snapshot {} has a malformed hash",
                    index_path.display(),
                    s.id.0
                )));
            }
            if !seen.insert(s.id) {
                return Err(HistoryError::Index(format!(
                    "{}: duplicate snapshot id {}",
                    index_path.display(),
                    s.id.0
                )));
            }
            max_id = max_id.max(s.id.0);
        }
        Ok(SnapshotStore {
            dir,
            snapshots: index.snapshots,
            next_id: index.next_id.max(max_id.saturating_add(1)).max(1),
        })
    }

    /// Stores the blob if new and appends a snapshot. If the newest snapshot of
    /// this game path has the same hash and `kind` is Save or Autosave, nothing
    /// is added and that snapshot is returned (dedupe).
    pub fn record(
        &mut self,
        game_path: &GamePath,
        bytes: &[u8],
        kind: SnapshotKind,
    ) -> Result<Snapshot> {
        self.record_at(game_path, bytes, kind, now_ms())
    }

    pub fn all(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// Snapshots of one game path, oldest first.
    pub fn timeline(&self, game_path: &GamePath) -> Vec<&Snapshot> {
        self.snapshots
            .iter()
            .filter(|s| &s.game_path == game_path)
            .collect()
    }

    pub fn get(&self, id: SnapshotId) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    /// Decompresses and verifies the hash (`Corrupt` on mismatch).
    pub fn read(&self, id: SnapshotId) -> Result<Vec<u8>> {
        let snap = self.get(id).ok_or(HistoryError::UnknownSnapshot(id.0))?;
        let corrupt = |detail: String| HistoryError::Corrupt { id: id.0, detail };
        let path = self.blob_path(&snap.hash);
        let compressed = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(corrupt(format!("blob {} is missing", path.display())));
            }
            Err(e) => return Err(HistoryError::io(&path, e)),
        };
        // Bounded read: a damaged blob must not expand without limit, and a
        // damaged `size` in index.json must not allocate without limit.
        // Deflate cannot expand by more than ~1032:1, so nothing a valid blob
        // holds lies beyond that bound.
        let possible = (compressed.len() as u64)
            .saturating_mul(MAX_DEFLATE_RATIO)
            .saturating_add(64);
        let limit = snap.size.min(possible);
        let mut out = Vec::with_capacity(usize::try_from(limit.min(MAX_PREALLOC)).unwrap_or(0));
        ZlibDecoder::new(compressed.as_slice())
            .take(limit.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| corrupt(format!("decompress failed: {e}")))?;
        if out.len() as u64 != snap.size {
            return Err(corrupt(format!(
                "size mismatch: expected {}, got {}",
                snap.size,
                out.len()
            )));
        }
        let actual = hash_bytes(&out);
        if actual != snap.hash {
            return Err(corrupt(format!(
                "hash mismatch: expected {}, got {}",
                snap.hash.0, actual.0
            )));
        }
        Ok(out)
    }

    pub fn set_label(&mut self, id: SnapshotId, label: Option<String>) -> Result<()> {
        let snap = self
            .snapshots
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(HistoryError::UnknownSnapshot(id.0))?;
        snap.label = label;
        self.write_index()
    }

    /// Turns any snapshot into a named checkpoint (or renames one).
    pub fn make_checkpoint(&mut self, id: SnapshotId, name: String) -> Result<()> {
        let snap = self
            .snapshots
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(HistoryError::UnknownSnapshot(id.0))?;
        snap.kind = SnapshotKind::Checkpoint { name };
        self.write_index()
    }

    /// Removes the snapshot from the timeline. Its blob is collected by `prune`.
    pub fn delete(&mut self, id: SnapshotId) -> Result<()> {
        let pos = self
            .snapshots
            .iter()
            .position(|s| s.id == id)
            .ok_or(HistoryError::UnknownSnapshot(id.0))?;
        self.snapshots.remove(pos);
        self.write_index()
    }

    /// Applies the retention policy, then removes blobs no snapshot refers to.
    pub fn prune(&mut self, policy: &Retention) -> Result<PruneReport> {
        self.prune_at(policy, now_ms())
    }

    // ---- internals -------------------------------------------------------

    fn record_at(
        &mut self,
        game_path: &GamePath,
        bytes: &[u8],
        kind: SnapshotKind,
        created_unix_ms: i64,
    ) -> Result<Snapshot> {
        let hash = hash_bytes(bytes);
        if matches!(kind, SnapshotKind::Save | SnapshotKind::Autosave)
            && let Some(newest) = self
                .snapshots
                .iter()
                .rev()
                .find(|s| &s.game_path == game_path)
            && newest.hash == hash
        {
            return Ok(newest.clone());
        }
        // A damaged index.json can carry `next_id = u64::MAX`.
        let id = self.next_id;
        let following = id.checked_add(1).ok_or_else(|| {
            HistoryError::Index(format!(
                "{}: snapshot ids are exhausted (next id {id})",
                self.dir.join(INDEX_FILE).display()
            ))
        })?;
        self.store_blob(&hash, bytes)?;
        let snap = Snapshot {
            id: SnapshotId(id),
            game_path: game_path.clone(),
            hash,
            size: bytes.len() as u64,
            created_unix_ms,
            kind,
            label: None,
        };
        self.next_id = following;
        self.snapshots.push(snap.clone());
        if let Err(e) = self.write_index() {
            // Keep memory and disk consistent when the index could not be written.
            self.snapshots.pop();
            self.next_id = id;
            return Err(e);
        }
        Ok(snap)
    }

    fn prune_at(&mut self, policy: &Retention, now_ms: i64) -> Result<PruneReport> {
        let cutoff = policy
            .max_age_days
            .map(|d| now_ms.saturating_sub(i64::from(d).saturating_mul(MS_PER_DAY)));
        // Newest first per game path; the first `keep_last` prunable ones survive.
        let mut seen: HashMap<&GamePath, usize> = HashMap::new();
        let mut doomed: HashSet<SnapshotId> = HashSet::new();
        for s in self.snapshots.iter().rev() {
            if matches!(s.kind, SnapshotKind::Checkpoint { .. }) {
                continue;
            }
            let rank = seen.entry(&s.game_path).or_insert(0);
            let too_many = *rank >= policy.keep_last;
            *rank += 1;
            let too_old = cutoff.is_some_and(|c| s.created_unix_ms < c);
            if too_many || too_old {
                doomed.insert(s.id);
            }
        }
        let mut report = PruneReport::default();
        if !doomed.is_empty() {
            let before = std::mem::take(&mut self.snapshots);
            let (gone, kept): (Vec<_>, Vec<_>) =
                before.into_iter().partition(|s| doomed.contains(&s.id));
            report.snapshots_removed = gone.len();
            self.snapshots = kept;
            if let Err(e) = self.write_index() {
                // Restore the in-memory timeline so it still matches the disk.
                self.snapshots.extend(gone);
                self.snapshots.sort_by_key(|s| s.id);
                return Err(e);
            }
        }
        self.collect_garbage(&mut report)?;
        Ok(report)
    }

    /// Removes blob files (and stale temp files) that no snapshot refers to.
    fn collect_garbage(&self, report: &mut PruneReport) -> Result<()> {
        let live: HashSet<&str> = self.snapshots.iter().map(|s| s.hash.0.as_str()).collect();
        let objects = self.dir.join(OBJECTS_DIR);
        let top = match fs::read_dir(&objects) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(HistoryError::io(&objects, e)),
        };
        for sub in top.flatten() {
            let sub_path = sub.path();
            if !sub_path.is_dir() {
                continue;
            }
            let prefix = sub.file_name().to_string_lossy().into_owned();
            let Ok(files) = fs::read_dir(&sub_path) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                let referenced = name
                    .strip_suffix(".z")
                    .is_some_and(|rest| live.contains(format!("{prefix}{rest}").as_str()));
                if referenced {
                    continue;
                }
                let path = file.path();
                let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                if fs::remove_file(&path).is_ok() {
                    report.bytes_freed += len;
                    if name.ends_with(".z") {
                        report.blobs_removed += 1;
                    }
                }
            }
            // Only succeeds when empty.
            let _ = fs::remove_dir(&sub_path);
        }
        Ok(())
    }

    fn blob_path(&self, hash: &ContentHash) -> PathBuf {
        let h = hash.0.as_str();
        // Hashes are validated as 64 hex digits on open; stay panic-free anyway.
        let cut = h.len().min(2);
        let cut = if h.is_char_boundary(cut) { cut } else { 0 };
        let (head, rest) = h.split_at(cut);
        self.dir
            .join(OBJECTS_DIR)
            .join(head)
            .join(format!("{rest}.z"))
    }

    fn store_blob(&self, hash: &ContentHash, bytes: &[u8]) -> Result<()> {
        let path = self.blob_path(hash);
        if path.is_file() {
            return Ok(());
        }
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(bytes)
            .map_err(|e| HistoryError::io(&path, e))?;
        let compressed = enc.finish().map_err(|e| HistoryError::io(&path, e))?;
        atomic_write(&path, &compressed)
    }

    fn write_index(&self) -> Result<()> {
        let index = IndexFile {
            version: INDEX_VERSION,
            next_id: self.next_id,
            snapshots: self.snapshots.clone(),
        };
        let json = serde_json::to_vec_pretty(&index)
            .map_err(|e| HistoryError::Index(format!("serialize: {e}")))?;
        atomic_write(&self.dir.join(INDEX_FILE), &json)
    }
}

fn hash_bytes(bytes: &[u8]) -> ContentHash {
    ContentHash(blake3::hash(bytes).to_hex().to_string())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp(s: &str) -> GamePath {
        GamePath::new(s)
    }

    #[test]
    fn max_age_and_keep_last() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = SnapshotStore::open(dir.path(), "p").unwrap();
        let a = gp("data\\a.ds1");
        let day = MS_PER_DAY;
        let now = 100 * day;
        // Ages in days: 50, 40, 30, 1.
        for (i, age) in [50, 40, 30, 1].into_iter().enumerate() {
            st.record_at(
                &a,
                format!("v{i}").as_bytes(),
                SnapshotKind::Save,
                now - age * day,
            )
            .unwrap();
        }
        let cp = st
            .record_at(
                &a,
                b"old-checkpoint",
                SnapshotKind::Checkpoint { name: "c".into() },
                now - 90 * day,
            )
            .unwrap();
        let rep = st
            .prune_at(
                &Retention {
                    keep_last: 3,
                    max_age_days: Some(35),
                },
                now,
            )
            .unwrap();
        // v0 (50d) and v1 (40d) are too old; v2, v3 stay; checkpoint stays.
        assert_eq!(rep.snapshots_removed, 2);
        assert_eq!(rep.blobs_removed, 2);
        assert!(rep.bytes_freed > 0);
        assert_eq!(st.all().len(), 3);
        assert!(st.get(cp.id).is_some());
        let rep = st
            .prune_at(
                &Retention {
                    keep_last: 1,
                    max_age_days: None,
                },
                now,
            )
            .unwrap();
        assert_eq!(rep.snapshots_removed, 1);
        assert_eq!(st.all().len(), 2);
    }
}
