//! Pure-Rust map rendering for the Capybara Map Editor (no Bevy).
//!
//! * [`TileSet`]: DT1 tiles, key index, the reference's variant rule and
//!   fallbacks.
//! * [`draw_list`]: which tile goes where, in the reference's draw order.
//! * [`render_rgba`]: a CPU compositor, used by the oracle test and the
//!   headless CLI.
//! * [`compare_images`]: image comparison against the DS1Edit oracle renders.
//!
//! The rules are `docs/api/capy-render.md`; the reference they were checked
//! against is `reference/d2-ds1-edit/src/render/preview.c`.

pub mod compare;
pub mod compose;
pub mod error;
pub mod order;
pub mod palette;
pub mod tileset;

#[cfg(test)]
mod testutil;

pub use compare::{
    CompareOptions, CompareReport, Coverage, ImageDiff, compare_images, compare_images_with,
};
pub use compose::{
    Blend, RenderOptions, RenderOutput, RgbaImage, downscale, reference_bounds, render, render_rgba,
};
pub use error::{RenderError, Result};
pub use order::{DrawItem, DrawPass, MissingTile, Visibility, draw_list, map_screen_bounds};
pub use palette::{palette_act, tileset_act};
pub use tileset::{TileClass, TileGeometry, TileRef, TileSet, VariantRule};
