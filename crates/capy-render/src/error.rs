//! Error type of the crate.

/// Everything `capy-render` can fail with.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    #[error(transparent)]
    Asset(#[from] d2_assets::AssetError),
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
    #[error("png: {0}")]
    Png(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = RenderError> = std::result::Result<T, E>;
