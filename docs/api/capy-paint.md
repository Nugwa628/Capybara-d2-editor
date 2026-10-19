# `capy-paint` public API: palette-locked sprite editing (owner: lead)

The editing engine behind the Sprites-mode editor: a lightweight, palette-
locked "mini-GIMP" for DC6/DCC frames. Pure Rust, no Bevy, no I/O except the
explicit export functions. Every pixel is always a palette index or
transparent; no operation ever produces a colour outside the active palette.
The UI (canvas, panels) is built later in `capy-app`.

Frames are `d2_formats::IndexedImage` (alpha 0 = transparent, 255 = opaque;
`pixels[i]` is the palette index). A sprite is `frames[dir][frame]`, same
shape as `capy_sprite::Sprite::directions`.

```rust
pub mod error;    pub use error::{PaintError, Result};
pub mod ramps;    pub use ramps::{Ramps, RampId};
pub mod mask;     pub use mask::{Mask, MaskOp};
pub mod session;  pub use session::{PaintSession, FrameRef, Paint, Brush, BrushShape};
pub mod tools;    // methods on PaintSession (below)
pub mod check;    pub use check::{dcc_cell_violations, reduce_dcc_cells, CellViolation};
pub mod export;   pub use export::{palette_gpl, palette_act, palette_jasc, indexed_png, animated_gif, onion_composite};

#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum PaintError {
    #[error("frame {0:?} does not exist")] NoFrame(FrameRef),
    #[error("{0}")] Invalid(String),
    #[error("encode: {0}")] Encode(String),
}
pub type Result<T, E = PaintError> = std::result::Result<T, E>;

/// Colour families of a palette: indices grouped by hue/saturation, each ramp
/// sorted dark → light by perceived luminance. Index 0 belongs to no ramp.
pub struct Ramps { /* ... */ }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct RampId(pub u16);
impl Ramps {
    pub fn build(palette: &d2_formats::palette::Palette) -> Ramps;
    pub fn ramp_of(&self, index: u8) -> Option<(RampId, usize /* position */)>;
    pub fn ramp(&self, id: RampId) -> &[u8];
    pub fn all(&self) -> impl Iterator<Item = (RampId, &[u8])>;
    /// Step along the index's own ramp (negative = darker), clamped at the ends.
    pub fn shift(&self, index: u8, steps: i32) -> u8;
    /// Nearest palette index to an RGB colour, optionally restricted to a set; never 0 unless allowed.
    pub fn nearest(&self, rgb: [u8; 3], within: Option<&[u8]>, allow_zero: bool) -> u8;
}

/// Per-pixel selection for one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask { pub width: u32, pub height: u32, pub bits: Vec<bool> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum MaskOp { Replace, Add, Subtract, Intersect }
impl Mask {
    pub fn none(w: u32, h: u32) -> Mask; pub fn all(w: u32, h: u32) -> Mask;
    pub fn is_empty(&self) -> bool; pub fn bounds(&self) -> Option<(u32, u32, u32, u32)>;
    pub fn rect(w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> Mask;
    pub fn lasso(w: u32, h: u32, polygon: &[(f32, f32)]) -> Mask;        // pixel centres, even-odd
    pub fn ellipse(w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32) -> Mask;
    pub fn combine(&mut self, other: &Mask, op: MaskOp);
    pub fn invert(&mut self);
    pub fn grow(&self, px: u32) -> Mask; pub fn shrink(&self, px: u32) -> Mask;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FrameRef { pub dir: usize, pub frame: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Paint { Index(u8), Transparent }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum BrushShape { Square, Round }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Brush { pub size: u32, pub shape: BrushShape }

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum FillMode { Contiguous, Global }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ShapeStyle { Outline, Filled }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum GradientShape { Linear, Radial }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Dither { None, Bayer4, FloydSteinberg }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum GradientColors { Two(u8, u8), Ramp(RampId) /* dark → light */ }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum OutlineColor { Fixed(u8), AutoDarker(i32 /* ramp steps */) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum OutlineSide { Outside, Inside }

/// An editable sprite with unlimited undo/redo. Every mutating call is one
/// undo step (or part of an open group). No-op calls return Ok and record nothing.
pub struct PaintSession { /* frames, palette, ramps, undo/redo of pixel diffs, revision, saved revision */ }
impl PaintSession {
    pub fn new(frames: Vec<Vec<d2_formats::IndexedImage>>, palette: d2_formats::palette::Palette) -> Result<PaintSession>;
    pub fn frames(&self) -> &[Vec<d2_formats::IndexedImage>];
    pub fn frame(&self, f: FrameRef) -> Option<&d2_formats::IndexedImage>;
    pub fn into_frames(self) -> Vec<Vec<d2_formats::IndexedImage>>;
    pub fn palette(&self) -> &d2_formats::palette::Palette;
    pub fn ramps(&self) -> &Ramps;
    /// Swap palette (e.g. another act); pixels keep their indices. Not
    /// undoable; the session is dirty until the next `mark_saved`.
    pub fn set_palette(&mut self, palette: d2_formats::palette::Palette);
    /// Display-only swap (preview under another act): same palette/ramps
    /// change and `revision` bump as `set_palette`, but never dirties the
    /// session and records no history. A pending `set_palette` stays dirty.
    pub fn set_display_palette(&mut self, palette: d2_formats::palette::Palette);

    // history
    pub fn undo(&mut self) -> bool; pub fn redo(&mut self) -> bool;
    pub fn can_undo(&self) -> bool; pub fn can_redo(&self) -> bool;
    pub fn begin_group(&mut self, label: &str); pub fn end_group(&mut self);
    pub fn revision(&self) -> u64; pub fn is_dirty(&self) -> bool; pub fn mark_saved(&mut self);

    // painting (coordinates are frame pixels; out-of-frame points are clipped)
    // `mask`: when Some, only selected pixels change.
    pub fn pencil(&mut self, f: FrameRef, points: &[(i32, i32)], brush: Brush, paint: Paint, mask: Option<&Mask>) -> Result<()>;
    pub fn line(&mut self, f: FrameRef, a: (i32, i32), b: (i32, i32), brush: Brush, paint: Paint, mask: Option<&Mask>) -> Result<()>;
    pub fn rectangle(&mut self, f: FrameRef, a: (i32, i32), b: (i32, i32), style: ShapeStyle, brush: Brush, paint: Paint, mask: Option<&Mask>) -> Result<()>;
    pub fn ellipse(&mut self, f: FrameRef, a: (i32, i32), b: (i32, i32), style: ShapeStyle, brush: Brush, paint: Paint, mask: Option<&Mask>) -> Result<()>;
    pub fn fill(&mut self, f: FrameRef, at: (i32, i32), paint: Paint, mode: FillMode, mask: Option<&Mask>) -> Result<()>;   // exact index match
    pub fn replace_color(&mut self, f: Option<FrameRef> /* None = whole sprite */, from: Paint, to: Paint, mask: Option<&Mask>) -> Result<()>;
    pub fn pick(&self, f: FrameRef, at: (i32, i32)) -> Option<Paint>;
    /// Computed in RGB along the colours, then mapped to the palette (restricted
    /// to the ramp for Ramp) with the chosen dithering. Only opaque output.
    pub fn gradient(&mut self, f: FrameRef, from: (f32, f32), to: (f32, f32), shape: GradientShape, colors: GradientColors, dither: Dither, mask: Option<&Mask>) -> Result<()>;
    /// Lighten (steps > 0) / darken (steps < 0) opaque pixels along their own ramp.
    pub fn shade(&mut self, f: FrameRef, points: &[(i32, i32)], brush: Brush, steps: i32, mask: Option<&Mask>) -> Result<()>;
    /// Outline around opaque pixels (or around `mask` when given), 1..=8 px.
    pub fn outline(&mut self, f: FrameRef, color: OutlineColor, side: OutlineSide, thickness: u32, diagonals: bool, mask: Option<&Mask>) -> Result<()>;
    /// Remove isolated single opaque pixels / fill single transparent holes.
    pub fn clean_strays(&mut self, f: FrameRef, mask: Option<&Mask>) -> Result<()>;

    // selection content
    pub fn magic_wand(&self, f: FrameRef, at: (i32, i32), mode: FillMode) -> Mask;
    pub fn copy(&self, f: FrameRef, mask: &Mask) -> d2_formats::IndexedImage;   // cropped to mask bounds, offsets = position in frame
    pub fn cut(&mut self, f: FrameRef, mask: &Mask) -> Result<d2_formats::IndexedImage>;
    pub fn paste(&mut self, f: FrameRef, clip: &d2_formats::IndexedImage, at: (i32, i32)) -> Result<Mask>;  // transparent clip pixels don't overwrite
    /// RGBA from the OS clipboard or a file, mapped to the palette (transparent protection on).
    pub fn paste_rgba(&mut self, f: FrameRef, width: u32, height: u32, rgba: &[u8], at: (i32, i32), dither: Dither) -> Result<Mask>;
    pub fn move_selection(&mut self, f: FrameRef, mask: &Mask, dx: i32, dy: i32) -> Result<Mask>;
    pub fn flip(&mut self, f: FrameRef, mask: Option<&Mask>, horizontal: bool) -> Result<()>;
    pub fn rotate90(&mut self, f: FrameRef, mask: Option<&Mask>, clockwise: bool) -> Result<()>;

    // frame structure
    pub fn resize_canvas(&mut self, f: FrameRef, width: u32, height: u32, anchor: (i32, i32)) -> Result<()>;
    pub fn set_offset(&mut self, f: FrameRef, x: i32, y: i32) -> Result<()>;
    pub fn insert_frame(&mut self, at: usize, copy_from: Option<usize>) -> Result<()>;      // in every direction
    pub fn delete_frame(&mut self, index: usize) -> Result<()>;
    pub fn move_frame(&mut self, from: usize, to: usize) -> Result<()>;
    /// Horizontally mirrored copy of `src` direction into `dst` (offsets mirrored about the anchor).
    pub fn mirror_direction(&mut self, src: usize, dst: usize) -> Result<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellViolation {
    pub frame: FrameRef,
    pub cell_x: u32, pub cell_y: u32,      // column / row in the direction's 4x4 grid
    pub colors: usize,                     // distinct values, transparent counts as one
    /// The cell's pixels in frame-image coordinates, (x0, y0, x1, y1), end
    /// exclusive. Exactly the decoder's cell (`d2_formats::dcc::cell_spans`):
    /// 1..=5 pixels per side (partial first cell, merged trailing sliver).
    pub rect: (u32, u32, u32, u32),
}
/// DCC stores each 4x4 cell with at most 4 distinct pixel values (see docs/specs/dc6-dcc.md); list cells over the limit.
pub fn dcc_cell_violations(frames: &[Vec<d2_formats::IndexedImage>]) -> Vec<CellViolation>;
/// Reduce offending cells to 4 colours (merge nearest by RGB distance); returns how many cells changed.
pub fn reduce_dcc_cells(session: &mut PaintSession) -> Result<usize>;

pub fn palette_gpl(palette: &d2_formats::palette::Palette, name: &str) -> String;   // GIMP palette
pub fn palette_act(palette: &d2_formats::palette::Palette) -> Vec<u8>;             // Adobe .act (768 bytes)
pub fn palette_jasc(palette: &d2_formats::palette::Palette) -> String;             // JASC-PAL
/// PNG with PLTE (index 0 transparent via tRNS): edits in GIMP/Photoshop keep exact indices.
pub fn indexed_png(img: &d2_formats::IndexedImage, palette: &d2_formats::palette::Palette) -> Result<Vec<u8>>;
/// Animated GIF of one direction at `delay_cs` centiseconds per frame, uniform canvas = union box.
pub fn animated_gif(frames: &[d2_formats::IndexedImage], palette: &d2_formats::palette::Palette, delay_cs: u16) -> Result<Vec<u8>>;
/// RGBA preview: `current` over faded `prev`/`next` (onion skin), uniform canvas.
pub fn onion_composite(current: &d2_formats::IndexedImage, prev: Option<&d2_formats::IndexedImage>, next: Option<&d2_formats::IndexedImage>, palette: &d2_formats::palette::Palette, fade: f32) -> (u32, u32, Vec<u8>);
```

## Palette conversion (`capy_paint::remap`, P7)

Plans how the colours of one palette are carried into another. Nothing is
automatic: the app shows the plan, lets the user adjust it, and applies the
table only when asked.

```rust
/// Indices whose colour differs between the act palettes (and units/sky).
/// Every other index is identical in act1-act5, units and sky.
pub const ACT_SPECIFIC: std::ops::RangeInclusive<u8> = 225..=254;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ColorDistance { #[default] OkLab, Redmean, Rgb }

#[derive(Clone, Debug, PartialEq, Default)]
pub enum TargetSet {
    /// Any target index except 0.
    #[default] All,
    /// Skip ACT_SPECIFIC: the result looks the same under every act palette.
    SharedActColors,
    /// Only these target indices (0 is ignored).
    Only(Vec<u8>),
}

/// Applied to the source colours before matching; all zero = unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Adjust {
    pub brightness: f32,  // -1..=1, added to OkLab L
    pub contrast: f32,    // -1..=1, scales L around 0.5
    pub saturation: f32,  // -1..=1, scales OkLab chroma
    pub hue_degrees: f32, // rotates OkLab hue
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemapSettings {
    pub distance: ColorDistance,
    pub targets: TargetSet,
    /// Keep index i when the target palette has the same colour at i and i
    /// is allowed (default true; act1<->act5 then only touches 30 indices).
    pub keep_exact: bool,
    /// > 1 favours matching lightness over hue/chroma (OkLab L weight);
    /// default 1.0. Ignored by Rgb/Redmean.
    pub lightness_weight: f32,
    pub adjust: Adjust,
    /// Source index -> target index, wins over everything. Entries with 0 on
    /// either side are ignored.
    pub overrides: std::collections::BTreeMap<u8, u8>,
}
impl Default for RemapSettings { /* OkLab, All, keep_exact, 1.0, zero, empty */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind { Kept, Nearest, Override }

#[derive(Clone, Debug, PartialEq)]
pub struct RemapEntry {
    pub source: u8,
    pub target: u8,
    pub source_rgb: [u8; 3],
    pub target_rgb: [u8; 3],
    /// OkLab distance x 100 between the (unadjusted) source colour and the
    /// chosen target colour: ~0 identical, ~2 barely visible, > 10 obvious.
    pub error: f32,
    pub pixels: u64,
    pub how: MatchKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemapPlan {
    /// Valid d2_formats::remap::IndexTable: [0] = 0, nothing else maps to 0;
    /// unused indices map by the same rules.
    pub table: [u8; 256],
    /// Used source indices only (pixels > 0, excluding 0), worst error first.
    pub entries: Vec<RemapEntry>,
    /// Pixel-weighted mean error and the worst error over `entries`.
    pub mean_error: f32,
    pub worst_error: f32,
}

/// Deterministic; ties go to the lowest target index.
pub fn plan(src: &Palette, dst: &Palette, used: &[u64; 256], settings: &RemapSettings) -> RemapPlan;

/// The `k` best allowed targets for one source index, best first, with
/// their errors (for the override picker).
pub fn candidates(src: &Palette, dst: &Palette, source: u8, settings: &RemapSettings, k: usize) -> Vec<(u8, f32)>;

/// Maps the pixels of an image (alpha unchanged), for previews.
pub fn apply_to_image(img: &IndexedImage, table: &[u8; 256]) -> IndexedImage;
```
