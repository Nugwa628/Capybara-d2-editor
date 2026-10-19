//! `capy-paint`: palette-locked sprite editing for DC6/DCC frames.
//!
//! A lightweight "mini-GIMP" engine. Frames are [`d2_formats::IndexedImage`]s;
//! a sprite is `frames[dir][frame]`. The public API is specified in
//! `docs/api/capy-paint.md`.
//!
//! # The palette lock
//!
//! After construction and after every operation, each pixel of every frame is
//! either transparent (alpha 0, index 0) or opaque (alpha 255, index 1..=255).
//! Colours are only ever read or produced through [`Ramps::nearest`],
//! [`Ramps::shift`], gradient mapping and `paste_rgba`; everything else moves
//! palette indices around.
//!
//! # Behaviours chosen where the contract was silent
//!
//! - **Corners are inclusive** for `rectangle`, `ellipse`, `Mask::rect` and
//!   `Mask::ellipse`.
//! - **`pencil` and `shade` join consecutive points with lines.**
//! - **`Paint::Index(0)` is an error** (`Invalid`); use `Paint::Transparent`.
//! - **A gradient without a mask covers the whole frame**, including
//!   transparent pixels; gradient endpoints are sampled at pixel coordinates.
//! - **`outline` with a mask outlines the mask region** (see its docs).
//! - **DCC cell check** treats transparent as a pixel value (it is palette
//!   index 0 in the DCC value table) and aligns the 4x4 grid to the direction
//!   box, the union of the frames' opaque bounding boxes.
//! - **`PaintSession::new` normalises** its input (alpha != 0 becomes 255,
//!   opaque index 0 becomes transparent) and requires the same number of
//!   frames in every direction.
//! - Extra accessors beyond the contract: `Ramps::len/is_empty/color`,
//!   `Mask::get/set/count`, `PaintSession::undo_label/redo_label`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::panic))]
#![allow(clippy::too_many_arguments)]

mod color;

pub mod check;
pub mod error;
pub mod export;
pub mod mask;
pub mod ramps;
pub mod remap;
pub mod session;
pub mod tools;

pub use check::{CellViolation, dcc_cell_violations, reduce_dcc_cells};
pub use error::{PaintError, Result};
pub use export::{
    animated_gif, indexed_png, onion_composite, palette_act, palette_gpl, palette_jasc,
};
pub use mask::{Mask, MaskOp};
pub use ramps::{RampId, Ramps};
pub use session::{Brush, BrushShape, FrameRef, Paint, PaintSession};
pub use tools::{
    Dither, FillMode, GradientColors, GradientShape, OutlineColor, OutlineSide, ShapeStyle,
};
