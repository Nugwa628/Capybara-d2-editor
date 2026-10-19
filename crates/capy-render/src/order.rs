//! The draw list: which tile goes where, in the reference's order.
//!
//! Citations are `reference/d2-ds1-edit/src/render/preview.c` unless noted.
//!
//! # Order
//!
//! `wpreview_draw_tiles_big_screenshot` (3364) and `wpreview_draw_tiles`
//! (2574) draw, each pass row-major over the cells (`y` outer, `x` inner):
//!
//! 1. per cell: lower walls, then floors, then DT1 shadows (the three are
//!    interleaved per cell, not three separate sweeps);
//! 2. object shadows (`wpreview_draw_obj_tile_shad`, 2306);
//! 3. objects with order flag 1 (`wpreview_draw_obj_tile_1`, 2429);
//! 4. per cell: upper walls, then the cell's objects with order flag 0/2
//!    (`wpreview_draw_obj_tile_0_2`, 2361; loop 3 at 3675-3697);
//! 5. roofs;
//! 6. special tiles (orientation 10 and 11), only when shown.
//!
//! Objects are visited in `editobj_set_drawing_order` order (`objects.c:15-80`):
//! tile row, tile column, the sub-tile table, then file order. Every object
//! of a tile is drawn when the pass reaches that tile, so in pass 4 an object
//! lands after the upper walls of its tile and before the walls of every
//! later tile, including tiles that hold no wall at all.
//!
//! Inside one cell the layers of one family are drawn by ascending `prop1`
//! (ties: lower layer index first), `wpreview_sort_order_data` (557); for
//! walls an orientation-10 cell counts as 255 (1087).
//!
//! The list order is the draw order. [`DrawItem::pass`] says which pass an
//! item belongs to; because pass 1 interleaves three families per cell, items
//! of `LowerWalls`, `Floors` and `Shadows` alternate in the list.

use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::TileKey;

use std::sync::Arc;

use crate::error::Result;
use crate::objects::{MissingObject, ObjectFrame, ObjectOptions, ObjectSprites, PlacedObject};
use crate::tileset::{TILE_H, TILE_W, TileClass, TileRef, TileSet};

/// Which sweep an item belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DrawPass {
    LowerWalls,
    Floors,
    Shadows,
    ObjectShadows,
    Objects1,
    UpperWalls,
    Roofs,
    Special,
}

/// Layer toggles. Default: everything on except special tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visibility {
    pub floors: [bool; 2],
    pub walls: [bool; 4],
    pub shadow: bool,
    pub roofs: bool,
    pub special: bool,
    pub objects: bool,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility {
            floors: [true; 2],
            walls: [true; 4],
            shadow: true,
            roofs: true,
            special: false,
            objects: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawItem {
    pub pass: DrawPass,
    /// Position in the list (stable order within the whole list).
    pub order: u32,
    /// `('f', i)` floor layer, `('w', i)` wall layer, `('s', 0)` shadow.
    pub layer: (char, u8),
    pub x: u32,
    pub y: u32,
    pub tile: TileRef,
    /// 1:1 map-space position of the image's (0,0), where the image is
    /// [`TileSet::image`] (its crop offset is already included).
    pub screen_x: i32,
    pub screen_y: i32,
}

/// A visible, non-empty cell for which no tile was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingTile {
    pub layer: (char, u8),
    pub x: u32,
    pub y: u32,
    pub key: TileKey,
}

/// One object (or monster) sprite in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectDrawItem {
    /// `ObjectShadows` for projected shadows, `Objects1` for order flag 1,
    /// `UpperWalls` for objects drawn in the upper-wall pass (order flag 0/2).
    pub pass: DrawPass,
    /// Position in the list.
    pub order: u32,
    /// Index into `Ds1::objects`.
    pub object: usize,
    pub kind: i32,
    pub id: i32,
    /// DS1 sub-tile coordinates (5 per tile).
    pub x: i32,
    pub y: i32,
    /// The tile the object stands on.
    pub tile: (u32, u32),
    /// 1:1 map-space position of the sprite anchor (`preview.c:2084-2086`);
    /// each layer sits at `anchor + frame.offset + layer.box_origin`.
    pub screen_x: i32,
    pub screen_y: i32,
    /// For an `ObjectShadows` item only the layers with `shadow` are drawn,
    /// each as a darkening silhouette ([`crate::Blend::Darken`]).
    pub frame: Arc<ObjectFrame>,
}

/// An entry of [`DrawList`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawEntry {
    Tile(DrawItem),
    Object(ObjectDrawItem),
}

impl DrawEntry {
    pub fn pass(&self) -> DrawPass {
        match self {
            DrawEntry::Tile(t) => t.pass,
            DrawEntry::Object(o) => o.pass,
        }
    }
}

/// The full draw list: tiles and objects in reference order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrawList {
    pub entries: Vec<DrawEntry>,
    pub missing: Vec<MissingTile>,
    /// Objects that draw nothing (only filled when objects are visible).
    pub missing_objects: Vec<MissingObject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Wall,
    Floor,
    Shadow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Lower,
    Floor,
    Shadow,
    Upper,
    Roof,
    Special,
}

#[derive(Clone, Debug)]
struct Entry {
    x: u32,
    y: u32,
    family: Family,
    layer: (char, u8),
    height: u8,
    kind: Kind,
    tile: TileRef,
}

fn main_index(cell: Cell) -> u8 {
    cell.main_index()
}

/// Layer toggle lookup that treats a layer beyond the visibility arrays as on.
fn layer_on(flags: &[bool], i: usize) -> bool {
    flags.get(i).copied().unwrap_or(true)
}

/// Collects every drawable entry, sorted by (y, x, family, prop1, layer).
fn collect(
    ds1: &Ds1,
    tiles: &TileSet,
    vis: &Visibility,
    missing: &mut Vec<MissingTile>,
) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let (map_w, map_h) = crate::compose::cell_extent(ds1);
    for y in 0..map_h {
        for x in 0..map_w {
            // Wall layers (and roofs, special tiles).
            for (i, layer) in ds1.walls.iter().enumerate() {
                if !layer_on(&vis.walls, i) {
                    continue;
                }
                let Some(cell) = layer.tiles.get(x, y) else {
                    continue;
                };
                if cell.is_empty() {
                    continue;
                }
                let layer_id = ('w', u8::try_from(i).unwrap_or(u8::MAX));
                let Some(orientation) = ds1.orientation(i, x, y) else {
                    missing.push(MissingTile {
                        layer: layer_id,
                        x,
                        y,
                        key: TileKey {
                            orientation: u8::MAX,
                            main: main_index(cell),
                            sub: cell.sub_index(),
                        },
                    });
                    continue;
                };
                let special = orientation == 10 || orientation == 11;
                // preview.c:1121-1132: a hidden wall cell is skipped (shown
                // as an "unknown" marker only with the special layer on) but
                // special tiles ignore the bit (1113-1119, 1461-1470).
                if cell.hidden() && !special {
                    continue;
                }
                if special && !vis.special {
                    continue;
                }
                if orientation == 15 && !vis.roofs {
                    continue;
                }
                let key = TileKey {
                    orientation,
                    main: main_index(cell),
                    sub: cell.sub_index(),
                };
                let Some(tile) = tiles.resolve(key) else {
                    missing.push(MissingTile {
                        layer: layer_id,
                        x,
                        y,
                        key,
                    });
                    continue;
                };
                let Some(class) = tiles
                    .tile(&tile)
                    .map(|t| TileClass::of_orientation(t.orientation))
                else {
                    continue;
                };
                // The pass comes from the cell's orientation (1102-1106), and
                // the tile must be of a wall type (1196-1202).
                let kind = match (orientation, class) {
                    (10 | 11, TileClass::Special) => Kind::Special,
                    (15, TileClass::Roof) => Kind::Roof,
                    (16.., TileClass::LowerWall) => Kind::Lower,
                    (..15, TileClass::UpperWall) => Kind::Upper,
                    _ => continue,
                };
                let mut height = cell.prop1();
                if orientation == 10 {
                    height = 255;
                }
                entries.push(Entry {
                    x,
                    y,
                    family: Family::Wall,
                    layer: layer_id,
                    height,
                    kind,
                    tile,
                });
            }
            // Floors.
            for (i, layer) in ds1.floors.iter().enumerate() {
                if !layer_on(&vis.floors, i) {
                    continue;
                }
                let Some(cell) = layer.get(x, y) else {
                    continue;
                };
                if cell.is_empty() || cell.hidden() {
                    continue;
                }
                let layer_id = ('f', u8::try_from(i).unwrap_or(u8::MAX));
                let key = TileKey {
                    orientation: 0,
                    main: main_index(cell),
                    sub: cell.sub_index(),
                };
                let Some(tile) = tiles.resolve(key) else {
                    missing.push(MissingTile {
                        layer: layer_id,
                        x,
                        y,
                        key,
                    });
                    continue;
                };
                entries.push(Entry {
                    x,
                    y,
                    family: Family::Floor,
                    layer: layer_id,
                    height: cell.prop1(),
                    kind: Kind::Floor,
                    tile: tiles.animation_frame0(&tile),
                });
            }
            // Shadow.
            if vis.shadow
                && let Some(cell) = ds1.shadow.get(x, y)
                && !cell.is_empty()
                && !cell.hidden()
            {
                let key = TileKey {
                    orientation: 13,
                    main: main_index(cell),
                    sub: cell.sub_index(),
                };
                match tiles.resolve(key) {
                    Some(tile) => entries.push(Entry {
                        x,
                        y,
                        family: Family::Shadow,
                        layer: ('s', 0),
                        height: cell.prop1(),
                        kind: Kind::Shadow,
                        tile,
                    }),
                    None => missing.push(MissingTile {
                        layer: ('s', 0),
                        x,
                        y,
                        key,
                    }),
                }
            }
        }
    }
    // Stable sort: (y, x, family) keeps the generation order across cells and
    // `wpreview_sort_order_data` orders layers of a family by (prop1, index).
    entries.sort_by_key(|e| (e.y, e.x, e.family as u8, e.height, e.layer.1));
    entries
}

/// Draw items in reference order plus every visible non-empty cell that has no
/// tile. See the module docs for the order. Terrain only; objects need
/// [`draw_list_with_objects`].
pub fn draw_list(
    ds1: &Ds1,
    tiles: &TileSet,
    vis: &Visibility,
) -> Result<(Vec<DrawItem>, Vec<MissingTile>)> {
    let list = draw_list_with_objects(ds1, tiles, vis, None)?;
    let items = list
        .entries
        .into_iter()
        .filter_map(|e| match e {
            DrawEntry::Tile(t) => Some(t),
            DrawEntry::Object(_) => None,
        })
        .collect();
    Ok((items, list.missing))
}

/// [`draw_list`] with objects: shadows, order-flag-1 objects and the
/// objects interleaved with the upper walls (module docs). `objects` is the
/// sprite source and its options; `None`, or `vis.objects == false`, gives
/// terrain only.
pub fn draw_list_with_objects(
    ds1: &Ds1,
    tiles: &TileSet,
    vis: &Visibility,
    objects: Option<(&ObjectSprites<'_>, &ObjectOptions)>,
) -> Result<DrawList> {
    let mut missing = Vec::new();
    let entries = collect(ds1, tiles, vis, &mut missing);
    let mut out: Vec<DrawEntry> = Vec::with_capacity(entries.len());

    let push =
        |out: &mut Vec<DrawEntry>, pass: DrawPass, e: &Entry, tile: &TileRef| -> Result<()> {
            let Some(geom) = tiles.geometry(tile) else {
                return Ok(());
            };
            if geom.is_empty() {
                return Ok(()); // no bitmap: nothing to draw (1186)
            }
            let image = tiles.image(tile)?;
            let x = i32::try_from(e.x).unwrap_or(i32::MAX);
            let y = i32::try_from(e.y).unwrap_or(i32::MAX);
            let mx = x.saturating_sub(y).saturating_mul(TILE_W / 2);
            let my = x.saturating_add(y).saturating_mul(TILE_H / 2);
            out.push(DrawEntry::Tile(DrawItem {
                pass,
                order: u32::try_from(out.len()).unwrap_or(u32::MAX),
                layer: e.layer,
                x: e.x,
                y: e.y,
                tile: tile.clone(),
                screen_x: mx.saturating_add(image.offset_x),
                screen_y: my
                    .saturating_add(geom.bitmap_top(1))
                    .saturating_add(image.offset_y),
            }));
            Ok(())
        };

    // Objects in drawing order.
    let (placed, missing_objects) = match objects {
        Some((sprites, opts)) if vis.objects => sprites.place(ds1, opts),
        _ => (Vec::new(), Vec::new()),
    };

    // Entries are sorted by cell; walk them as per-cell groups.
    let groups = cell_groups(&entries);

    // Pass 1: lower walls, floors, shadows (1a).
    for g in &groups {
        for (kind, pass) in [
            (Kind::Lower, DrawPass::LowerWalls),
            (Kind::Floor, DrawPass::Floors),
            (Kind::Shadow, DrawPass::Shadows),
        ] {
            for e in g.iter().filter(|e| e.kind == kind) {
                push(&mut out, pass, e, &e.tile)?;
            }
        }
    }
    // Pass 2 (1B, 3629): shadows of objects, every object, drawing order.
    // Pass 3 (2, 3651): objects with order flag 1.
    // Both only need the objects' own order: the reference's per-tile cursor
    // visits them in that order.
    let object_item = |pass: DrawPass, p: &PlacedObject, order: usize| {
        let (sx, sy) = crate::objects::object_anchor(p.x, p.y, 1);
        DrawEntry::Object(ObjectDrawItem {
            pass,
            order: u32::try_from(order).unwrap_or(u32::MAX),
            object: p.index,
            kind: p.kind,
            id: p.id,
            x: p.x,
            y: p.y,
            tile: p.tile(),
            screen_x: sx,
            screen_y: sy,
            frame: Arc::clone(&p.frame),
        })
    };
    for p in placed.iter().filter(|p| p.frame.has_shadow()) {
        let item = object_item(DrawPass::ObjectShadows, p, out.len());
        out.push(item);
    }
    for p in placed.iter().filter(|p| p.frame.order_flag == 1) {
        let item = object_item(DrawPass::Objects1, p, out.len());
        out.push(item);
    }
    // Pass 4 (loop 3, 3675): per tile, the upper walls and then the tile's
    // objects with order flag 0 or 2, over every tile of the map.
    let front: Vec<&PlacedObject> = placed
        .iter()
        .filter(|p| matches!(p.frame.order_flag, 0 | 2))
        .collect();
    let (mut gi, mut oi) = (0, 0);
    while gi < groups.len() || oi < front.len() {
        let group_cell = groups.get(gi).and_then(|g| g.first()).map(|e| (e.y, e.x));
        let object_cell = front.get(oi).map(|p| {
            let (tx, ty) = p.tile();
            (ty, tx)
        });
        let cell = match (group_cell, object_cell) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        if group_cell == Some(cell)
            && let Some(g) = groups.get(gi)
        {
            // Upper walls, each followed by the orientation-3 corner partner
            // (1131-1160).
            for e in g.iter().filter(|e| e.kind == Kind::Upper) {
                push(&mut out, DrawPass::UpperWalls, e, &e.tile)?;
                if let Some(partner) = tiles.corner_pair(&e.tile) {
                    push(&mut out, DrawPass::UpperWalls, e, &partner)?;
                }
            }
            gi += 1;
        }
        while let Some(p) = front.get(oi) {
            let (tx, ty) = p.tile();
            if (ty, tx) != cell {
                break;
            }
            let item = object_item(DrawPass::UpperWalls, p, out.len());
            out.push(item);
            oi += 1;
        }
    }
    // Pass 5: roofs.
    for g in &groups {
        for e in g.iter().filter(|e| e.kind == Kind::Roof) {
            push(&mut out, DrawPass::Roofs, e, &e.tile)?;
        }
    }
    // Pass 6: special tiles (only collected when visible).
    for g in &groups {
        for e in g.iter().filter(|e| e.kind == Kind::Special) {
            push(&mut out, DrawPass::Special, e, &e.tile)?;
        }
    }
    Ok(DrawList {
        entries: out,
        missing,
        missing_objects,
    })
}

/// Splits sorted entries into runs of one cell.
fn cell_groups(entries: &[Entry]) -> Vec<&[Entry]> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < entries.len() {
        let (x, y) = (entries[start].x, entries[start].y);
        let mut end = start + 1;
        while end < entries.len() && entries[end].x == x && entries[end].y == y {
            end += 1;
        }
        groups.push(&entries[start..end]);
        start = end;
    }
    groups
}

/// `(min_x, min_y, max_x, max_y)` of all draw items at 1:1, `max` exclusive,
/// for canvas sizing and centering. `None` for an empty list.
pub fn map_screen_bounds(items: &[DrawItem], tiles: &TileSet) -> Option<(i32, i32, i32, i32)> {
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    for item in items {
        let Ok(image) = tiles.image(&item.tile) else {
            continue;
        };
        let w = i32::try_from(image.width).unwrap_or(0);
        let h = i32::try_from(image.height).unwrap_or(0);
        if w == 0 || h == 0 {
            continue;
        }
        let (x0, y0) = (item.screen_x, item.screen_y);
        let (x1, y1) = (x0.saturating_add(w), y0.saturating_add(h));
        bounds = Some(match bounds {
            None => (x0, y0, x1, y1),
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        });
    }
    bounds
}

#[cfg(test)]
mod tests;
