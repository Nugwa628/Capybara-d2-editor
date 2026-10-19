//! Milestones: a git repository at `capybara\saves\` (all profiles), managed with gix.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use gix::ObjectId;
use gix::bstr::BString;
use gix::objs::Tree;
use gix::objs::tree::{Entry, EntryKind};

use crate::error::{HistoryError, Result};

const AUTHOR_NAME: &str = "Capybara Map Editor";
const AUTHOR_EMAIL: &str = "capybara@localhost";
/// Deepest folder nesting a milestone follows (saves mirror game paths, a
/// handful of levels deep); deeper trees are an error instead of unbounded
/// recursion.
const MAX_TREE_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MilestoneInfo {
    pub id: String,
    pub message: String,
    pub time_unix: i64,
}

/// A git repository at `capybara\saves\`.
pub struct Milestones {
    repo: gix::Repository,
}

fn err<E: std::fmt::Display>(what: &str, e: E) -> HistoryError {
    HistoryError::Milestone(format!("{what}: {e}"))
}

impl Milestones {
    /// Opens the repository in `saves_root`, creating the folder and
    /// initialising the repository on first use.
    pub fn open_or_init(saves_root: &Path) -> Result<Milestones> {
        fs::create_dir_all(saves_root).map_err(|e| HistoryError::io(saves_root, e))?;
        let repo = if saves_root.join(".git").exists() {
            gix::open(saves_root).map_err(|e| err("open repository", e))?
        } else {
            gix::init(saves_root).map_err(|e| err("init repository", e))?
        };
        Ok(Milestones { repo })
    }

    /// Commits the full current content of `saves_root` (every file, including
    /// deletions since the last milestone). Author "Capybara Map Editor".
    /// Returns the commit id. No-op (returns the previous id) if nothing changed.
    pub fn commit(&self, message: &str) -> Result<String> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| HistoryError::Milestone("repository has no work tree".into()))?
            .to_path_buf();
        let tree_id = self.write_tree(&workdir, 0)?;

        let parent = self.head_commit_id()?;
        if let Some(parent) = parent {
            let prev = self
                .repo
                .find_commit(parent)
                .map_err(|e| err("read previous commit", e))?;
            let prev_tree = prev.tree_id().map_err(|e| err("read previous tree", e))?;
            if prev_tree.detach() == tree_id {
                return Ok(parent.to_string());
            }
        }

        let time = format!("{} +0000", now_secs());
        let sig = gix::actor::SignatureRef {
            name: AUTHOR_NAME.into(),
            email: AUTHOR_EMAIL.into(),
            time: &time,
        };
        let id = self
            .repo
            .commit_as(sig, sig, "HEAD", message, tree_id, parent)
            .map_err(|e| err("write commit", e))?;
        Ok(id.to_string())
    }

    /// Newest first, following the first parent.
    pub fn log(&self, limit: usize) -> Result<Vec<MilestoneInfo>> {
        let mut out = Vec::new();
        let mut next = self.head_commit_id()?;
        while let Some(id) = next {
            if out.len() >= limit {
                break;
            }
            let commit = self
                .repo
                .find_commit(id)
                .map_err(|e| err("read commit", e))?;
            let decoded = commit.decode().map_err(|e| err("decode commit", e))?;
            let time_unix = decoded
                .committer()
                .map_err(|e| err("decode committer", e))?
                .seconds();
            out.push(MilestoneInfo {
                id: id.to_string(),
                message: decoded.message.to_string().trim_end().to_string(),
                time_unix,
            });
            next = decoded.parents().next();
        }
        Ok(out)
    }

    fn head_commit_id(&self) -> Result<Option<ObjectId>> {
        let head = self.repo.head().map_err(|e| err("read HEAD", e))?;
        if head.is_unborn() {
            return Ok(None);
        }
        let id = head.into_peeled_id().map_err(|e| err("resolve HEAD", e))?;
        Ok(Some(id.detach()))
    }

    /// Writes the directory as git tree objects (blobs for files), recursively.
    /// `depth` is 0 for the work tree root.
    fn write_tree(&self, dir: &Path, depth: usize) -> Result<ObjectId> {
        if depth > MAX_TREE_DEPTH {
            return Err(HistoryError::Milestone(format!(
                "{} is nested more than {MAX_TREE_DEPTH} folders deep",
                dir.display()
            )));
        }
        let is_root = depth == 0;
        let mut entries: Vec<Entry> = Vec::new();
        let rd = fs::read_dir(dir).map_err(|e| HistoryError::io(dir, e))?;
        for item in rd {
            let item = item.map_err(|e| HistoryError::io(dir, e))?;
            let name = item.file_name().to_string_lossy().into_owned();
            if (is_root && name == ".git") || name.contains(".capytmp-") {
                continue;
            }
            let path = item.path();
            let file_type = item.file_type().map_err(|e| HistoryError::io(&path, e))?;
            if file_type.is_dir() {
                let sub = self.write_tree(&path, depth + 1)?;
                if sub == ObjectId::empty_tree(self.repo.object_hash()) {
                    continue; // git does not track empty directories
                }
                entries.push(Entry {
                    mode: EntryKind::Tree.into(),
                    filename: BString::from(name),
                    oid: sub,
                });
            } else if file_type.is_file() {
                let bytes = fs::read(&path).map_err(|e| HistoryError::io(&path, e))?;
                let blob = self
                    .repo
                    .write_blob(&bytes)
                    .map_err(|e| err("write blob", e))?;
                entries.push(Entry {
                    mode: EntryKind::Blob.into(),
                    filename: BString::from(name),
                    oid: blob.detach(),
                });
            }
        }
        entries.sort();
        let tree = Tree { entries };
        let id = self
            .repo
            .write_object(&tree)
            .map_err(|e| err("write tree", e))?;
        Ok(id.detach())
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}
