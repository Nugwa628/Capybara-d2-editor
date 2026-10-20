//! Error type of the level-tool modules.

use d2_formats::FormatError;

/// Why a level-tool edit could not be made.
#[derive(Debug, thiserror::Error)]
pub enum ModError {
    /// The edit is refused, with the reason a person can act on.
    #[error("{0}")]
    Refused(String),
    /// A table or string edit failed.
    #[error(transparent)]
    Format(#[from] FormatError),
}

/// Result alias of the level-tool modules.
pub type Result<T, E = ModError> = std::result::Result<T, E>;

pub(crate) fn refuse<T>(message: impl Into<String>) -> Result<T> {
    Err(ModError::Refused(message.into()))
}
