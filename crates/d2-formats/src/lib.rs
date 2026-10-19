//! Pure parsers and writers for Diablo II data formats.
//!
//! Nothing here performs I/O beyond `&[u8]`, uses `unsafe`, or panics on
//! malformed data. The public API is specified in `docs/api/d2-formats.md`.

pub mod error;

pub use error::{FormatError, Result};
