//! Pure parsers and writers for Diablo II data formats.
//!
//! Nothing here performs I/O beyond `&[u8]`, uses `unsafe`, or panics on
//! malformed data. The public API is specified in `docs/api/d2-formats.md`.

mod bytes;
pub mod error;
pub mod grid;
pub mod image;

pub mod animdata;
pub mod blend;
pub mod cof;
pub mod dc6;
pub mod dcc;
pub mod ds1;
pub mod dt1;
pub mod excel;
pub mod palette;
pub mod remap;
pub mod tables;
pub mod tbl;
pub mod txt;

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod robustness;
#[cfg(test)]
mod testutil;

pub use error::{FormatError, Result};
pub use grid::Grid;
pub use image::IndexedImage;
