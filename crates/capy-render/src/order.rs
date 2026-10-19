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
//! 2. object shadows;
//! 3. objects with order flag 1;
//! 4. per cell: upper walls, then the cell's objects with order flag 0/2;
//! 5. roofs;
//! 6. special tiles (orientation 10 and 11), only when shown.
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

use crate::error::Result;
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
    for y in 0..ds1.height {
        for x in 0..ds1.width {
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
/// tile. See the module docs for the order.
pub fn draw_list(
    ds1: &Ds1,
    tiles: &TileSet,
    vis: &Visibility,
) -> Result<(Vec<DrawItem>, Vec<MissingTile>)> {
    let mut missing = Vec::new();
    let entries = collect(ds1, tiles, vis, &mut missing);
    let mut items: Vec<DrawItem> = Vec::with_capacity(entries.len());

    let mut push = |pass: DrawPass, e: &Entry, tile: &TileRef| -> Result<()> {
        let Some(geom) = tiles.geometry(tile) else {
            return Ok(());
        };
        if geom.is_empty() {
            return Ok(()); // no bitmap: nothing to draw (1186)
        }
        let image = tiles.image(tile)?;
        let x = i32::try_from(e.x).unwrap_or(i32::MAX);
        let y = i32::try_from(e.y).unwrap_or(i32::MAX);
        let mx = (x - y).saturating_mul(TILE_W / 2);
        let my = (x + y).saturating_mul(TILE_H / 2);
        items.push(DrawItem {
            pass,
            order: u32::try_from(items.len()).unwrap_or(u32::MAX),
            layer: e.layer,
            x: e.x,
            y: e.y,
            tile: tile.clone(),
            screen_x: mx + image.offset_x,
            screen_y: my + geom.bitmap_top(1) + image.offset_y,
        });
        Ok(())
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
                push(pass, e, &e.tile)?;
            }
        }
    }
    // Passes 2 and 3 (object shadows, objects behind walls): objects are not
    // drawn yet.
    // Pass 4: upper walls, each followed by the orientation-3 corner partner
    // (1131-1160); objects of the cell would follow here.
    for g in &groups {
        for e in g.iter().filter(|e| e.kind == Kind::Upper) {
            push(DrawPass::UpperWalls, e, &e.tile)?;
            if let Some(partner) = tiles.corner_pair(&e.tile) {
                push(DrawPass::UpperWalls, e, &partner)?;
            }
        }
    }
    // Pass 5: roofs.
    for g in &groups {
        for e in g.iter().filter(|e| e.kind == Kind::Roof) {
            push(DrawPass::Roofs, e, &e.tile)?;
        }
    }
    // Pass 6: special tiles (only collected when visible).
    for g in &groups {
        for e in g.iter().filter(|e| e.kind == Kind::Special) {
            push(DrawPass::Special, e, &e.tile)?;
        }
    }
    Ok((items, missing))
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
