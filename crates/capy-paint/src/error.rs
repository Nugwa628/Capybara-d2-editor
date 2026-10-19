//! Errors for sprite editing and the export functions.

use crate::session::FrameRef;

/// Everything `capy-paint` can fail with. Out-of-range coordinates are never
/// an error (they are clipped); a frame that does not exist is.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PaintError {
    #[error("frame {0:?} does not exist")]
    NoFrame(FrameRef),
    #[error("{0}")]
    Invalid(String),
    #[error("encode: {0}")]
    Encode(String),
}

pub type Result<T, E = PaintError> = std::result::Result<T, E>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(PaintError::Invalid(msg.into()))
}
