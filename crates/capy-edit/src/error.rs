//! Error type for editing operations.

use crate::layer::LayerRef;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    #[error("layer {0:?} does not exist")]
    NoSuchLayer(LayerRef),
    #[error("cell ({x}, {y}) is outside the {width}x{height} map")]
    OutOfBounds {
        x: i64,
        y: i64,
        width: u32,
        height: u32,
    },
    /// The change needs a newer DS1 version (e.g. a 2nd wall layer in a v3 file).
    /// The app offers `Document::upgrade_version(required)`.
    #[error("needs DS1 version {required} (file is version {current})")]
    NeedsVersion { current: i32, required: i32 },
    #[error("layer limit reached: {0}")]
    LayerLimit(String),
    /// Kept for compatibility; no method returns it. An edit that would change
    /// nothing succeeds and records no history entry.
    #[error("nothing to do")]
    Empty,
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
}

pub type Result<T, E = EditError> = std::result::Result<T, E>;

/// A descriptive failure for requests that make no sense for the document
/// (bad object index, arithmetic overflow, a patch applied to the wrong state).
pub(crate) fn invalid(detail: impl Into<String>) -> EditError {
    EditError::Format(d2_formats::FormatError::Invalid {
        format: "ds1",
        what: "edit",
        offset: 0,
        detail: detail.into(),
    })
}
