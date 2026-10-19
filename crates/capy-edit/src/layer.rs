//! Layer addressing and the value stored in one cell.

use d2_formats::ds1::{Cell, Ds1};

/// A cell layer. Wall indices are 0..4, floor 0..2.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum LayerRef {
    Floor(u8),
    Wall(u8),
    Shadow,
    Tag,
}

/// One cell's full content. For walls `orientation` is the raw orientation
/// dword that travels with the tile; for every other layer it is 0.
/// Tag layer cells store their u32 in `cell`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct CellValue {
    pub cell: u32,
    pub orientation: u32,
}

impl CellValue {
    pub const EMPTY: CellValue = CellValue {
        cell: 0,
        orientation: 0,
    };

    /// `prop1 == 0` (`Cell::is_empty`). This is the rule for the floor, wall
    /// and shadow layers; a tag value has no `prop1`, so use
    /// [`LayerRef::is_empty_value`] when the layer may be [`LayerRef::Tag`]
    /// (there a cell is empty exactly when it is zero).
    pub fn is_empty(self) -> bool {
        Cell(self.cell).is_empty()
    }
}

impl LayerRef {
    pub fn exists_in(self, ds1: &Ds1) -> bool {
        match self {
            LayerRef::Floor(i) => usize::from(i) < ds1.floors.len(),
            LayerRef::Wall(i) => usize::from(i) < ds1.walls.len(),
            LayerRef::Shadow => true,
            LayerRef::Tag => ds1.tag.is_some(),
        }
    }

    /// Floors, walls, shadow, tag (if any); draw order is not implied.
    pub fn all_in(ds1: &Ds1) -> Vec<LayerRef> {
        let mut out: Vec<LayerRef> = (0..ds1.floors.len())
            .filter_map(|i| u8::try_from(i).ok())
            .map(LayerRef::Floor)
            .collect();
        out.extend(
            (0..ds1.walls.len())
                .filter_map(|i| u8::try_from(i).ok())
                .map(LayerRef::Wall),
        );
        out.push(LayerRef::Shadow);
        if ds1.tag.is_some() {
            out.push(LayerRef::Tag);
        }
        out
    }

    /// The cell at `(x, y)`; `None` when the layer or the coordinate does not exist.
    pub fn get(self, ds1: &Ds1, x: u32, y: u32) -> Option<CellValue> {
        match self {
            LayerRef::Floor(i) => {
                let c = ds1.floors.get(usize::from(i))?.get(x, y)?;
                Some(CellValue {
                    cell: c.0,
                    orientation: 0,
                })
            }
            LayerRef::Wall(i) => {
                let wall = ds1.walls.get(usize::from(i))?;
                Some(CellValue {
                    cell: wall.tiles.get(x, y)?.0,
                    orientation: wall.orientations.get(x, y)?,
                })
            }
            LayerRef::Shadow => {
                let c = ds1.shadow.get(x, y)?;
                Some(CellValue {
                    cell: c.0,
                    orientation: 0,
                })
            }
            LayerRef::Tag => {
                let c = ds1.tag.as_ref()?.get(x, y)?;
                Some(CellValue {
                    cell: c,
                    orientation: 0,
                })
            }
        }
    }

    /// Whether `value` counts as an empty cell in this layer: `prop1 == 0` for
    /// floor, wall and shadow cells, `cell == 0` for the tag layer.
    pub fn is_empty_value(self, value: CellValue) -> bool {
        match self {
            LayerRef::Tag => value.cell == 0,
            _ => value.is_empty(),
        }
    }

    /// Clears the parts of `value` this layer cannot store (the orientation of
    /// everything that is not a wall).
    pub(crate) fn normalize(self, value: CellValue) -> CellValue {
        match self {
            LayerRef::Wall(_) => value,
            _ => CellValue {
                cell: value.cell,
                orientation: 0,
            },
        }
    }

    /// Stores `value`; `false` (and nothing changes) when the layer or the
    /// coordinate does not exist.
    pub(crate) fn set(self, ds1: &mut Ds1, x: u32, y: u32, value: CellValue) -> bool {
        match self {
            LayerRef::Floor(i) => ds1
                .floors
                .get_mut(usize::from(i))
                .is_some_and(|g| g.set(x, y, Cell(value.cell))),
            LayerRef::Wall(i) => {
                let Some(wall) = ds1.walls.get_mut(usize::from(i)) else {
                    return false;
                };
                if wall.tiles.get(x, y).is_none() || wall.orientations.get(x, y).is_none() {
                    return false;
                }
                wall.tiles.set(x, y, Cell(value.cell))
                    && wall.orientations.set(x, y, value.orientation)
            }
            LayerRef::Shadow => ds1.shadow.set(x, y, Cell(value.cell)),
            LayerRef::Tag => ds1.tag.as_mut().is_some_and(|g| g.set(x, y, value.cell)),
        }
    }
}
