#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use capy_edit::{CellValue, LayerRef};
use d2_formats::Grid;
use d2_formats::ds1::{Cell, Ds1};

/// A non-empty cell dword with the given tile indices (`prop1` = 1).
pub fn tile(main: u8, sub: u8) -> u32 {
    (u32::from(main) << 20) | (u32::from(sub) << 8) | 1
}

pub fn cv(main: u8, sub: u8) -> CellValue {
    CellValue {
        cell: tile(main, sub),
        orientation: 0,
    }
}

pub fn wall(main: u8, sub: u8, orientation: u32) -> CellValue {
    CellValue {
        cell: tile(main, sub),
        orientation,
    }
}

/// A blank version-3 map: one wall, one floor, a tag layer.
pub fn blank_v3(w: u32, h: u32) -> Ds1 {
    let mut ds = Ds1::new(w, h, 0);
    ds.version = 3;
    ds.tag = Some(Grid::new(w, h));
    ds
}

pub fn set(ds: &mut Ds1, layer: LayerRef, x: u32, y: u32, v: CellValue) {
    assert!(layer.set_for_tests(ds, x, y, v), "{layer:?} ({x},{y})");
}

/// The map must serialize and parse back to itself.
pub fn assert_roundtrips(ds: &Ds1) {
    let bytes = ds.to_bytes().unwrap();
    let back = Ds1::parse(&bytes).unwrap();
    assert_eq!(&back, ds);
}

pub fn fill_floor(ds: &mut Ds1, index: usize, v: CellValue) {
    let (w, h) = (ds.width, ds.height);
    for y in 0..h {
        for x in 0..w {
            ds.floors[index].set(x, y, Cell(v.cell));
        }
    }
}

trait SetForTests {
    fn set_for_tests(self, ds: &mut Ds1, x: u32, y: u32, v: CellValue) -> bool;
}

impl SetForTests for LayerRef {
    fn set_for_tests(self, ds: &mut Ds1, x: u32, y: u32, v: CellValue) -> bool {
        match self {
            LayerRef::Floor(i) => ds
                .floors
                .get_mut(usize::from(i))
                .is_some_and(|g| g.set(x, y, Cell(v.cell))),
            LayerRef::Wall(i) => ds.walls.get_mut(usize::from(i)).is_some_and(|w| {
                w.tiles.set(x, y, Cell(v.cell)) && w.orientations.set(x, y, v.orientation)
            }),
            LayerRef::Shadow => ds.shadow.set(x, y, Cell(v.cell)),
            LayerRef::Tag => ds.tag.as_mut().is_some_and(|g| g.set(x, y, v.cell)),
        }
    }
}
