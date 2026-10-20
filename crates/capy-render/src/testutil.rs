//! Synthetic DT1/DS1 builders for the unit tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use d2_assets::GamePath;
use d2_formats::ds1::{Cell, Ds1};
use d2_formats::dt1::{Block, BlockFormat, Dt1, Tile};
use d2_formats::palette::Palette;

use crate::tileset::TileSet;

/// One isometric 32x15 block filled with palette index `pixel`.
pub fn iso_block(x: i16, y: i16, pixel: u8) -> Block {
    Block {
        x,
        y,
        grid_x: 0,
        grid_y: 0,
        raw_format: 1,
        format: BlockFormat::Isometric,
        data: vec![pixel; 256],
    }
}

/// A tile of the given key with five isometric blocks side by side (a 160x15
/// strip) and the stored `height`/`width`.
pub fn tile(orientation: i32, main: i32, sub: i32, rarity: i32, pixel: u8) -> Tile {
    let (height, width) = match orientation {
        0 | 15 => (-80, 160),
        13 => (-96, 160),
        o if o > 15 => (-32, 160),
        _ => (-128, 160),
    };
    // Floors and roofs start at the bitmap top, upper walls and shadows end at
    // its bottom (negative y), lower walls start 96 above their anchor.
    let y = match orientation {
        0 | 15 => 0,
        o if o > 15 => -96,
        _ => -15,
    };
    let blocks = (0..5).map(|i| iso_block(i * 32, y, pixel)).collect();
    Tile {
        direction: 0,
        roof_height: 0,
        sound: 0,
        animated: 0,
        height,
        width,
        orientation,
        main_index: main,
        sub_index: sub,
        rarity,
        subtile_flags: [0; 25],
        preserved: Default::default(),
        blocks,
    }
}

pub fn dt1(tiles: Vec<Tile>) -> Dt1 {
    Dt1 {
        version: (7, 6),
        tiles,
    }
}

pub fn tileset(dt1s: Vec<Dt1>) -> TileSet {
    let paths = (0..dt1s.len())
        .map(|i| GamePath::new(&format!("data\\global\\tiles\\test{i}.dt1")))
        .collect();
    TileSet::from_dt1s(paths, dt1s)
}

/// A cell with the given `prop1` (draw height), main and sub index.
pub fn cell(prop1: u8, main: u8, sub: u8) -> Cell {
    Cell(u32::from(prop1) | (u32::from(sub) << 8) | (u32::from(main) << 20))
}

pub fn set_floor(ds1: &mut Ds1, layer: usize, x: u32, y: u32, c: Cell) {
    assert!(ds1.floors[layer].set(x, y, c));
}

pub fn set_wall(ds1: &mut Ds1, layer: usize, x: u32, y: u32, orientation: u32, c: Cell) {
    assert!(ds1.walls[layer].tiles.set(x, y, c));
    assert!(ds1.walls[layer].orientations.set(x, y, orientation));
}

pub fn set_shadow(ds1: &mut Ds1, x: u32, y: u32, c: Cell) {
    assert!(ds1.shadow.set(x, y, c));
}

/// Palette whose entry `i` is `(i, i / 2, 255 - i)`.
pub fn palette() -> Palette {
    let mut colors = [[0u8; 3]; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        let i = u8::try_from(i).unwrap();
        *c = [i, i / 2, 255 - i];
    }
    Palette { colors }
}
