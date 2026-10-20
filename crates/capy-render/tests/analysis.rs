//! P10 map analysis on real vanilla maps: the walkable-area count against an
//! independent by-hand implementation of ds1-studio's algorithm, and the pops
//! cases from ds1-studio's `pops.test.ts`.
//!
//! Skips when the vanilla install is absent. Reads the game through the
//! overlay only.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::type_complexity,
    clippy::needless_range_loop
)]

mod common;

use std::collections::HashMap;

use capy_render::pops::{find_pops, pop_targets, tiles_to_hide};
use capy_render::walk::{Collision, sub_tile_flags, walkable_area};
use capy_render::{TileSet, VariantRule};
use common::*;
use d2_assets::{GamePath, MapCatalog, Profile};
use d2_formats::dt1::{Dt1, Tile};

const MAPS: &[&str] = &[
    "act1\\outdoors\\cott6.ds1",
    "act1\\town\\towne1.ds1",
    "act2\\town\\lutn.ds1",
    "act1\\outdoors\\bord1.ds1",
];

fn gp(m: &str) -> GamePath {
    GamePath::new(&format!("data\\global\\tiles\\{m}"))
}

/// ds1-studio's walkable area computed straight from raw tiles: no TileSet,
/// no `sub_tile_flags`. Variant rule: highest rarity, ties to the first in
/// load order; with every rarity 0 the last tile of the first DT1 holding the
/// key (the documented game rule, same as `TileSet::resolve`).
fn by_hand(ds1: &d2_formats::ds1::Ds1, dt1s: &[Dt1]) -> f64 {
    let mut by_key: HashMap<(i32, i32, i32), Vec<(usize, &Tile)>> = HashMap::new();
    for (di, d) in dt1s.iter().enumerate() {
        for t in &d.tiles {
            by_key
                .entry((t.orientation, t.main_index, t.sub_index))
                .or_default()
                .push((di, t));
        }
    }
    let pick = |o: i32, m: i32, s: i32| -> Option<&Tile> {
        let c = by_key.get(&(o, m, s))?;
        if c.iter().all(|(_, t)| t.rarity == 0) {
            let first = c[0].0;
            return c.iter().rfind(|(d, _)| *d == first).map(|(_, t)| *t);
        }
        let mut best = c[0].1;
        for (_, t) in c {
            if t.rarity > best.rarity {
                best = t;
            }
        }
        Some(best)
    };
    let (w, h) = (ds1.width as usize, ds1.height as usize);
    let mut blocked = vec![[false; 25]; w * h];
    for cy in 0..h {
        for cx in 0..w {
            let b = &mut blocked[cy * w + cx];
            let (x, y) = (cx as u32, cy as u32);
            let mut mark = |t: &Tile| {
                for k in 0..25 {
                    // IDX_TABLE: file byte t -> (4 - t / 5) * 5 + t % 5. The parsed
                    // array is already in that order.
                    if t.subtile_flags[k] & 1 != 0 {
                        b[k] = true;
                    }
                }
            };
            let mut any_floor = false;
            let mut cell_block = false;
            for f in &ds1.floors {
                let c = f.get(x, y).unwrap();
                if c.prop1() == 0 {
                    continue;
                }
                any_floor = true;
                cell_block |= (c.0 >> 17) & 1 == 1;
                if c.hidden() {
                    for (_, t) in by_key
                        .get(&(0, i32::from(c.main_index()), i32::from(c.sub_index())))
                        .into_iter()
                        .flatten()
                    {
                        mark(t);
                    }
                } else if let Some(t) = pick(0, i32::from(c.main_index()), i32::from(c.sub_index()))
                {
                    mark(t);
                }
            }
            for (li, l) in ds1.walls.iter().enumerate() {
                let c = l.tiles.get(x, y).unwrap();
                if c.prop1() == 0 {
                    continue;
                }
                cell_block |= (c.0 >> 17) & 1 == 1;
                let o = i32::from(ds1.orientation(li, x, y).unwrap());
                if o == 10 || o == 11 {
                    continue;
                }
                let (m, s) = (i32::from(c.main_index()), i32::from(c.sub_index()));
                if c.hidden() {
                    for (_, t) in by_key.get(&(o, m, s)).into_iter().flatten() {
                        mark(t);
                    }
                    if o == 3 {
                        for (_, t) in by_key.get(&(4, m, s)).into_iter().flatten() {
                            mark(t);
                        }
                    }
                } else {
                    if let Some(t) = pick(o, m, s) {
                        mark(t);
                    }
                    if o == 3 {
                        // The partner: first orientation-4 tile in (DT1, rarity, block) order.
                        let mut c4: Vec<_> = by_key.get(&(4, m, s)).into_iter().flatten().collect();
                        c4.sort_by_key(|(d, t)| (*d, t.rarity));
                        if let Some((_, t)) = c4.first() {
                            mark(t);
                        }
                    }
                }
            }
            if cell_block || !any_floor {
                *b = [true; 25];
            }
        }
    }
    blocked.iter().flatten().filter(|v| !**v).count() as f64 / 25.0
}

#[test]
fn walkable_area_matches_the_hand_derived_count_on_vanilla_maps() {
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let game = open_profile(Profile::vanilla(&vanilla));
    let names: Vec<GamePath> = MAPS.iter().map(|m| gp(m)).collect();
    let (catalog, _) = MapCatalog::build(&game.overlay, game.version, &names).unwrap();
    for m in MAPS {
        let path = gp(m);
        let ds1 = load_ds1(&game, &path.to_string());
        let info = catalog
            .best(&path)
            .unwrap_or_else(|| panic!("{m}: no map info"));
        let (tiles, _) = TileSet::load(&game.overlay, &info.dt1_files).unwrap();
        let tiles = tiles.with_variant_rule(VariantRule::Documented);
        let dt1s: Vec<Dt1> = info
            .dt1_files
            .iter()
            .map(|p| {
                Dt1::parse(&game.overlay.read_bytes(p).unwrap().unwrap_or_default()).unwrap_or(
                    Dt1 {
                        version: (7, 6),
                        tiles: vec![],
                    },
                )
            })
            .collect();
        let ours = walkable_area(&ds1, &tiles);
        let hand = by_hand(&ds1, &dt1s);
        eprintln!(
            "{m}: {}x{} walkable area {ours} tiles2 (by hand {hand})",
            ds1.width, ds1.height
        );
        assert!((ours - hand).abs() < 1e-9, "{m}: {ours} vs {hand}");
        assert!(
            ours > 0.0 && ours <= f64::from(ds1.width * ds1.height),
            "{m}: {ours}"
        );
        // Game collision (void open) never blocks more than the editor overlay.
        let editor = sub_tile_flags(&ds1, &tiles, Collision::Editor);
        let game_c = sub_tile_flags(&ds1, &tiles, Collision::Game { blank: None });
        assert!(
            editor
                .flags
                .iter()
                .zip(&game_c.flags)
                .all(|(e, g)| e & g == *g)
        );
    }
}

#[test]
fn pops_of_vanilla_maps_read_like_ds1_studio() {
    let Some(vanilla) = vanilla_dir() else {
        eprintln!("skipped: vanilla install not found");
        return;
    };
    let game = open_profile(Profile::vanilla(&vanilla));
    // pops.test.ts: the cottage roof area.
    let ds1 = load_ds1(&game, &gp("act1\\outdoors\\cott6.ds1").to_string());
    let areas = find_pops(&ds1);
    assert_eq!(areas.len(), 1);
    let a = &areas[0];
    assert_eq!((a.main, a.target, a.group), (8, 46, 1));
    assert_eq!((a.rect.x0, a.rect.y0, a.rect.x1, a.rect.y1), (3, 3, 6, 6));
    let all = tiles_to_hide(
        &ds1,
        capy_render::pops::CellRect {
            x0: 0,
            y0: 0,
            x1: ds1.width as i32 - 1,
            y1: ds1.height as i32 - 1,
        },
        46,
    );
    assert_eq!(pop_targets(&ds1, a).len(), all.len());
    assert!(all.len() > 10);
    // Lut Gholein has three areas.
    let lut = load_ds1(&game, &gp("act2\\town\\lutn.ds1").to_string());
    let mut names: Vec<String> = find_pops(&lut)
        .iter()
        .map(|a| format!("{}/{}", a.main, a.target))
        .collect();
    names.sort();
    assert_eq!(names, vec!["12/13", "13/13", "8/8"]);
}
