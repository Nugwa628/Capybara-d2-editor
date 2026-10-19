//! Error type shared by the whole crate.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoryError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not writable: {detail}")]
    NotWritable { path: PathBuf, detail: String },
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

impl HistoryError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> HistoryError {
        HistoryError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
