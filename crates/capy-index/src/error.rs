//! Error type for the index and the user database.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IndexError {
    #[error("database {path}: {source}")]
    Sql {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error(transparent)]
    Asset(#[from] d2_assets::AssetError),
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
    #[error("build cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = IndexError> = std::result::Result<T, E>;

impl IndexError {
    pub(crate) fn sql(path: &Path, source: rusqlite::Error) -> IndexError {
        IndexError::Sql {
            path: path.to_path_buf(),
            source,
        }
    }
}
