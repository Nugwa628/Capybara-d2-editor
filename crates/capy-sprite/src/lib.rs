//! Viewer and converter logic for DC6/DCC/COF sprites (no Bevy, no GUI).
//!
//! The public API is `docs/api/capy-sprite.md`:
//!
//! * [`Sprite`]: palette-indexed frames, `directions[dir][frame]`, with
//!   [`Sprite::decode`] / [`Sprite::encode`] for DC6 and DCC;
//! * [`quantize`] and [`image_io`]: PNG/BMP/GIF/PCX in, PNG out, RGBA to
//!   palette indices with optional dithering and transparent protection;
//! * [`ops`]: split/join, trim, centre, add/remove frames and directions;
//! * [`transfer`]: frame folders with a `sprite.json` (exact offsets) and
//!   sprite sheets;
//! * [`batch`]: tree conversion in both directions;
//! * [`cof`]: component sprite paths and COF layer compositing.
//!
//! The encoders themselves live in `d2-formats` (`Dc6::from_frames`,
//! `Dcc::from_frames`); the format rules are in `docs/specs/dc6-dcc.md`.

pub mod batch;
pub mod cof;
pub mod error;
pub mod image_io;
pub mod model;
pub mod ops;
pub mod quantize;
pub mod transfer;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests;
#[cfg(test)]
mod testutil;

pub use batch::{BatchDirection, BatchOptions, BatchReport, batch_convert};
pub use cof::{CofLayerSprite, UnitKind, cof_component_paths, compose_cof_frame};
pub use error::{Result, SpriteError};
pub use image_io::{ImageKind, export_png, import_image};
pub use model::{RgbaFrame, Sprite, SpriteFormat};
pub use ops::{
    add_direction, center_offset, join_frames, offset_all, remove_directions, remove_frames,
    split_frame, trim_borders,
};
pub use quantize::{Dither, QuantizeOptions, quantize, to_rgba};
pub use transfer::{
    ExportMeta, FrameMeta, export_frames, export_sheet, import_frames, import_frames_subset,
    import_sheet,
};
