# `capy-render` public API and render rules (owner: lead)

Pure-Rust map rendering, no Bevy: DT1 tile lookup with the reference variant
rules, a draw list in reference order, and a CPU compositor producing RGBA.
The Bevy view uses `TileSet` and `draw_list` for textures and z-order; the
oracle test and the headless CLI use `render_rgba`. Depends on `d2-formats`,
`d2-assets`.

## Rules (normative; cite `docs/specs/*.md` and reference lines in code comments)
- **Tile key** of a cell: floors `(0, main, sub)`; shadow `(13, main, sub)`;
  walls `(effective orientation, main, sub)` (`Ds1::orientation`, which applies
  DIR_LOOKUP for v<7). Orientation 10/11 cells are special tiles. Empty cells
  (`Cell::is_empty`) and hidden cells (bit 31) are not drawn.
- **Lookup fallbacks**, in order: the exact key; for orientation 18 try 19 and
  for 19 try 18; then the same orientation and main with sub 0. Report the cell
  as missing if nothing matches.
- **Variant choice** among tiles sharing the resolved key across the TileSet,
  which holds DT1s in `MapInfo::dt1_files` order and tiles in file order: the
  highest rarity wins and ties go to the first in load order (the documented
  game rule, recorded in PLAN.md). If every candidate has rarity 0, use the
  **last** matching tile of the **first** DT1 that has the key.
- **Wall pairs**: orientations 1–15 except 10, 11 and 13 draw as upper walls.
  Orientation 3 (and 4 per the specs) also draws its lower-wall companion with
  orientation 18 (19 for 4?), per `docs/specs/dt1.md`. Follow the reference
  exactly and cite it.
- **Placement** at 1:1: a cell's anchor is `((x - y) * 80, (x + y) * 40)`.
  A tile image's `offset_x`/`offset_y` from `Tile::render` is relative to that
  anchor. Walls and roofs also apply the reference's vertical adjustments
  (floor vs wall baseline, roof_height, orientation 13 shadow placement) exactly
  as `preview.c` does.
- **Pass order** (`preview.c:2574`), each pass row-major over cells (y outer, x
  inner) unless the reference iterates differently. Match it.
  1. LowerWalls (orientations 16–19), then Floors (floor layers in index
     order), then DT1 Shadows (shadow layer).
  2. ObjectShadows (none until objects exist).
  3. Objects with orderflag 1.
  4. UpperWalls interleaved with objects of orderflag 0/2, using the
     reference's interleave rule.
  5. Roofs (orientation 15).
  6. Special tiles (orientation 10/11). Shown only when `special` is visible.
- **Blend**:
  - Terrain tiles draw opaque with per-pixel transparency (`alpha == 0` skipped).
  - DT1 shadow tiles use the reference's shadow blend.
  - Sprite blend modes are keyed on COF `trans_b`: 25/50/75% alpha, additive,
    multiply, and (ONE, INV_SRC_COLOR) per `preview.c:291-380`. They apply to
    objects, which come later.
- **Palette**: `pal.dat` of `MapInfo::palette_act`, then gamma
  `255*(i/255)^(1/g)` (the reference default is 1.30). The byte order is
  whatever `d2_formats::palette` settled; the oracle test confirms it.

```rust
pub mod error;    pub use error::{RenderError, Result};
pub mod tileset;  pub use tileset::{TileSet, TileRef};
pub mod order;    pub use order::{DrawPass, DrawItem, Visibility, MissingTile, draw_list, map_screen_bounds};
pub mod compose;  pub use compose::{Blend, RenderOptions, RgbaImage, render_rgba, downscale};
pub mod compare;  pub use compare::{ImageDiff, compare_images};

#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum RenderError {
    #[error(transparent)] Asset(#[from] d2_assets::AssetError),
    #[error(transparent)] Format(#[from] d2_formats::FormatError),
    #[error("png: {0}")] Png(String),
    #[error("{0}")] Other(String),
}
pub type Result<T, E = RenderError> = std::result::Result<T, E>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TileRef { pub dt1: usize /* index into dt1 list */, pub tile: usize /* index in Dt1::tiles */ }
pub struct TileSet { /* dt1 paths, parsed Dt1s, key index, image cache (Mutex<HashMap<TileRef, Arc<IndexedImage>>>) */ }
impl TileSet {
    /// Loads and parses each DT1 through the overlay; unreadable ones become warnings.
    pub fn load(overlay: &d2_assets::Overlay, dt1_files: &[d2_assets::GamePath]) -> Result<(TileSet, Vec<String>)>;
    pub fn dt1_paths(&self) -> &[d2_assets::GamePath];
    pub fn tile(&self, r: &TileRef) -> Option<&d2_formats::dt1::Tile>;
    pub fn candidates(&self, key: d2_formats::dt1::TileKey) -> &[TileRef];      // load order
    pub fn resolve(&self, key: d2_formats::dt1::TileKey) -> Option<TileRef>;    // fallbacks + variant rule
    pub fn image(&self, r: &TileRef) -> Result<std::sync::Arc<d2_formats::IndexedImage>>; // cached render
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DrawPass { LowerWalls, Floors, Shadows, ObjectShadows, Objects1, UpperWalls, Roofs, Special }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visibility { pub floors: [bool; 2], pub walls: [bool; 4], pub shadow: bool, pub roofs: bool, pub special: bool, pub objects: bool }
impl Default for Visibility { /* all true except special=false */ }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawItem {
    pub pass: DrawPass, pub order: u32,      // stable order within the whole list
    pub layer: (char, u8),                   // ('f',i) | ('w',i) | ('s',0)
    pub x: u32, pub y: u32, pub tile: TileRef,
    pub screen_x: i32, pub screen_y: i32,    // 1:1 map-space position of the image's (0,0)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingTile { pub layer: (char, u8), pub x: u32, pub y: u32, pub key: d2_formats::dt1::TileKey }
/// Draw items in reference order plus every visible non-empty cell that has no tile.
pub fn draw_list(ds1: &d2_formats::ds1::Ds1, tiles: &TileSet, vis: &Visibility) -> Result<(Vec<DrawItem>, Vec<MissingTile>)>;
/// (min_x, min_y, max_x, max_y) of all draw items at 1:1, for canvas sizing/centering.
pub fn map_screen_bounds(items: &[DrawItem], tiles: &TileSet) -> Option<(i32, i32, i32, i32)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub use d2_formats::blend::Blend; // enum Blend { Normal, Alpha25, Alpha50, Alpha75, Additive, Multiply, Screen, Darken(u8) }; defined in d2-formats
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions { pub zoom_div: u32 /* 1..=16 */, pub gamma: f32, pub visibility: Visibility, pub background: [u8; 4] }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage { pub width: u32, pub height: u32, pub pixels: Vec<u8> }
impl RgbaImage { pub fn to_png(&self) -> Result<Vec<u8>>; pub fn from_png(bytes: &[u8]) -> Result<RgbaImage>; }
/// Composite at 1:1, then `downscale` by zoom_div with the method that best matches the oracle.
pub fn render_rgba(ds1: &d2_formats::ds1::Ds1, tiles: &TileSet, palette: &d2_formats::palette::Palette, opts: &RenderOptions) -> Result<(RgbaImage, Vec<MissingTile>)>;
pub fn downscale(img: &RgbaImage, div: u32) -> RgbaImage;

#[derive(Clone, Debug, PartialEq)]
pub struct ImageDiff { pub width: u32, pub height: u32, pub best_shift: (i32, i32), pub within_tolerance: f64 /* fraction of pixels with max channel diff <= tol */, pub mean_abs: f64 }
/// Compares after searching integer shifts in -max_shift..=max_shift (canvas origins may differ).
/// Pixels transparent/background in both images are excluded.
pub fn compare_images(ours: &RgbaImage, oracle: &RgbaImage, tolerance: u8, max_shift: i32) -> ImageDiff;
```

## Objects (render-objects lane)
Objects and monsters draw through `ObjectSprites` (borrowing the `Overlay` and an
`ObjectCatalog`); terrain-only `render`/`draw_list` are unchanged.

```rust
pub struct ObjectSprites<'a>;                     // ::new(&Overlay, &ObjectCatalog), caches COF + sprites per (key, direction, frame)
pub struct ObjectOptions { pub direction: u8, pub animation_tick: Option<u32>,
    pub frame_overrides: Option<Arc<[Option<u8>]>>, pub direction_overrides: Option<Arc<[Option<u8>]>> }
pub struct ObjectFrame { pub layers: Vec<FrameLayer>, pub offset: (i32, i32), pub order_flag: u8,
    pub frames_per_direction: u8 /* the COF's frame count (>= 1): what frame_index / forced frames wrap at */,
    pub directions: u8 /* the COF's direction count (>= 1): the user direction maps onto this many file directions */ }  // .composite(&Palette), .to_rgba()
/// User direction -> file direction (index into the sprite's and COF's directions), main.c:331-340:
/// 8/16/32 use the dir8/dir16/dir32 tables (user 0 = file 4; past the table = user 0's entry),
/// 4 is the user direction clamped to 3, any other count gives 0. Always < directions when > 0.
pub fn file_direction(directions: u8, user_dir: u8) -> u8;   // also re-exported at the crate root
pub fn frame_index(tick: Option<u32>, delta: u32, count: u32) -> u8;
pub struct FrameLayer { component, image: Arc<IndexedImage>, box_origin: (i32, i32), blend: Blend, shadow: Option<u8> }
pub struct MissingObject { index, kind, id, x, y, reason: MissingReason }
pub fn draw_list_with_objects(ds1, tiles, vis, Option<(&ObjectSprites, &ObjectOptions)>) -> Result<DrawList>;
pub struct DrawList { entries: Vec<DrawEntry>, missing: Vec<MissingTile>, missing_objects: Vec<MissingObject> }
pub enum DrawEntry { Tile(DrawItem), Object(ObjectDrawItem) }
pub fn render_with_objects(ds1, tiles, Option<&ObjectSprites>, palette, &RenderOptions) -> Result<RenderOutput>; // + RenderOutput::missing_objects
// RenderOptions gains animation_tick, object_direction, object_frames, object_directions.
// Blend gains Darken(level): the reference's object shadow, out = dst * level / 32.
```
Rules (`preview.c` line numbers in `order.rs` and `objects.rs`): object shadows (pass 2),
orderflag-1 objects (pass 3), then per tile the upper walls followed by that tile's
orderflag-0/2 objects (pass 4, over every map tile), in `editobj_set_drawing_order` order.
Anchor `((x-y+5)*16, 4+(x+y+1)*8)` at 1:1 (`object_anchor` does the zoomed integer maths).
Palette shifts (`ObjectInfo::colormap`): the layers with `trans_a == 0` have their visible pixel
indices mapped through the shift's row as the sprites load (`cof.c:343-361`), so
`FrameLayer::image` holds the shifted indices; a missing or short colormap draws unshifted.
`capy-sprite::compose_cof_frame` is not used: the reference draws each layer as its own
point-sampled bitmap, which a pre-composited frame cannot reproduce at zoom. The rules both
share live in `d2-formats`: sprite paths and the last-record rule (`d2_formats::cof`), the
`trans_a`/`trans_b` blend mapping (`Blend::for_cof_layer`). `capy-sprite` no longer depends
on this crate.

