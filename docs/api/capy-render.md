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
- **Placement** at 1:1: a cell's origin is `((x - y) * 80, (x + y) * 40)`
  (`mx`, `my` in `preview.c`). It is the **left edge** of the cell's 160x80
  box, on the row of the diamond's top corner; the diamond's top corner is
  `CELL_DIAMOND_TOP_X` (80) to the right of it, at `(mx + 80, my)`, and its
  left, right and bottom corners at `(mx, my + 40)`, `(mx + 160, my + 40)`
  and `(mx + 80, my + 80)`. At `1:div` the offset is `CELL_DIAMOND_TOP_X / div`.
  A tile image's `offset_x`/`offset_y` from `Tile::render` is relative to that
  origin. Walls and roofs also apply the reference's vertical adjustments
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
pub mod tileset;  pub use tileset::{TileSet, TileRef, CELL_DIAMOND_TOP_X};

/// x of a cell's diamond top corner relative to the cell origin
/// `((x - y) * 80, (x + y) * 40)`, at 1:1 (= 160 / 2).
pub const CELL_DIAMOND_TOP_X: i32 = 80;
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
`ObjectCatalog`, or holding them in `Arc`s); terrain-only `render`/`draw_list` are
unchanged.

```rust
pub struct ObjectSprites<'a>;                     // ::new(&Overlay, &ObjectCatalog), caches COF + sprites per (key, direction, frame)
pub type OwnedObjectSprites = ObjectSprites<'static>;
impl ObjectSprites<'static> {
    /// Holds the overlay and catalog through Arcs: 'static + Send + Sync, so the
    /// cache (and the frame Arcs it hands out) can live across builds.
    pub fn new_shared(overlay: Arc<Overlay>, catalog: Arc<ObjectCatalog>) -> OwnedObjectSprites;
}
impl<'a> ObjectSprites<'a> {
    pub fn overlay(&self) -> &Overlay;
    pub fn catalog(&self) -> &ObjectCatalog;
    /// Same overlay and catalog (by address): whether the cache can be reused.
    pub fn uses(&self, overlay: &Overlay, catalog: &ObjectCatalog) -> bool;
    pub fn place(&self, ds1: &Ds1, opts: &ObjectOptions) -> (Vec<PlacedObject>, Vec<MissingObject>);
    pub fn frame_for(&self, ds1: &Ds1, object: &Object, opts: &ObjectOptions, forced_frame: Option<u8>, direction: u8)
        -> Result<Arc<ObjectFrame>, MissingReason>;
    /// One frame at user `direction`: `forced_frame % count`, else the frame of
    /// opts.animation_tick at the object's speed. Err(NoLayers) for an empty frame.
    pub fn frame_of_info(&self, info: &ObjectInfo, opts: &ObjectOptions, forced_frame: Option<u8>, direction: u8)
        -> Result<Arc<ObjectFrame>, MissingReason>;
    /// Every frame of user `direction` (frame 0 first, frames_per_direction of
    /// them; the same cached Arcs frame_of_info returns) and the animation speed
    /// for frame_index: ObjectInfo::frame_delta (objects), else the AnimData speed
    /// of {token}{mode}{class} (monsters), a missing or 0 speed being 256.
    /// Empty frames stay in the list with no layers; Err is why the object has no
    /// sprite, or NoLayers when every frame is empty.
    pub fn frames_of_info(&self, info: &ObjectInfo, direction: u8)
        -> Result<(Vec<Arc<ObjectFrame>>, u32), MissingReason>;
}
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


## Map analysis (P10)

Pure modules, no Bevy; ports of ds1-studio's `walkArea`, `walkEdit` (read side),
`cellFlags`, `spawnRegions`, `mapOverlays`, `warps`, `exits`, `arrival`, `pops`,
`waterAnimation` (playback side) and Game view. Game-table data is always
passed in by the caller.

```rust
pub mod walk;   // sub-tile collision and the walkable-area badge
pub struct SubTileGrid { pub width: u32, pub height: u32, pub flags: Vec<u8> } // (cy*w+cx)*25 + sy*5 + sx
pub enum Collision { Editor, Game { blank: Option<[u8; 25]> } }
pub fn sub_tile_flags(ds1: &Ds1, tiles: &TileSet, mode: Collision) -> SubTileGrid;
pub fn blank_fill_flags(tiles: &TileSet, level_id: i32) -> [u8; 25];   // LvlPrest FillBlanks=1 tile
pub fn walkable_area(ds1: &Ds1, tiles: &TileSet) -> f64;               // tiles squared (Editor mode)
pub fn area_colour(area: f64) -> ([u8; 3], AreaBand);                  // 3300/3800/4300/5300 bands
pub const BLOCK_WALK: u8 = 1; BLOCK_LIGHT = 2; BLOCK_JUMP = 4; BLOCK_PLAYER_WALK = 8;

pub mod rooms;  // 8x8 game rooms, spawn regions, overviews
pub fn preset_rooms(w, h) -> Vec<PresetRoom>;  pub fn room_at(..);
pub fn room_regions(ds1: &Ds1, room: PresetRoom) -> RoomRegions;       // { room, regions, warp }
pub fn spawns(rr: &RoomRegions, r: &LogicRegion) -> bool;
pub fn partly_no_spawn_rooms(ds1: &Ds1) -> Vec<(RoomRegions, Vec<usize>)>;
pub fn open_void(ds1, &SubTileGrid) -> OpenVoid;
pub fn walkable_overlay(ds1, &SubTileGrid) -> Vec<WalkClass>;          // per sub-tile
pub fn spawn_overlay(ds1, &SubTileGrid, &SpawnLevel) -> Vec<SpawnClass>;

pub mod links;  // warps, exits, arrival, level light
pub struct LevelRow { id, name, vis: [i32; 8], warp: [i32; 8], intensity, rgb: [i32; 3] } // caller fills from Levels.txt
pub fn warp_tiles(ds1) -> BTreeMap<u8, Vec<(u32, u32)>>;
pub fn level_links(row, target: &dyn Fn(i32) -> LevelRef, warp_name: &dyn Fn(i32) -> Option<String>) -> [Option<WarpLink>; 8];
pub fn map_warps(ds1, &[Option<WarpLink>; 8]) -> Vec<MapWarp>;         // each warp tile + its target level
pub fn exit_problems(ds1, &LevelRow, &ExitOptions) -> Vec<ExitProblem>;
pub fn arrival_problem(ds1, is_waypoint: &dyn Fn(i32, i32) -> bool) -> Option<ArrivalProblem>;
pub struct LevelLight { intensity, rgb }  // multiplier() = (I/255)*(c/255); 0 = act daylight

pub mod pops;   // roof/wall hiding areas
pub fn find_pops(ds1) -> Vec<PopArea>;  hide_rect; trigger_rect(rect, pop_pad);
pub fn tiles_to_hide(..); pop_targets(..); pop_reach(ds1, &PopArea, pop_pad); pop_problems(ds1, &[PopArea], pops, pop_pad);

pub mod anim;   // animated floors and Game view
pub fn animation_frames(tiles: &TileSet, key: TileKey) -> Vec<TileRef>; // variants all animated, distinct rarities from 0
pub fn frame_at(n_frames: usize, seconds: f64) -> usize;               // 10 fps, looping
pub fn animated_floor_cells(ds1, tiles) -> Vec<AnimatedCell>;          // .frame(seconds)
pub struct GameView { width, height } // default 800x600, 320..=7680; world_rect(cx, cy), fit_zoom(vw, vh, max)
```

Rules and where ds1-studio keeps things:

- **Walkability** ORs the 25 sub-tile flags of every floor, wall and roof tile
  in a cell (shadows and special tiles excluded). Hidden tiles count, with every
  variant of their key. Visible tiles use the renderer's variant rule
  (ds1-studio picks a variant at random, so counts can differ by a few
  sub-tiles on maps whose variants differ in flags). DS1 cell bit 17
  (`prop3 & 2`, "unwalkable") blocks all 25 sub-tiles; bit 16 adds `BLOCK_JUMP`.
  `Collision::Editor` also blocks cells with no floor (the badge and WinDS1
  overlay); `Collision::Game` leaves them open, as D2Common builds collision.
- **Walk painting in ds1-studio writes both, via two tools.** Sub-tile painting
  (`walkEdit.ts`) writes DT1 flags: a per-map `<map>_walk.dt1` plus hidden
  blocker floors in the cells. The whole-cell tool (`cellFlags.ts`) writes the
  DS1 cell flag (bit 17). Neither is ported yet (needs the DT1 writer UI, P11).
- **Rooms**: 8x8 cells from the top corner over the DS1 size minus one; regions
  are flooded through first-layer walls with D2Common's tables; a region is a
  node when its seed has a hidden first-layer floor or main index 30; a room
  with a warp (special tile main 0-7) spawns nothing. `noSpawnPlan` (the edit
  planner) is not ported.
- **Level light** lives in Levels.txt (`Intensity`, `Red`, `Green`, `Blue`), not
  in the DS1; ds1-studio edits the map's Levels.txt row ("Level light" mode).
- **Animation**: the clock runs in game ticks (25/s); animated floors advance
  every 2.5 ticks (10 fps). Act 5 snow-style tiles (repeated rarities) do not
  animate.
