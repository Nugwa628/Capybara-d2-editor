//! Error type for the asset layer.

use std::path::PathBuf;

/// Why game data could not be opened, read or decoded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AssetError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{archive}: {detail}")]
    Mpq { archive: PathBuf, detail: String },
    #[error("{archive}: cannot decompress {file} (method {method:#04x}): {detail}")]
    Decompress {
        archive: PathBuf,
        file: String,
        method: u8,
        detail: String,
    },
    #[error("profile {profile}: {detail}")]
    Profile { profile: String, detail: String },
    #[error("{0} is not supported yet")]
    Unsupported(&'static str),
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
}

/// Result alias for asset operations.
pub type Result<T, E = AssetError> = std::result::Result<T, E>;

impl AssetError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> AssetError {
        AssetError::Io {
            path: path.into(),
            source,
        }
    }
}
