//! Turning an index tile into a paint brush, and parsing asset keys.
//!
//! A tile asset is keyed `<dt1 path>#<orientation>/<main>/<sub>/<n>`. The
//! brush is the [`CellValue`] a cell of the right layer holds for that tile:
//!
//! * orientation 0 is a floor, 13 a shadow, anything else a wall (the wall
//!   layers also hold the special tiles 10/11 and roofs);
//! * `prop1` (the draw height) and the shadow flag follow the reference
//!   editor's tile window, which writes 194 for a new floor, 128 plus bit 27
//!   for a shadow and 129 for a wall (`edit_window.c:1474-1499`);
//! * walls carry the orientation dword; DS1 files older than v7 store it
//!   through [`DIR_LOOKUP`], so the brush stores the first index that maps
//!   back to the tile's orientation.

use capy_edit::{CellValue, LayerRef};
use d2_formats::ds1::{Cell, DIR_LOOKUP};

/// `prop1` of a new floor cell (`edit_window.c:1475`).
pub const FLOOR_PROP1: u32 = 194;
/// `prop1` of a new shadow cell (`edit_window.c:1486`).
pub const SHADOW_PROP1: u32 = 128;
/// `prop1` of a new wall cell (`edit_window.c:1498`).
pub const WALL_PROP1: u32 = 129;
/// Bit 27 of a shadow cell (`prop4 | 0x08`, `edit_window.c:1489`).
pub const SHADOW_BIT: u32 = 1 << 27;

/// The parts of a tile key after the `#`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileId {
    pub orientation: u8,
    pub main: u8,
    pub sub: u8,
    /// Index among the tiles with the same (orientation, main, sub) in the DT1.
    pub n: u32,
}

/// Splits `<dt1 path>#o/m/s/n`. `None` for file keys and malformed text.
pub fn parse_tile_key(key: &str) -> Option<(&str, TileId)> {
    let (path, rest) = key.rsplit_once('#')?;
    let mut parts = rest.split('/');
    let orientation = parts.next()?.trim().parse().ok()?;
    let main = parts.next()?.trim().parse().ok()?;
    let sub = parts.next()?.trim().parse().ok()?;
    let n = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() || path.is_empty() {
        return None;
    }
    Some((
        path,
        TileId {
            orientation,
            main,
            sub,
            n,
        },
    ))
}

/// The key of tile `id` in `dt1` (lower-case, like the index).
pub fn tile_key(dt1: &str, id: TileId) -> String {
    format!(
        "{}#{}/{}/{}/{}",
        dt1.to_ascii_lowercase(),
        id.orientation,
        id.main,
        id.sub,
        id.n
    )
}

/// Which layer family a tile orientation belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Floor,
    Wall,
    Shadow,
}

pub fn family(orientation: u8) -> Family {
    match orientation {
        0 => Family::Floor,
        13 => Family::Shadow,
        _ => Family::Wall,
    }
}

/// The layer a tile is painted on: the active layer when it is of the
/// tile's family, else the first layer of that family.
pub fn layer_for(orientation: u8, active: LayerRef) -> LayerRef {
    match (family(orientation), active) {
        (Family::Floor, LayerRef::Floor(i)) => LayerRef::Floor(i),
        (Family::Floor, _) => LayerRef::Floor(0),
        (Family::Wall, LayerRef::Wall(i)) => LayerRef::Wall(i),
        (Family::Wall, _) => LayerRef::Wall(0),
        (Family::Shadow, _) => LayerRef::Shadow,
    }
}

/// The orientation dword a wall cell stores for `orientation` in a DS1 of
/// `version`. `None` when an old file has no code for it.
pub fn stored_orientation(orientation: u8, version: i32) -> Option<u32> {
    if version >= 7 {
        return Some(u32::from(orientation));
    }
    DIR_LOOKUP
        .iter()
        .position(|&o| o == orientation)
        .and_then(|i| u32::try_from(i).ok())
}

/// The brush for tile `id` with `active` as the active layer of a DS1 of
/// `version`: the layer to paint and the value.
pub fn brush_for(id: TileId, active: LayerRef, version: i32) -> Result<(LayerRef, CellValue), String> {
    let layer = layer_for(id.orientation, active);
    let value = match family(id.orientation) {
        Family::Floor => CellValue {
            cell: Cell(FLOOR_PROP1).with_indices(id.main, id.sub).0,
            orientation: 0,
        },
        Family::Shadow => CellValue {
            cell: Cell(SHADOW_PROP1 | SHADOW_BIT).with_indices(id.main, id.sub).0,
            orientation: 0,
        },
        Family::Wall => CellValue {
            cell: Cell(WALL_PROP1).with_indices(id.main, id.sub).0,
            orientation: stored_orientation(id.orientation, version).ok_or_else(|| {
                format!(
                    "a v{version} map cannot store wall orientation {} (upgrade the map first)",
                    id.orientation
                )
            })?,
        },
    };
    if id.main > 0x3F {
        return Err(format!("main index {} does not fit a DS1 cell", id.main));
    }
    Ok((layer, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(orientation: u8, main: u8, sub: u8) -> TileId {
        TileId {
            orientation,
            main,
            sub,
            n: 0,
        }
    }

    #[test]
    fn keys_parse_and_print() {
        let key = "data\\global\\tiles\\mxl\\zone\\whimsy\\whimsyfloor.dt1#0/23/0/0";
        let (path, t) = parse_tile_key(key).expect("tile key");
        assert_eq!(path, "data\\global\\tiles\\mxl\\zone\\whimsy\\whimsyfloor.dt1");
        assert_eq!(t, id(0, 23, 0));
        assert_eq!(tile_key(path, t), key);
        assert_eq!(
            parse_tile_key("a.dt1#5/12/3/2").map(|(_, t)| t.n),
            Some(2)
        );
        for bad in [
            "data\\x.ds1",
            "a.dt1#0/1/2",
            "a.dt1#0/1/2/3/4",
            "a.dt1#x/1/2/0",
            "#0/1/2/0",
            "a.dt1#0/300/2/0",
        ] {
            assert_eq!(parse_tile_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn floor_brush_uses_floor_prop1_and_keeps_the_active_floor() {
        let (layer, v) = brush_for(id(0, 23, 0), LayerRef::Floor(1), 18).expect("floor");
        assert_eq!(layer, LayerRef::Floor(1));
        assert_eq!(v.cell, 0x0170_00C2);
        assert_eq!(v.orientation, 0);
        let c = Cell(v.cell);
        assert_eq!((c.main_index(), c.sub_index(), c.prop1()), (23, 0, 194));
        // From a wall layer a floor tile goes to the first floor.
        let (layer, _) = brush_for(id(0, 5, 2), LayerRef::Wall(2), 18).expect("floor");
        assert_eq!(layer, LayerRef::Floor(0));
    }

    #[test]
    fn wall_brush_carries_the_orientation() {
        let (layer, v) = brush_for(id(4, 30, 2), LayerRef::Wall(1), 18).expect("wall");
        assert_eq!(layer, LayerRef::Wall(1));
        assert_eq!(v.orientation, 4);
        let c = Cell(v.cell);
        assert_eq!((c.main_index(), c.sub_index(), c.prop1()), (30, 2, 129));
        assert_eq!(v.cell, 0x01E0_0281);
        // From the floor layer, a wall goes to wall 0.
        let (layer, _) = brush_for(id(1, 1, 0), LayerRef::Floor(0), 18).expect("wall");
        assert_eq!(layer, LayerRef::Wall(0));
        // Old files store the orientation through DIR_LOOKUP.
        let (_, old) = brush_for(id(3, 1, 0), LayerRef::Wall(0), 5).expect("v5");
        assert_eq!(DIR_LOOKUP[old.orientation as usize], 3);
        assert_eq!(old.orientation, 5);
        // 19 has no old code.
        assert!(brush_for(id(19, 1, 0), LayerRef::Wall(0), 5).is_err());
        assert_eq!(
            brush_for(id(19, 1, 0), LayerRef::Wall(0), 7)
                .expect("v7")
                .1
                .orientation,
            19
        );
    }

    #[test]
    fn shadow_brush_sets_the_shadow_bit() {
        let (layer, v) = brush_for(id(13, 2, 7), LayerRef::Floor(0), 18).expect("shadow");
        assert_eq!(layer, LayerRef::Shadow);
        assert_eq!(v.orientation, 0);
        assert_eq!(v.cell, 0x0820_0780);
        let c = Cell(v.cell);
        assert_eq!((c.main_index(), c.sub_index(), c.prop1()), (2, 7, 128));
        assert!(!layer.is_empty_value(v));
    }

    #[test]
    fn mains_beyond_six_bits_are_refused() {
        assert!(brush_for(id(0, 64, 0), LayerRef::Floor(0), 18).is_err());
    }
}
