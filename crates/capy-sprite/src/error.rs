//! Error type of the crate.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SpriteError {
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// An image file that cannot be read or written (bad data, unsupported
    /// PCX variant, ...).
    #[error("unsupported image: {0}")]
    Image(String),
    /// Arguments or data that do not make sense (mismatched frame counts,
    /// a path outside its folder, ...).
    #[error("{0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T, E = SpriteError> = std::result::Result<T, E>;

impl SpriteError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        SpriteError::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        SpriteError::Invalid(msg.into())
    }
}
