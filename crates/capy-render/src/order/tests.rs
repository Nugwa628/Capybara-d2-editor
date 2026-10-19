#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;
use crate::testutil::{cell, dt1, set_floor, set_shadow, set_wall, tile, tileset};
use d2_formats::ds1::Cell;

/// Floors (0,1,0), shadow (13,1,0), upper wall (1,1,0), lower wall (16,1,0),
/// roof (15,1,0), special (10,1,0), a corner pair (3,2,0) + (4,2,0) and a
/// fallback-only lower wall (19,5,0).
fn tiles() -> TileSet {
    tileset(vec![dt1(vec![
        tile(0, 1, 0, 0, 10),
        tile(13, 1, 0, 0, 11),
        tile(1, 1, 0, 0, 12),
        tile(16, 1, 0, 0, 13),
        tile(15, 1, 0, 0, 14),
        tile(10, 1, 0, 0, 15),
        tile(3, 2, 0, 0, 16),
        tile(4, 2, 0, 0, 17),
        tile(19, 5, 0, 0, 18),
    ])])
}

fn ds1(width: u32, height: u32) -> Ds1 {
    let mut ds1 = Ds1::new(width, height, 0);
    ds1.add_wall_layer().unwrap();
    ds1.add_floor_layer().unwrap();
    ds1
}

fn list(ds1: &Ds1, vis: &Visibility) -> (Vec<DrawItem>, Vec<MissingTile>) {
    draw_list(ds1, &tiles(), vis).unwrap()
}

fn passes(items: &[DrawItem]) -> Vec<(DrawPass, u32, u32)> {
    items.iter().map(|i| (i.pass, i.x, i.y)).collect()
}

#[test]
fn pass_order_follows_the_reference() {
    let mut d = ds1(2, 1);
    set_floor(&mut d, 0, 0, 0, cell(1, 1, 0));
    set_wall(&mut d, 0, 0, 0, 16, cell(1, 1, 0));
    set_shadow(&mut d, 0, 0, cell(1, 1, 0));
    set_floor(&mut d, 0, 1, 0, cell(1, 1, 0));
    set_wall(&mut d, 0, 1, 0, 1, cell(1, 1, 0));
    set_wall(&mut d, 1, 1, 0, 15, cell(1, 1, 0));
    let (items, missing) = list(&d, &Visibility::default());
    assert!(missing.is_empty());
    assert_eq!(
        passes(&items),
        vec![
            (DrawPass::LowerWalls, 0, 0),
            (DrawPass::Floors, 0, 0),
            (DrawPass::Shadows, 0, 0),
            (DrawPass::Floors, 1, 0),
            (DrawPass::UpperWalls, 1, 0),
            (DrawPass::Roofs, 1, 0),
        ]
    );
    // `order` is the position in the list.
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.order as usize, i);
    }
}

#[test]
fn pass_one_interleaves_lower_walls_floors_and_shadows_per_cell() {
    // preview.c:3688-3712: one loop over the cells draws w, f, s per cell,
    // not three separate sweeps.
    let mut d = ds1(2, 1);
    for x in 0..2 {
        set_wall(&mut d, 0, x, 0, 16, cell(1, 1, 0));
        set_floor(&mut d, 0, x, 0, cell(1, 1, 0));
    }
    let (items, _) = list(&d, &Visibility::default());
    assert_eq!(
        passes(&items),
        vec![
            (DrawPass::LowerWalls, 0, 0),
            (DrawPass::Floors, 0, 0),
            (DrawPass::LowerWalls, 1, 0),
            (DrawPass::Floors, 1, 0),
        ]
    );
}

#[test]
fn cells_are_visited_row_major() {
    let mut d = ds1(2, 2);
    for y in 0..2 {
        for x in 0..2 {
            set_floor(&mut d, 0, x, y, cell(1, 1, 0));
        }
    }
    let (items, _) = list(&d, &Visibility::default());
    let cells: Vec<_> = items.iter().map(|i| (i.x, i.y)).collect();
    assert_eq!(cells, vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
}

#[test]
fn layers_of_a_cell_are_sorted_by_prop1_then_index() {
    let mut d = ds1(1, 1);
    set_floor(&mut d, 0, 0, 0, cell(9, 1, 0));
    set_floor(&mut d, 1, 0, 0, cell(3, 1, 0));
    let (items, _) = list(&d, &Visibility::default());
    let layers: Vec<_> = items.iter().map(|i| i.layer).collect();
    assert_eq!(layers, vec![('f', 1), ('f', 0)]);

    // Equal prop1: the lower layer index first.
    let mut d = ds1(1, 1);
    set_floor(&mut d, 0, 0, 0, cell(4, 1, 0));
    set_floor(&mut d, 1, 0, 0, cell(4, 1, 0));
    let (items, _) = list(&d, &Visibility::default());
    let layers: Vec<_> = items.iter().map(|i| i.layer).collect();
    assert_eq!(layers, vec![('f', 0), ('f', 1)]);

    // Walls sort the same way inside a pass.
    let mut d = ds1(1, 1);
    set_wall(&mut d, 0, 0, 0, 1, cell(7, 1, 0));
    set_wall(&mut d, 1, 0, 0, 1, cell(2, 1, 0));
    let (items, _) = list(&d, &Visibility::default());
    let layers: Vec<_> = items.iter().map(|i| i.layer).collect();
    assert_eq!(layers, vec![('w', 1), ('w', 0)]);
}

#[test]
fn empty_cells_are_skipped_without_being_missing() {
    let mut d = ds1(2, 1);
    set_floor(&mut d, 0, 0, 0, Cell::EMPTY);
    set_floor(&mut d, 0, 1, 0, cell(1, 1, 0));
    let (items, missing) = list(&d, &Visibility::default());
    assert_eq!(items.len(), 1);
    assert!(missing.is_empty());
}

#[test]
fn hidden_cells_are_skipped_without_being_missing() {
    let mut d = ds1(1, 1);
    // A hidden cell whose tile does not even exist is still not missing.
    set_floor(&mut d, 0, 0, 0, cell(1, 1, 0).with_hidden(true));
    set_floor(&mut d, 1, 0, 0, cell(1, 63, 0).with_hidden(true));
    set_wall(&mut d, 0, 0, 0, 1, cell(1, 1, 0).with_hidden(true));
    set_wall(&mut d, 1, 0, 0, 15, cell(1, 1, 0).with_hidden(true));
    set_shadow(&mut d, 0, 0, cell(1, 1, 0).with_hidden(true));
    let (items, missing) = list(&d, &Visibility::default());
    assert!(items.is_empty(), "{items:?}");
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn special_tiles_are_hidden_by_default() {
    let mut d = ds1(1, 1);
    set_wall(&mut d, 0, 0, 0, 10, cell(1, 1, 0));
    set_wall(&mut d, 1, 0, 0, 11, cell(1, 9, 9)); // no such tile: not missing while hidden
    set_floor(&mut d, 0, 0, 0, cell(1, 1, 0));
    let (items, missing) = list(&d, &Visibility::default());
    assert_eq!(passes(&items), vec![(DrawPass::Floors, 0, 0)]);
    assert!(missing.is_empty());

    let vis = Visibility {
        special: true,
        ..Visibility::default()
    };
    let (items, missing) = list(&d, &vis);
    assert_eq!(
        passes(&items),
        vec![(DrawPass::Floors, 0, 0), (DrawPass::Special, 0, 0)]
    );
    assert_eq!(missing.len(), 1);
}

#[test]
fn special_tiles_ignore_the_hidden_bit_like_the_reference() {
    // preview.c:1113-1119, 1461-1470: only the wall passes look at bit 31.
    let mut d = ds1(1, 1);
    set_wall(&mut d, 0, 0, 0, 10, cell(1, 1, 0).with_hidden(true));
    let vis = Visibility {
        special: true,
        ..Visibility::default()
    };
    let (items, _) = list(&d, &vis);
    assert_eq!(passes(&items), vec![(DrawPass::Special, 0, 0)]);
}

#[test]
fn missing_tiles_are_reported_with_their_key() {
    let mut d = ds1(3, 1);
    set_floor(&mut d, 0, 0, 0, cell(1, 9, 3));
    set_wall(&mut d, 0, 1, 0, 1, cell(1, 7, 2));
    set_shadow(&mut d, 2, 0, cell(1, 8, 1));
    let (items, missing) = list(&d, &Visibility::default());
    assert!(items.is_empty());
    let key = |orientation, main, sub| TileKey {
        orientation,
        main,
        sub,
    };
    assert_eq!(
        missing,
        vec![
            MissingTile {
                layer: ('f', 0),
                x: 0,
                y: 0,
                key: key(0, 9, 3)
            },
            MissingTile {
                layer: ('w', 0),
                x: 1,
                y: 0,
                key: key(1, 7, 2)
            },
            MissingTile {
                layer: ('s', 0),
                x: 2,
                y: 0,
                key: key(13, 8, 1)
            },
        ]
    );
}

#[test]
fn visibility_toggles_hide_layers_and_their_missing_tiles() {
    let mut d = ds1(1, 1);
    set_floor(&mut d, 0, 0, 0, cell(1, 1, 0));
    set_floor(&mut d, 1, 0, 0, cell(1, 9, 9));
    set_wall(&mut d, 0, 0, 0, 1, cell(1, 1, 0));
    set_wall(&mut d, 1, 0, 0, 15, cell(1, 1, 0));
    set_shadow(&mut d, 0, 0, cell(1, 1, 0));
    let mut vis = Visibility {
        floors: [false, false],
        walls: [false, true, true, true],
        shadow: false,
        ..Visibility::default()
    };
    let (items, missing) = list(&d, &vis);
    assert_eq!(passes(&items), vec![(DrawPass::Roofs, 0, 0)]);
    assert!(missing.is_empty());
    vis.roofs = false;
    let (items, _) = list(&d, &vis);
    assert!(items.is_empty());
}

#[test]
fn orientation_3_draws_its_orientation_4_partner() {
    // preview.c:1131-1160 (`corner_pair_idx`, `misc.c:1056`).
    let mut d = ds1(1, 1);
    set_wall(&mut d, 0, 0, 0, 3, cell(1, 2, 0));
    let (items, missing) = list(&d, &Visibility::default());
    assert!(missing.is_empty());
    assert_eq!(items.len(), 2);
    let ts = tiles();
    let ori = |i: &DrawItem| ts.tile(&i.tile).unwrap().orientation;
    assert_eq!((ori(&items[0]), ori(&items[1])), (3, 4));
    assert!(items.iter().all(|i| i.pass == DrawPass::UpperWalls));
    assert_eq!(items[0].layer, items[1].layer);
}

#[test]
fn lower_walls_use_the_18_19_fallback() {
    let mut d = ds1(1, 1);
    set_wall(&mut d, 0, 0, 0, 18, cell(1, 5, 3));
    let (items, missing) = list(&d, &Visibility::default());
    assert!(missing.is_empty());
    assert_eq!(passes(&items), vec![(DrawPass::LowerWalls, 0, 0)]);
}

#[test]
fn a_wall_cell_that_resolves_to_a_floor_or_shadow_tile_is_not_drawn() {
    // preview.c:1196-1202: only wall-type blocks are drawn by the wall passes.
    let mut d = ds1(1, 1);
    set_wall(&mut d, 0, 0, 0, 0, cell(1, 1, 0)); // key (0,1,0) is a floor
    set_wall(&mut d, 1, 0, 0, 13, cell(1, 1, 0)); // key (13,1,0) is a shadow
    let (items, missing) = list(&d, &Visibility::default());
    assert!(items.is_empty());
    assert!(missing.is_empty());
}

#[test]
fn screen_position_is_cell_anchor_plus_image_offset() {
    let ts = tiles();
    let mut d = ds1(3, 2);
    set_floor(&mut d, 0, 2, 1, cell(1, 1, 0));
    set_wall(&mut d, 0, 2, 1, 1, cell(1, 1, 0));
    let (items, _) = draw_list(&d, &ts, &Visibility::default()).unwrap();
    let floor = &items[0];
    let img = ts.image(&floor.tile).unwrap();
    assert_eq!(floor.screen_x, 80 + img.offset_x);
    assert_eq!(floor.screen_y, (2 + 1) * 40 + img.offset_y);
    // Upper walls hang off the cell's bottom corner: bitmap top = my + 80 - 96.
    let wall = &items[1];
    let wimg = ts.image(&wall.tile).unwrap();
    assert_eq!(wall.screen_x, 80 + wimg.offset_x);
    assert_eq!(wall.screen_y, 120 - 16 + wimg.offset_y);
}

#[test]
fn bounds_cover_all_items_and_are_none_for_an_empty_list() {
    let ts = tiles();
    assert_eq!(map_screen_bounds(&[], &ts), None);
    let mut d = ds1(2, 1);
    set_floor(&mut d, 0, 0, 0, cell(1, 1, 0));
    set_floor(&mut d, 0, 1, 0, cell(1, 1, 0));
    let (items, _) = draw_list(&d, &ts, &Visibility::default()).unwrap();
    let (x0, y0, x1, y1) = map_screen_bounds(&items, &ts).unwrap();
    assert!(x0 <= 0 && x1 >= 80 + 128);
    assert!(y0 <= 0 && y1 > y0);
}

#[test]
fn default_visibility_matches_the_contract() {
    let v = Visibility::default();
    assert!(v.floors.iter().all(|&b| b) && v.walls.iter().all(|&b| b));
    assert!(v.shadow && v.roofs && v.objects);
    assert!(!v.special);
}
