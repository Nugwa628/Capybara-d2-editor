# Sprites mode: `d2-formats` encoders + `capy-sprite` (owner: lead)

Viewer and converter for DC6/DCC/COF (user request, 2026-10-19). The feature
list comes from SixDice v0.63's published README/CHANGES only. **SixDice's
license forbids reverse engineering: nobody opens, decompiles or runs its jar
for this work.** The encoders come from `docs/specs/dc6-dcc.md` and the format
guides, and they are verified against our own decoders.

## `d2-formats` additions (encoders)
```rust
impl Dc6Frame {
    /// RLE-encode an indexed image. Transparent runs and raw runs are each
    /// capped at 127 per control byte (a run of 128+ transparent pixels is
    /// split: Diablo II's decoder crashes otherwise); 0x80 ends each scanline.
    pub fn from_image(img: &IndexedImage) -> Dc6Frame;   // offsets: inverse of Dc6Frame::render
}
impl Dc6 {
    /// frames[dir][frame]; every direction must have the same frame count.
    pub fn from_frames(frames: &[Vec<IndexedImage>]) -> Result<Dc6>;  // version 6, flags 1, encoding 0, termination 0xEEEEEEEE
    pub fn to_bytes(&self) -> Result<Vec<u8>>;
}
impl Dcc {
    /// Builds a version-6 DCC: per direction a bounding box, frame headers,
    /// and the four optional streams chosen per direction; no state may leak
    /// between frames beyond what the format defines (pixel buffer per
    /// direction, fresh per file).
    pub fn from_frames(frames: &[Vec<IndexedImage>]) -> Result<Dcc>;
    pub fn to_bytes(&self) -> Result<Vec<u8>>;
}
```
Invariant tested on every DC6 and DCC in both corpora:
`parse(to_bytes(parse(x)))` yields the same frames (pixels, alpha, offsets)
as `parse(x)`. Byte-identical output is not required.

## `capy-sprite` (new crate, pure Rust, no Bevy)
```rust
pub mod error;    pub use error::{SpriteError, Result};
pub mod model;    pub use model::{Sprite, SpriteFormat, RgbaFrame};
pub mod quantize; pub use quantize::{QuantizeOptions, Dither, quantize, to_rgba};
pub mod image_io; pub use image_io::{import_image, export_png, ImageKind};   // PNG, BMP, GIF, PCX (own PCX reader)
pub mod ops;      pub use ops::{split_frame, join_frames, trim_borders, center_offset, offset_all, add_direction, remove_directions, remove_frames};
pub mod transfer; pub use transfer::{ExportMeta, FrameMeta, export_frames, import_frames, export_sheet, import_sheet};
pub mod batch;    pub use batch::{BatchDirection, BatchOptions, BatchReport, batch_convert};
pub mod cof;      pub use cof::{CofLayerSprite, compose_cof_frame, cof_component_paths, UnitKind};

#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum SpriteError {
    #[error(transparent)] Format(#[from] d2_formats::FormatError),
    #[error("I/O error on {path}: {source}")] Io { path: std::path::PathBuf, #[source] source: std::io::Error },
    #[error("unsupported image: {0}")] Image(String),
    #[error("{0}")] Invalid(String),
    #[error("cancelled")] Cancelled,
}
pub type Result<T, E = SpriteError> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SpriteFormat { Dc6, Dcc }
/// Palette-indexed frames, frames[dir][frame]; equal frame counts per direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprite { pub directions: Vec<Vec<d2_formats::IndexedImage>> }
impl Sprite {
    pub fn decode(bytes: &[u8], format: SpriteFormat) -> Result<Sprite>;
    pub fn encode(&self, format: SpriteFormat) -> Result<Vec<u8>>;
    pub fn frames_per_direction(&self) -> usize;
    /// Union box of all frames (anchor space): (min_x, min_y, max_x, max_y).
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFrame { pub width: u32, pub height: u32, pub offset_x: i32, pub offset_y: i32, pub pixels: Vec<u8> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dither { None, FloydSteinberg }
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QuantizeOptions {
    pub dither: Dither,                       // default None
    /// Opaque RGB treated as transparent (D2 convention: black). None = alpha only.
    pub transparent_rgb: Option<[u8; 3]>,     // default Some([0,0,0])
    pub alpha_threshold: u8,                  // alpha < threshold → transparent; default 128
    /// Never map an opaque pixel to palette index 0 (near-black stays visible). Default true.
    pub protect_transparent: bool,
}
pub fn quantize(frame: &RgbaFrame, palette: &d2_formats::palette::Palette, opts: &QuantizeOptions) -> d2_formats::IndexedImage;
pub fn to_rgba(img: &d2_formats::IndexedImage, palette: &d2_formats::palette::Palette) -> RgbaFrame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ImageKind { Png, Bmp, Gif, Pcx }
pub fn import_image(bytes: &[u8], kind: ImageKind) -> Result<RgbaFrame>;    // offsets 0
pub fn export_png(frame: &RgbaFrame) -> Result<Vec<u8>>;

pub fn split_frame(img: &d2_formats::IndexedImage, tile_w: u32, tile_h: u32) -> Vec<d2_formats::IndexedImage>; // row-major, offsets kept consistent
pub fn join_frames(frames: &[d2_formats::IndexedImage], columns: u32) -> Result<d2_formats::IndexedImage>;
pub fn trim_borders(img: &d2_formats::IndexedImage) -> d2_formats::IndexedImage;    // keeps on-screen position; all-transparent → 1x1 transparent
pub fn center_offset(img: &mut d2_formats::IndexedImage);                         // anchor at bottom-centre
pub fn offset_all(sprite: &mut Sprite, dx: i32, dy: i32);
pub fn add_direction(sprite: &mut Sprite, frames: Vec<d2_formats::IndexedImage>) -> Result<()>;
pub fn remove_directions(sprite: &mut Sprite, dirs: &[usize]);
pub fn remove_frames(sprite: &mut Sprite, frames: &[usize]);                       // same indices from every direction

/// `sprite.json` written next to exported PNGs so re-import restores offsets exactly.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FrameMeta { pub dir: u32, pub frame: u32, pub file: String, pub offset_x: i32, pub offset_y: i32 }
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportMeta { pub version: u32, pub format: SpriteFormat, pub directions: u32, pub frames_per_direction: u32, pub palette: String, pub source: Option<String>, pub frames: Vec<FrameMeta> }
/// Writes `d{dir:02}_f{frame:03}.png` + `sprite.json` into `dir`.
pub fn export_frames(sprite: &Sprite, format: SpriteFormat, palette: &d2_formats::palette::Palette, palette_name: &str, source: Option<&str>, dir: &std::path::Path) -> Result<ExportMeta>;
/// Reads sprite.json when present (exact offsets); otherwise every image in
/// natural sort order as one direction, offsets from `center_offset`.
pub fn import_frames(dir: &std::path::Path, palette: &d2_formats::palette::Palette, opts: &QuantizeOptions) -> Result<(Sprite, Option<ExportMeta>)>;
/// One PNG: rows = directions, columns = frames, uniform cells of the union box.
pub fn export_sheet(sprite: &Sprite, palette: &d2_formats::palette::Palette) -> Result<(Vec<u8>, ExportMeta)>;
pub fn import_sheet(png: &[u8], directions: u32, frames: u32, palette: &d2_formats::palette::Palette, opts: &QuantizeOptions, meta: Option<&ExportMeta>) -> Result<Sprite>;

#[derive(Clone, Debug, PartialEq)]
pub enum BatchDirection { ToSprite(SpriteFormat), ToPng }
#[derive(Clone, Debug, PartialEq)]
pub struct BatchOptions { pub recursive: bool, pub quantize: QuantizeOptions, pub split: Option<(u32, u32)>, pub overwrite: bool }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchReport { pub converted: usize, pub skipped: usize, pub failed: Vec<(std::path::PathBuf, String)>, pub log: Vec<String> }
/// Mirrors the input tree under `output`. ToPng: each sprite → a folder of
/// frames + sprite.json. ToSprite: each folder with sprite.json (or a lone
/// image) → one sprite file. Never writes inside `input`.
pub fn batch_convert(input: &std::path::Path, output: &std::path::Path, direction: &BatchDirection, palette: &d2_formats::palette::Palette, opts: &BatchOptions,
                     progress: &mut dyn FnMut(usize, usize, &str), cancel: &std::sync::atomic::AtomicBool) -> Result<BatchReport>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum UnitKind { Object, Monster, Character }
impl UnitKind { pub fn base_dir(self) -> &'static str; } // data\global\{objects|monsters|chars}
pub use d2_formats::cof::COMPONENT_CODES;                // as cof::COMPONENT_CODES
/// Component sprite paths for a COF layer per docs/specs/cof.md: `.dcc` first,
/// then `.dc6` as a separate candidate (the caller picks what resolves).
/// Delegates to `d2_formats::cof::component_paths`, which capy-render uses too.
pub fn cof_component_paths(kind: UnitKind, token: &str, mode: &str, weapon_class: &str, component: &str, armor: &str) -> Vec<String>;
pub struct CofLayerSprite { pub component: u8, pub sprite: Sprite, pub draw_effect: u8, pub transparent: bool }
/// Composite one (file direction, frame) with the COF draw order and blend modes
/// (`d2_formats::blend::Blend::for_cof_layer`; this crate depends on d2-formats, not capy-render).
pub fn compose_cof_frame(cof: &d2_formats::cof::Cof, layers: &[CofLayerSprite], dir: u8, frame: u8, palette: &d2_formats::palette::Palette) -> Result<RgbaFrame>;
```
