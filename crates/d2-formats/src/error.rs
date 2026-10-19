//! Error type shared by every format parser.

/// Why a buffer could not be parsed or a model could not be written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FormatError {
    #[error("{format}: unexpected end of data at offset {offset:#x} (needed {needed} more bytes)")]
    Truncated {
        format: &'static str,
        offset: usize,
        needed: usize,
    },
    #[error("{format}: invalid {what} at offset {offset:#x}: {detail}")]
    Invalid {
        format: &'static str,
        what: &'static str,
        offset: usize,
        detail: String,
    },
    #[error("{format}: unsupported version {version}")]
    UnsupportedVersion { format: &'static str, version: i64 },
    #[error("{format}: missing column {column:?}")]
    MissingColumn {
        format: &'static str,
        column: String,
    },
    /// The model cannot be written in the requested file version
    /// (e.g. two wall layers in a v3 DS1).
    #[error("{format}: cannot write version {version}: {detail}")]
    NotRepresentable {
        format: &'static str,
        version: i64,
        detail: String,
    },
}

/// Result alias for format operations.
pub type Result<T, E = FormatError> = std::result::Result<T, E>;
