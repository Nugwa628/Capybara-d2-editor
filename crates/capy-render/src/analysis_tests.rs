//! Tests of the P10 map-analysis modules on synthetic maps. The cases are
//! translated from ds1-studio's vitest files (`walkArea`, `spawnRegions`,
//! `exits`, `pops`, `mapOverlays`).
#![allow(clippy::unwrap_used, clippy::panic)]

use d2_formats::ds1::{Cell, Ds1};

use crate::anim::{GameView, animation_frames, frame_at};
use crate::links::{
    ExitOptions, LevelLight, LevelRow, arrival_problem, exit_problems, level_links, map_warps,
    warp_tiles,
};
use crate::pops::{CellRect, find_pops, pop_problems, pop_targets, tiles_to_hide};
use crate::rooms::{
    SpawnClass, SpawnLevel, WalkClass, partly_no_spawn_rooms, preset_rooms, room_regions,
    spawn_overlay, spawns, walkable_overlay,
};
use crate::testutil::{cell, dt1, set_floor, set_wall, tile, tileset};
use crate::walk::{
    AreaBand, BLOCK_PLAYER_WALK, BLOCK_WALK, Collision, area_colour, blank_fill_flags,
    sub_tile_flags, walkable_area,
};

fn floored(w: u32, h: u32) -> Ds1 {
    let mut d = Ds1::new(w, h, 0);
    for y in 0..h {
        for x in 0..w {
            set_floor(&mut d, 0, x, y, cell(0xc2, 1, 0));
        }
    }
    d
}

fn flat_tiles() -> crate::tileset::TileSet {
    tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1)])])
}

#[test]
fn area_counts_tiles_squared_and_no_floor_blocks() {
    let empty = Ds1::new(10, 10, 0);
    assert_eq!(walkable_area(&empty, &flat_tiles()), 0.0);
    let d = floored(6, 6);
    assert_eq!(walkable_area(&d, &flat_tiles()), 36.0);
}

#[test]
fn subtile_flags_come_from_tiles_hidden_variants_and_cell_bits() {
    let mut blocker = tile(0, 2, 0, 0, 1);
    blocker.subtile_flags[0] = BLOCK_WALK;
    blocker.subtile_flags[7] = BLOCK_PLAYER_WALK;
    let mut variant_a = tile(0, 3, 0, 0, 1);
    variant_a.subtile_flags[24] = BLOCK_WALK;
    let mut variant_b = tile(0, 3, 0, 5, 1);
    variant_b.subtile_flags[23] = BLOCK_WALK;
    let ts = tileset(vec![dt1(vec![
        tile(0, 1, 0, 0, 1),
        blocker,
        variant_a,
        variant_b,
    ])]);
    let mut d = floored(4, 4);
    // Visible blocker: one sub-tile blocks walking, one blocks players only.
    set_floor(&mut d, 0, 1, 1, cell(0xc2, 2, 0));
    // Hidden floor 2 tile: every variant counts.
    d.add_floor_layer().unwrap();
    set_floor(&mut d, 1, 2, 2, Cell(cell(0xc2, 3, 0).0 | 1 << 31));
    // Whole-cell unwalkable bit.
    let c = Cell(cell(0xc2, 1, 0).0 | 1 << 17);
    set_floor(&mut d, 0, 3, 3, c);
    let g = sub_tile_flags(&d, &ts, Collision::Editor);
    assert_eq!(g.flags[g.index(1, 1, 0, 0)], BLOCK_WALK);
    assert_eq!(g.flags[g.index(1, 1, 2, 1)], BLOCK_PLAYER_WALK);
    assert_eq!(g.flags[g.index(2, 2, 4, 4)], BLOCK_WALK);
    assert_eq!(g.flags[g.index(2, 2, 3, 4)], BLOCK_WALK);
    assert!((0..25).all(|k| g.flags[((3 * 4 + 3) * 25 + k) as usize] & BLOCK_WALK != 0));
    // 16 tiles - 1 cell (unwalkable) - 1 sub-tile - 2 hidden sub-tiles.
    let area = walkable_area(&d, &ts);
    assert!((area - (15.0 - 3.0 / 25.0)).abs() < 1e-9, "{area}");
}

#[test]
fn game_mode_leaves_void_open_and_adds_the_blank_tile() {
    let mut blank = tile(0, 30, 0, 0, 1);
    blank.subtile_flags = [BLOCK_WALK; 25];
    let ts = tileset(vec![dt1(vec![tile(0, 1, 0, 0, 1), blank])]);
    let mut d = floored(3, 3);
    set_floor(&mut d, 0, 1, 1, Cell::EMPTY);
    let open = sub_tile_flags(&d, &ts, Collision::Game { blank: None });
    assert_eq!(open.flags[open.index(1, 1, 2, 2)], 0);
    let b = blank_fill_flags(&ts, 1);
    assert_eq!(b, [BLOCK_WALK; 25]);
    let filled = sub_tile_flags(&d, &ts, Collision::Game { blank: Some(b) });
    assert_eq!(filled.flags[filled.index(1, 1, 2, 2)], BLOCK_WALK);
    assert_eq!(filled.flags[filled.index(0, 0, 2, 2)], 0);
    assert_eq!(blank_fill_flags(&flat_tiles(), 1), [0; 25]);
}

#[test]
fn colours_by_size_bands() {
    assert_eq!(area_colour(3000.0).1, AreaBand::Small);
    assert_eq!(area_colour(3300.0), ([60, 200, 90], AreaBand::Green));
    assert_eq!(area_colour(3799.0).1, AreaBand::Green);
    assert_eq!(area_colour(3800.0), ([255, 150, 40], AreaBand::Orange));
    assert_eq!(area_colour(4300.0), ([255, 80, 40], AreaBand::Red));
    assert!(area_colour(4800.0).0[1] < 80);
    assert_eq!(area_colour(9000.0).0, [200, 20, 20]);
}

/// 17x9 map (two rooms plus the last row and column) with a line of left walls
/// down x = 4.
fn room_map() -> Ds1 {
    let mut d = floored(17, 9);
    for y in 0..9 {
        set_wall(&mut d, 0, 4, y, 1, cell(0x81, 2, 0));
    }
    d
}

#[test]
fn rooms_are_8x8_from_the_corner() {
    let r = preset_rooms(17, 9);
    assert_eq!(r.len(), 2);
    assert_eq!((r[1].x0, r[1].y0, r[1].w, r[1].h), (8, 0, 8, 8));
    let r = preset_rooms(18, 10);
    let s: Vec<_> = r.iter().map(|r| (r.x0, r.y0, r.w, r.h)).collect();
    assert_eq!(
        s,
        vec![
            (0, 0, 8, 8),
            (8, 0, 8, 8),
            (16, 0, 1, 8),
            (0, 8, 8, 1),
            (8, 8, 8, 1),
            (16, 8, 1, 1)
        ]
    );
}

#[test]
fn regions_are_bounded_by_first_layer_walls_and_nodes_by_their_seed() {
    let d = room_map();
    let rr = room_regions(&d, preset_rooms(17, 9)[0]);
    let got: Vec<_> = rr
        .regions
        .iter()
        .map(|r| (r.seed, r.cells.len(), r.node))
        .collect();
    assert_eq!(got, vec![((0, 0), 32, false), ((4, 0), 32, false)]);
    assert!(rr.regions.iter().all(|r| spawns(&rr, r)));

    // A wall on another layer bounds nothing.
    let mut moved = room_map();
    moved.add_wall_layer().unwrap();
    for y in 0..9 {
        let c = moved.walls[0].tiles.get(4, y).unwrap();
        moved.walls[1].tiles.set(4, y, c);
        moved.walls[1].orientations.set(4, y, 1);
        moved.walls[0].tiles.set(4, y, Cell::EMPTY);
        moved.walls[0].orientations.set(4, y, 0);
    }
    assert_eq!(
        room_regions(&moved, preset_rooms(17, 9)[0]).regions.len(),
        1
    );

    // Node: hidden floor 1 at the seed, or main index 30.
    let mut n = room_map();
    n.floors[0].set(4, 0, Cell(cell(0xc2, 1, 0).0 | 1 << 31));
    let nodes: Vec<_> = room_regions(&n, preset_rooms(17, 9)[0])
        .regions
        .iter()
        .map(|r| r.node)
        .collect();
    assert_eq!(nodes, vec![false, true]);
    n.floors[0].set(0, 0, cell(0xc2, 30, 0));
    let nodes: Vec<_> = room_regions(&n, preset_rooms(17, 9)[0])
        .regions
        .iter()
        .map(|r| r.node)
        .collect();
    assert_eq!(nodes, vec![true, true]);
}

#[test]
fn a_room_with_a_warp_spawns_nothing() {
    let mut d = room_map();
    set_wall(&mut d, 0, 2, 2, 10, cell(0x81, 0, 0));
    let rr = room_regions(&d, preset_rooms(17, 9)[0]);
    assert!(rr.warp);
    assert!(!rr.regions.iter().any(|r| spawns(&rr, r)));
}

#[test]
fn partly_no_spawn_rooms_flags_marked_rooms_with_open_regions() {
    let mut d = room_map();
    d.floors[0].set(4, 0, Cell(cell(0xc2, 1, 0).0 | 1 << 31));
    let rooms = partly_no_spawn_rooms(&d);
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].1.len(), 1);
}

#[test]
fn overviews_classify_sub_tiles() {
    let d = room_map();
    let ts = flat_tiles();
    let mut g = sub_tile_flags(&d, &ts, Collision::Game { blank: None });
    let i = g.index(1, 1, 0, 0);
    g.flags[i] = BLOCK_WALK;
    let j = g.index(2, 2, 0, 0);
    g.flags[j] = BLOCK_PLAYER_WALK;
    let w = walkable_overlay(&d, &g);
    assert_eq!(w[i], WalkClass::Blocked);
    assert_eq!(w[j], WalkClass::MonstersOnly);
    assert_eq!(w[g.index(0, 0, 0, 0)], WalkClass::Walk);
    let s = spawn_overlay(&d, &g, &SpawnLevel::default());
    assert_eq!(s[i], SpawnClass::Blocked);
    assert_eq!(s[g.index(0, 0, 2, 2)], SpawnClass::Spawn);
    // The last row/column belongs to no room.
    assert_eq!(s[g.index(16, 8, 2, 2)], SpawnClass::Edge);
    let off = spawn_overlay(
        &d,
        &g,
        &SpawnLevel {
            populate: Some(false),
            ..SpawnLevel::default()
        },
    );
    assert_eq!(off[g.index(0, 0, 2, 2)], SpawnClass::Off);
}

fn warp_map(warps: &[(u32, u32, u8)]) -> Ds1 {
    let mut d = Ds1::new(8, 8, 4);
    d.add_wall_layer().unwrap();
    for &(x, y, m) in warps {
        set_wall(&mut d, 1, x, y, 10, cell(1, m, 0));
    }
    // A "Map entry" marker (30/11) is not a warp.
    set_wall(&mut d, 1, 0, 0, 10, cell(1, 30, 11));
    d
}

fn level_name(id: i32) -> String {
    if id == 109 {
        "Harrogath".into()
    } else {
        format!("Level {id}")
    }
}

#[test]
fn warps_and_exits() {
    let d = warp_map(&[(2, 3, 1), (3, 3, 1)]);
    assert_eq!(
        warp_tiles(&d).into_iter().collect::<Vec<_>>(),
        vec![(1, vec![(2, 3), (3, 3)])]
    );

    let name: &dyn Fn(i32) -> String = &level_name;
    let o = ExitOptions {
        has_waypoint: false,
        only_preset: true,
        is_town: false,
        level_name: name,
    };
    let none = LevelRow::default();
    let p = exit_problems(&warp_map(&[]), &none, &o);
    assert_eq!(p[0].title, "No way out of this level except a town portal");
    assert_eq!(
        p[0].fix.as_ref().map(|f| (f.vis, f.edit, f.place)),
        Some((0, true, true))
    );

    let p = exit_problems(&warp_map(&[(4, 4, 2)]), &none, &o);
    assert_eq!(p[0].title, "A warp tile for link 2 leads nowhere");
    assert_eq!(p[0].cells, vec![(4, 4)]);
    assert!(p[1].fix.is_none());

    let mut row = LevelRow::default();
    row.vis[3] = 109;
    let p = exit_problems(&warp_map(&[]), &row, &o);
    assert_eq!(
        p[0].fix.as_ref().unwrap().label,
        "Place the warp tile for link 3 (to Harrogath)"
    );

    let mut row = LevelRow::default();
    row.vis[0] = 109;
    assert!(exit_problems(&warp_map(&[(4, 4, 0)]), &row, &o).is_empty());
    let wp = ExitOptions {
        has_waypoint: true,
        ..ExitOptions {
            level_name: name,
            ..o
        }
    };
    assert!(exit_problems(&warp_map(&[]), &none, &wp).is_empty());
}

#[test]
fn warp_targets_resolve_through_the_callers_tables() {
    let mut row = LevelRow {
        id: 5,
        ..LevelRow::default()
    };
    row.vis[1] = 109;
    row.warp[1] = 2;
    row.warp[0] = -1;
    let links = level_links(
        &row,
        &|id| crate::links::LevelRef {
            id,
            name: level_name(id),
            maps: vec![],
        },
        &|w| (w == 2).then(|| "Stairs".to_owned()),
    );
    assert!(links[0].is_none());
    let w = map_warps(&warp_map(&[(2, 2, 1), (3, 3, 4)]), &links);
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].to.as_ref().unwrap().target.name, "Harrogath");
    assert_eq!(w[0].to.as_ref().unwrap().warp, Some((2, "Stairs".into())));
    assert!(w[1].to.is_none());
}

#[test]
fn arrival_needs_floor_in_the_centre_room() {
    let wp: &dyn Fn(i32, i32) -> bool = &|_, _| false;
    assert!(arrival_problem(&floored(20, 20), wp).is_none());
    // Floor only in a corner of a big map: nothing at the centre.
    let mut d = Ds1::new(60, 60, 0);
    for y in 0..6 {
        for x in 0..6 {
            set_floor(&mut d, 0, x, y, cell(0xc2, 1, 0));
        }
    }
    let p = arrival_problem(&d, wp).unwrap();
    assert_eq!(p.room, (24, 24));
    assert_eq!(p.cropped, Some((8, 8)));
    // A warp tile anywhere makes arrivals fine.
    set_wall(&mut d, 0, 1, 1, 10, cell(1, 0, 0));
    assert!(arrival_problem(&d, wp).is_none());
}

#[test]
fn light_multiplier() {
    let row = LevelRow {
        intensity: 255,
        rgb: [255, 0, 255],
        ..LevelRow::default()
    };
    assert_eq!(LevelLight::of(&row).multiplier(), Some([1.0, 0.0, 1.0]));
    assert_eq!(
        LevelLight {
            intensity: 0,
            rgb: [9, 9, 9]
        }
        .multiplier(),
        None
    );
}

#[test]
fn pops_pair_markers_and_hide_the_target_tiles() {
    let mut d = Ds1::new(20, 20, 0);
    d.add_wall_layer().unwrap();
    set_wall(&mut d, 1, 3, 3, 10, cell(1, 8, 46));
    set_wall(&mut d, 1, 6, 6, 10, cell(1, 8, 46));
    for x in 2..8 {
        set_wall(&mut d, 0, x, 2, 15, cell(1, 46, 0));
    }
    set_wall(&mut d, 0, 15, 15, 15, cell(1, 46, 0));
    let areas = find_pops(&d);
    assert_eq!(areas.len(), 1);
    let a = &areas[0];
    assert_eq!((a.main, a.target, a.group), (8, 46, 1));
    assert_eq!(
        a.rect,
        CellRect {
            x0: 3,
            y0: 3,
            x1: 6,
            y1: 6
        }
    );
    assert_eq!(pop_targets(&d, a).len(), 6);
    let all = tiles_to_hide(
        &d,
        CellRect {
            x0: 0,
            y0: 0,
            x1: 19,
            y1: 19,
        },
        46,
    );
    assert_eq!(all.len(), 7);
    assert!(pop_problems(&d, &areas, Some(1), 0).is_empty());
    assert_eq!(pop_problems(&d, &areas, Some(0), 0).len(), 1);
}

#[test]
fn animations_need_distinct_rarities_from_zero() {
    let mut frames = Vec::new();
    for r in [2, 0, 1] {
        let mut t = tile(0, 5, 0, r, r as u8);
        t.animated = 1;
        frames.push(t);
    }
    let mut snow = Vec::new();
    for _ in 0..2 {
        let mut t = tile(0, 6, 0, 3, 1);
        t.animated = 1;
        snow.push(t);
    }
    frames.extend(snow);
    let ts = tileset(vec![dt1(frames)]);
    let key = d2_formats::dt1::TileKey {
        orientation: 0,
        main: 5,
        sub: 0,
    };
    let f = animation_frames(&ts, key);
    let rarities: Vec<_> = f.iter().map(|r| ts.tile(r).unwrap().rarity).collect();
    assert_eq!(rarities, vec![0, 1, 2]);
    assert!(animation_frames(&ts, d2_formats::dt1::TileKey { main: 6, ..key }).is_empty());
    assert_eq!(frame_at(3, 0.0), 0);
    assert_eq!(frame_at(3, 0.35), 0);
    assert_eq!(frame_at(3, 0.45), 1);
    assert_eq!(frame_at(3, 0.35 + 0.1 * 3.0), 0);
}

#[test]
fn game_view_defaults_and_limits() {
    let v = GameView::default();
    assert_eq!((v.width, v.height), (800, 600));
    assert!(GameView::new(100, 600).is_none());
    assert_eq!(v.world_rect(0.0, 0.0), (-400.0, -300.0, 800.0, 600.0));
    assert_eq!(v.fit_zoom(4000.0, 4000.0, 1.0), 1.0);
    assert!(v.fit_zoom(400.0, 4000.0, 1.0) < 0.5);
}
