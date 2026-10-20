//! P9 map-editing operations: behaviour, undo laws (proptest) and the corpus.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use capy_edit::{
    Brush, CellRect, CellValue, Document, EditError, FloorChoice, LayerRef, NewMapSpec,
    ObjectCheckOptions, ObjectProblem, RerollOptions, Rng, Selection, TileKey, check_objects,
    new_map,
};
use common::{assert_roundtrips, cv, tile};
use d2_formats::ds1::{Ds1, Group, Object};
use proptest::prelude::*;

fn doc(w: u32, h: u32) -> Document {
    Document::new(Ds1::new(w, h, 1))
}

fn floor_key(main: u8, sub: u8) -> TileKey {
    TileKey {
        orientation: 0,
        main,
        sub,
    }
}

fn main_sub(d: &Document, layer: LayerRef, x: u32, y: u32) -> Option<(u8, u8)> {
    let v = layer.get(d.ds1(), x, y)?;
    capy_edit::tile_key(layer, v).map(|k| (k.main, k.sub))
}

#[test]
fn paint_rect_and_erase_rect() {
    let mut d = doc(6, 6);
    let f = LayerRef::Floor(0);
    let brush = [Brush::floor(3, 4)];
    d.paint_rect(f, CellRect::new((1, 1), (3, 2)), &brush, &mut Rng::new(1))
        .unwrap();
    assert_eq!(main_sub(&d, f, 1, 1), Some((3, 4)));
    assert_eq!(main_sub(&d, f, 3, 2), Some((3, 4)));
    assert_eq!(main_sub(&d, f, 4, 2), None);
    // clipped to the map
    d.paint_rect(f, CellRect::new((4, 4), (99, 99)), &brush, &mut Rng::new(1))
        .unwrap();
    assert_eq!(main_sub(&d, f, 5, 5), Some((3, 4)));
    d.erase_rect(f, CellRect::new((0, 0), (5, 5))).unwrap();
    assert_eq!(f.get(d.ds1(), 1, 1), Some(CellValue::EMPTY));
    assert_eq!(d.undo_label(), Some("Erase"));
    d.undo();
    assert_eq!(main_sub(&d, f, 1, 1), Some((3, 4)));
}

#[test]
fn painting_the_same_tile_is_a_noop() {
    let mut d = doc(3, 3);
    let f = LayerRef::Floor(0);
    let r = CellRect::new((0, 0), (2, 2));
    d.paint_rect(f, r, &[Brush::floor(1, 1)], &mut Rng::new(0))
        .unwrap();
    let rev = d.revision();
    d.paint_rect(f, r, &[Brush::floor(1, 1)], &mut Rng::new(0))
        .unwrap();
    assert_eq!(d.revision(), rev);
}

#[test]
fn wall_paint_sets_orientation_and_tag_layer_is_refused() {
    let mut d = Document::new(common::blank_v3(3, 3));
    let b = Brush {
        main: 2,
        sub: 5,
        orientation: 4,
    };
    d.paint_cells_for_test(LayerRef::Wall(0), b);
    assert_eq!(LayerRef::Wall(0).get(d.ds1(), 0, 0).unwrap().orientation, 4);
    let err = d.paint_rect(
        LayerRef::Tag,
        CellRect::new((0, 0), (1, 1)),
        &[b],
        &mut Rng::new(0),
    );
    assert!(err.is_err());
    let err = d.paint_rect(
        LayerRef::Floor(1),
        CellRect::new((0, 0), (1, 1)),
        &[b],
        &mut Rng::new(0),
    );
    assert!(matches!(err, Err(EditError::NoSuchLayer(_))));
}

trait PaintAll {
    fn paint_cells_for_test(&mut self, layer: LayerRef, b: Brush);
}
impl PaintAll for Document {
    fn paint_cells_for_test(&mut self, layer: LayerRef, b: Brush) {
        let r = CellRect::new((0, 0), (2, 2));
        self.paint_rect(layer, r, &[b], &mut Rng::new(0)).unwrap();
    }
}

#[test]
fn random_mix_is_seeded_and_uses_every_tile() {
    let mix = [Brush::floor(1, 0), Brush::floor(1, 1), Brush::floor(1, 2)];
    let run = |seed| {
        let mut d = doc(10, 10);
        d.paint_rect(
            LayerRef::Floor(0),
            CellRect::new((0, 0), (9, 9)),
            &mix,
            &mut Rng::new(seed),
        )
        .unwrap();
        d.into_ds1()
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7), run(8));
    let d = Document::new(run(7));
    let mut seen = std::collections::BTreeSet::new();
    for y in 0..10 {
        for x in 0..10 {
            seen.insert(main_sub(&d, LayerRef::Floor(0), x, y).unwrap());
        }
    }
    assert_eq!(seen.len(), 3);
}

#[test]
fn flood_region_is_four_connected_same_tile() {
    // A diagonal neighbour of the same tile is not connected.
    let mut d = doc(4, 4);
    let f = LayerRef::Floor(0);
    for (x, y) in [(0, 0), (1, 0), (2, 1)] {
        d.paint_mix(f, &[(x, y)], &[Brush::floor(1, 1)], &mut Rng::new(0))
            .unwrap();
    }
    assert_eq!(d.flood_region(f, 0, 0, None), vec![(0, 0), (1, 0)]);
    // the empty region is everything else, connected around the tiles
    assert_eq!(d.flood_region(f, 3, 3, None).len(), 13);
    // out of the map
    assert!(d.flood_region(f, 9, 9, None).is_empty());
}

#[test]
fn flood_fill_erase_and_selection_limit() {
    let mut d = doc(5, 1);
    let f = LayerRef::Floor(0);
    d.paint_rect(
        f,
        CellRect::new((0, 0), (4, 0)),
        &[Brush::floor(2, 2)],
        &mut Rng::new(0),
    )
    .unwrap();
    let mut sel = Selection::new();
    for x in 1..=3 {
        sel.insert(f, x, 0);
    }
    d.flood_fill(f, 2, 0, &[Brush::floor(9, 9)], Some(&sel), &mut Rng::new(0))
        .unwrap();
    assert_eq!(main_sub(&d, f, 0, 0), Some((2, 2)));
    assert_eq!(main_sub(&d, f, 2, 0), Some((9, 9)));
    assert_eq!(main_sub(&d, f, 4, 0), Some((2, 2)));
    // outside the selection nothing happens
    let rev = d.revision();
    d.flood_fill(f, 0, 0, &[Brush::floor(1, 1)], Some(&sel), &mut Rng::new(0))
        .unwrap();
    assert_eq!(d.revision(), rev);
    // flood erase
    d.flood_fill(f, 2, 0, &[], None, &mut Rng::new(0)).unwrap();
    assert_eq!(main_sub(&d, f, 2, 0), None);
    assert_eq!(main_sub(&d, f, 3, 0), None);
    assert_eq!(main_sub(&d, f, 4, 0), Some((2, 2)));
    assert_eq!(d.undo_label(), Some("Flood erase"));
}

#[test]
fn replace_tiles_scope_and_layers() {
    let mut d = doc(4, 1);
    let f = LayerRef::Floor(0);
    d.paint_rect(
        f,
        CellRect::new((0, 0), (3, 0)),
        &[Brush::floor(1, 1)],
        &mut Rng::new(0),
    )
    .unwrap();
    d.paint_mix(
        LayerRef::Shadow,
        &[(0, 0)],
        &[Brush::floor(1, 1)],
        &mut Rng::new(0),
    )
    .unwrap();
    let mut sel = Selection::new();
    sel.insert(f, 0, 0);
    sel.insert(f, 1, 0);
    // selection scope, floor only (the shadow key differs anyway)
    let n = d
        .replace_tiles(&[f], floor_key(1, 1), floor_key(5, 6), Some(&sel))
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(main_sub(&d, f, 1, 0), Some((5, 6)));
    assert_eq!(main_sub(&d, f, 2, 0), Some((1, 1)));
    // whole map
    let n = d
        .replace_tiles(&[f], floor_key(1, 1), floor_key(5, 6), None)
        .unwrap();
    assert_eq!(n, 2);
    // nothing left to find
    let n = d
        .replace_tiles(&[f], floor_key(1, 1), floor_key(7, 7), None)
        .unwrap();
    assert_eq!(n, 0);
    // replace keeps other bits (hidden)
    let hidden = 1u32 << 31;
    let mut ds = d.into_ds1();
    ds.floors[0].set(0, 0, d2_formats::ds1::Cell(tile(1, 1) | hidden));
    let mut d = Document::new(ds);
    d.replace_tiles(&[f], floor_key(1, 1), floor_key(2, 2), None)
        .unwrap();
    assert!(d2_formats::ds1::Cell(f.get(d.ds1(), 0, 0).unwrap().cell).hidden());
}

#[test]
fn reroll_is_deterministic_balanced_and_respects_rarity() {
    let choices = [
        FloorChoice {
            main: 1,
            sub: 0,
            rarity: 1,
            walk: 0,
        },
        FloorChoice {
            main: 1,
            sub: 1,
            rarity: 1,
            walk: 0,
        },
        FloorChoice {
            main: 1,
            sub: 2,
            rarity: 0,
            walk: 0,
        },
    ];
    let make = || {
        let mut d = doc(8, 8);
        d.paint_rect(
            LayerRef::Floor(0),
            CellRect::new((0, 0), (7, 7)),
            &[Brush::floor(1, 0)],
            &mut Rng::new(0),
        )
        .unwrap();
        d
    };
    let opts = RerollOptions {
        seed: 42,
        ..RerollOptions::default()
    };
    let mut a = make();
    let rep = a
        .reroll_floors(0, None, &choices, &opts, &|_, _| None)
        .unwrap();
    let mut b = make();
    b.reroll_floors(0, None, &choices, &opts, &|_, _| None)
        .unwrap();
    assert_eq!(a.ds1(), b.ds1());
    assert_eq!(rep.skipped, 0);
    let mut counts = BTreeMap::new();
    for y in 0..8 {
        for x in 0..8 {
            *counts
                .entry(main_sub(&a, LayerRef::Floor(0), x, y).unwrap())
                .or_insert(0) += 1;
        }
    }
    assert!(!counts.contains_key(&(1, 2)), "rarity 0 never chosen");
    assert_eq!(counts[&(1, 0)], 32);
    assert_eq!(counts[&(1, 1)], 32);
    a.undo();
    assert_eq!(a.ds1(), make().ds1());
}

#[test]
fn reroll_preserve_walkability_skips_unknown_and_mismatches() {
    let choices = [
        FloorChoice {
            main: 2,
            sub: 0,
            rarity: 1,
            walk: 1,
        },
        FloorChoice {
            main: 2,
            sub: 1,
            rarity: 1,
            walk: 2,
        },
    ];
    let mut d = doc(2, 1);
    d.paint_mix(
        LayerRef::Floor(0),
        &[(0, 0)],
        &[Brush::floor(9, 0)],
        &mut Rng::new(0),
    )
    .unwrap();
    d.paint_mix(
        LayerRef::Floor(0),
        &[(1, 0)],
        &[Brush::floor(9, 1)],
        &mut Rng::new(0),
    )
    .unwrap();
    let opts = RerollOptions {
        preserve_walkability: true,
        ..RerollOptions::default()
    };
    let rep = d
        .reroll_floors(0, None, &choices, &opts, &|m, s| {
            (m == 9 && s == 0).then_some(2)
        })
        .unwrap();
    assert_eq!(rep.skipped, 1);
    assert_eq!(main_sub(&d, LayerRef::Floor(0), 0, 0), Some((2, 1)));
    assert_eq!(main_sub(&d, LayerRef::Floor(0), 1, 0), Some((9, 1)));
}

#[test]
fn reassign_moves_floors_walls_and_shadows() {
    let mut d = Document::new(common::blank_v3(3, 1));
    let mut ds = d.ds1().clone();
    ds.floors[0].set(0, 0, d2_formats::ds1::Cell(tile(1, 1)));
    ds.shadow.set(0, 0, d2_formats::ds1::Cell(tile(1, 1)));
    ds.walls[0]
        .tiles
        .set(0, 0, d2_formats::ds1::Cell(tile(1, 1)));
    ds.walls[0].orientations.set(0, 0, 1);
    d = Document::new(ds);
    let mut moves = BTreeMap::new();
    moves.insert(floor_key(1, 1), floor_key(3, 3));
    moves.insert(
        TileKey {
            orientation: 13,
            main: 1,
            sub: 1,
        },
        TileKey {
            orientation: 13,
            main: 4,
            sub: 4,
        },
    );
    moves.insert(
        TileKey {
            orientation: 1,
            main: 1,
            sub: 1,
        },
        TileKey {
            orientation: 2,
            main: 5,
            sub: 5,
        },
    );
    assert_eq!(d.reassign_tiles(&moves).unwrap(), 3);
    assert_eq!(main_sub(&d, LayerRef::Floor(0), 0, 0), Some((3, 3)));
    assert_eq!(main_sub(&d, LayerRef::Shadow, 0, 0), Some((4, 4)));
    assert_eq!(main_sub(&d, LayerRef::Wall(0), 0, 0), Some((5, 5)));
    assert_eq!(LayerRef::Wall(0).get(d.ds1(), 0, 0).unwrap().orientation, 2);
    assert_eq!(d.reassign_tiles(&BTreeMap::new()).unwrap(), 0);
}

#[test]
fn new_map_defaults_and_limits() {
    let ds = new_map(&NewMapSpec::new(9, 7, 2)).unwrap();
    assert_eq!((ds.version, ds.width, ds.height, ds.act), (18, 9, 7, 2));
    assert_eq!(
        (ds.walls.len(), ds.floors.len(), ds.tag.is_some()),
        (1, 1, false)
    );
    assert_roundtrips(&ds);

    let spec = NewMapSpec {
        walls: 4,
        floors: 2,
        tag: true,
        ..NewMapSpec::new(4, 4, 0)
    };
    let ds = new_map(&spec).unwrap();
    assert_eq!((ds.walls.len(), ds.floors.len()), (4, 2));
    assert_roundtrips(&ds);

    let old = NewMapSpec {
        version: Some(3),
        tag: true,
        ..NewMapSpec::new(4, 4, 0)
    };
    assert_roundtrips(&new_map(&old).unwrap());
    let too_old = NewMapSpec {
        version: Some(3),
        walls: 2,
        ..NewMapSpec::new(4, 4, 0)
    };
    assert!(matches!(
        new_map(&too_old),
        Err(EditError::NeedsVersion { required, .. }) if required >= 4
    ));
    assert!(new_map(&NewMapSpec::new(0, 4, 0)).is_err());
    assert!(new_map(&NewMapSpec::new(2000, 2000, 0)).is_err());
    assert!(
        new_map(&NewMapSpec {
            walls: 5,
            ..NewMapSpec::new(2, 2, 0)
        })
        .is_err()
    );
}

fn grouped() -> Document {
    let spec = NewMapSpec {
        tag: true,
        ..NewMapSpec::new(10, 10, 1)
    };
    Document::new(new_map(&spec).unwrap())
}

fn group(x: i32, y: i32, w: i32, h: i32) -> Group {
    Group {
        x,
        y,
        width: w,
        height: h,
        unknown: 0,
    }
}

#[test]
fn group_add_move_resize_set_delete_and_undo() {
    let mut d = grouped();
    let start = d.ds1().clone();
    assert_eq!(d.add_group(group(1, 1, 2, 2)).unwrap(), 0);
    assert_eq!(d.add_group(group(5, 5, 3, 3)).unwrap(), 1);
    d.move_group(0, 4, 0).unwrap();
    d.resize_group(0, 6, 1).unwrap();
    d.set_group(
        1,
        Group {
            unknown: 7,
            ..group(0, 0, 1, 1)
        },
    )
    .unwrap();
    assert_eq!(d.ds1().groups[0], group(4, 0, 6, 1));
    assert_eq!(d.ds1().groups[1].unknown, 7);
    assert_roundtrips(d.ds1());
    d.delete_group(0).unwrap();
    assert_eq!(d.ds1().groups.len(), 1);
    assert_roundtrips(d.ds1());
    while d.undo() {}
    assert_eq!(d.ds1(), &start);
    // bad requests change nothing
    assert!(d.delete_group(0).is_err());
    assert!(d.add_group(group(0, 0, 0, 1)).is_err());
    assert!(d.add_group(group(-1, 0, 1, 1)).is_err());
    assert!(!d.can_undo() || d.ds1() == &start);
}

#[test]
fn groups_need_version_and_tag_layer() {
    let mut no_tag = doc(4, 4);
    assert!(no_tag.add_group(group(0, 0, 1, 1)).is_err());
    let mut old = Document::new(
        new_map(&NewMapSpec {
            version: Some(10),
            tag: true,
            ..NewMapSpec::new(4, 4, 0)
        })
        .unwrap(),
    );
    assert!(matches!(
        old.add_group(group(0, 0, 1, 1)),
        Err(EditError::NeedsVersion { required: 12, .. })
    ));
    let mut v12 = Document::new(
        new_map(&NewMapSpec {
            version: Some(12),
            tag: true,
            ..NewMapSpec::new(4, 4, 0)
        })
        .unwrap(),
    );
    v12.add_group(group(0, 0, 1, 1)).unwrap();
    assert!(matches!(
        v12.set_group(
            0,
            Group {
                unknown: 1,
                ..group(0, 0, 1, 1)
            }
        ),
        Err(EditError::NeedsVersion { required: 13, .. })
    ));
    assert_roundtrips(v12.ds1());
}

#[test]
fn resize_edges_shifts_content_and_undoes() {
    let mut d = grouped();
    let f = LayerRef::Floor(0);
    d.paint_mix(f, &[(2, 3)], &[Brush::floor(1, 1)], &mut Rng::new(0))
        .unwrap();
    d.add_group(group(1, 1, 4, 4)).unwrap();
    let mut ds = d.ds1().clone();
    ds.objects.push(Object {
        kind: 1,
        id: 5,
        x: 12,
        y: 17,
        flags: 0,
    });
    ds.objects.push(Object {
        kind: 1,
        id: 6,
        x: 1,
        y: 1,
        flags: 0,
    });
    let mut d = Document::new(ds);
    let before = d.ds1().clone();
    d.resize_edges(2, 1, -1, 3).unwrap();
    assert_eq!((d.ds1().width, d.ds1().height), (11, 14));
    assert_eq!(main_sub(&d, f, 4, 4), Some((1, 1)));
    assert_eq!(d.ds1().objects.len(), 2);
    assert_eq!((d.ds1().objects[0].x, d.ds1().objects[0].y), (22, 22));
    assert_eq!(d.ds1().groups[0], group(3, 2, 4, 4));
    assert_roundtrips(d.ds1());
    // shrinking from the left drops what falls off
    d.resize_edges(-3, 0, 0, 0).unwrap();
    assert_eq!(d.ds1().groups[0], group(0, 2, 4, 4));
    assert!(d.ds1().objects.len() < 2);
    while d.undo() {}
    assert_eq!(d.ds1(), &before);
    assert!(d.resize_edges(-6, 0, -5, 0).is_err());
}

#[test]
fn object_checks() {
    let mut ds = Ds1::new(4, 4, 0);
    let obj = |kind, id, x, y| Object {
        kind,
        id,
        x,
        y,
        flags: 0,
    };
    ds.objects = vec![
        obj(2, 10, 3, 3),
        obj(2, 10, 3, 3),   // duplicate of 0
        obj(2, 11, 20, 3),  // x == width * 5: outside
        obj(2, 12, -1, 0),  // outside
        obj(1, 999, 0, 0),  // unknown
        obj(2, 30, 0, 0),   // pad, alone
        obj(2, 30, 100, 0), // pad, partnered below
    ];
    let known = |o: &Object| o.id < 100;
    let pad = |o: &Object| (o.id == 30).then_some(7);
    let none = check_objects(&ds, &ObjectCheckOptions::default());
    assert_eq!(
        none.iter()
            .map(|f| (f.index, f.problem))
            .collect::<Vec<_>>(),
        vec![
            (1, ObjectProblem::Duplicate { of: 0 }),
            (2, ObjectProblem::OutsideMap),
            (3, ObjectProblem::OutsideMap),
            (6, ObjectProblem::OutsideMap),
        ]
    );
    ds.objects.push(obj(2, 30, 90, 0)); // outside too but near 6
    let opts = ObjectCheckOptions {
        is_known: Some(&known),
        teleport_pad_row: Some(&pad),
    };
    let all = check_objects(&ds, &opts);
    assert!(
        all.iter()
            .any(|f| f.index == 4 && f.problem == ObjectProblem::UnknownId)
    );
    let unpaired: Vec<_> = all
        .iter()
        .filter(|f| f.problem == ObjectProblem::UnpairedPad)
        .map(|f| f.index)
        .collect();
    assert_eq!(unpaired, vec![5]);
}

// ----- undo laws ---------------------------------------------------------------

fn random_doc(seed: u64) -> Document {
    let mut rng = Rng::new(seed);
    let spec = NewMapSpec {
        walls: 2,
        floors: 2,
        tag: true,
        ..NewMapSpec::new(1 + rng.below(9) as u32, 1 + rng.below(9) as u32, 0)
    };
    let mut ds = new_map(&spec).unwrap();
    for layer in LayerRef::all_in(&ds) {
        if layer == LayerRef::Tag {
            continue;
        }
        for y in 0..ds.height {
            for x in 0..ds.width {
                if rng.below(3) > 0 {
                    let v = painted_value(layer, rng.below(3) as u8 + 1, rng.below(3) as u8);
                    assert!(set_cell(&mut ds, layer, x, y, v));
                }
            }
        }
    }
    Document::new(ds)
}

fn painted_value(layer: LayerRef, main: u8, sub: u8) -> CellValue {
    match layer {
        LayerRef::Wall(_) => common::wall(main, sub, u32::from(main)),
        _ => cv(main, sub),
    }
}

fn set_cell(ds: &mut Ds1, layer: LayerRef, x: u32, y: u32, v: CellValue) -> bool {
    use d2_formats::ds1::Cell;
    match layer {
        LayerRef::Floor(i) => ds.floors[usize::from(i)].set(x, y, Cell(v.cell)),
        LayerRef::Wall(i) => {
            let w = &mut ds.walls[usize::from(i)];
            w.tiles.set(x, y, Cell(v.cell)) && w.orientations.set(x, y, v.orientation)
        }
        LayerRef::Shadow => ds.shadow.set(x, y, Cell(v.cell)),
        LayerRef::Tag => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_map_op_undoes_to_the_original(seed in any::<u64>(), a in 0u32..10, b in 0u32..10, c in 0u32..10, d in 0u32..10) {
        let mut doc = random_doc(seed);
        let original = doc.ds1().clone();
        let layers = [LayerRef::Floor(0), LayerRef::Floor(1), LayerRef::Wall(0), LayerRef::Wall(1), LayerRef::Shadow];
        let mut rng = Rng::new(seed ^ 1);
        let mix = [Brush { main: 1, sub: 1, orientation: 2 }, Brush { main: 2, sub: 0, orientation: 3 }];
        let rect = CellRect::new((a, b), (c, d));
        for layer in layers {
            doc.paint_rect(layer, rect, &mix, &mut rng).unwrap();
            doc.erase_rect(layer, CellRect::new((c, d), (a, b))).unwrap();
            doc.flood_fill(layer, a, b, &mix, None, &mut rng).unwrap();
            doc.flood_fill(layer, c, d, &[], None, &mut rng).unwrap();
            let from = TileKey { orientation: if let LayerRef::Wall(_) = layer { 2 } else if layer == LayerRef::Shadow { 13 } else { 0 }, main: 1, sub: 1 };
            doc.replace_tiles(&[layer], from, floor_key(3, 3), None).unwrap();
        }
        let choices = [
            FloorChoice { main: 1, sub: 0, rarity: 2, walk: 0 },
            FloorChoice { main: 1, sub: 1, rarity: 1, walk: 0 },
        ];
        let opts = RerollOptions { seed, ..RerollOptions::default() };
        doc.reroll_floors(0, None, &choices, &opts, &|_, _| None).unwrap();
        let mut moves = BTreeMap::new();
        moves.insert(floor_key(1, 0), floor_key(4, 4));
        doc.reassign_tiles(&moves).unwrap();
        doc.add_group(group(a as i32, b as i32, 1 + c as i32, 1 + d as i32)).unwrap();
        doc.move_group(0, 1, 1).unwrap();
        doc.resize_group(0, 3, 3).unwrap();
        doc.add_group(group(0, 0, 1, 1)).unwrap();
        doc.delete_group(0).unwrap();
        doc.resize_edges(a as i32 % 3, b as i32 % 3, -(c as i32 % 2), 2).ok();
        assert_roundtrips(doc.ds1());
        while doc.undo() {}
        prop_assert_eq!(doc.ds1(), &original);
        while doc.redo() {}
        assert_roundtrips(doc.ds1());
    }

    #[test]
    fn flood_region_cells_share_one_key_and_are_connected(seed in any::<u64>(), x in 0u32..10, y in 0u32..10) {
        let doc = random_doc(seed);
        let layer = LayerRef::Floor(0);
        let region = doc.flood_region(layer, x, y, None);
        if x >= doc.ds1().width || y >= doc.ds1().height {
            prop_assert!(region.is_empty());
        } else {
            let key = |x, y| capy_edit::tile_key(layer, layer.get(doc.ds1(), x, y).unwrap());
            let k = key(x, y);
            prop_assert!(region.contains(&(x, y)));
            for &(cx, cy) in &region {
                prop_assert_eq!(key(cx, cy), k);
            }
            // maximal: no same-key edge neighbour is left out
            for &(cx, cy) in &region {
                for (nx, ny) in [(cx + 1, cy), (cx.wrapping_sub(1), cy), (cx, cy + 1), (cx, cy.wrapping_sub(1))] {
                    if nx < doc.ds1().width && ny < doc.ds1().height && key(nx, ny) == k {
                        prop_assert!(region.contains(&(nx, ny)));
                    }
                }
            }
        }
    }
}

// ----- corpus -------------------------------------------------------------------

fn corpus_root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CAPY_PRIVATE_DIR") {
        let p = PathBuf::from(p).join("extracted");
        if p.is_dir() {
            return Some(p);
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|a| a.join("testdata").join("private").join("extracted"))
        .find(|p| p.is_dir())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ds1"))
        {
            out.push(path);
        }
    }
}

#[test]
fn corpus_roundtrips_after_group_and_map_edits_and_undo() {
    let Some(root) = corpus_root() else {
        eprintln!("skipped: no private corpus on disk");
        return;
    };
    let mut files = Vec::new();
    collect(&root, &mut files);
    let (mut checked, mut grouped_maps) = (0, 0);
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let Ok(ds1) = Ds1::parse(&bytes) else {
            continue;
        };
        if ds1.to_bytes().ok().as_deref() != Some(bytes.as_slice()) {
            continue;
        }
        let name = path.display();
        let original = ds1.clone();
        let mut d = Document::new(ds1);
        let had_groups = !d.ds1().groups.is_empty();
        let (w, h) = (d.ds1().width, d.ds1().height);

        // group edits where the file can hold groups
        let mut edited = false;
        if d.ds1().version >= 12 && matches!(d.ds1().tag_type, 1 | 2) {
            d.add_group(group(0, 0, 1, 1)).unwrap();
            let last = d.ds1().groups.len() - 1;
            d.move_group(last, 1, 1).unwrap();
            d.resize_group(last, 2, 2).unwrap();
            if had_groups {
                d.delete_group(0).unwrap();
            }
            let out = d.ds1().to_bytes().unwrap();
            // a truncated file parses with `missing` padding that the writer drops
            let mut back = Ds1::parse(&out).unwrap();
            back.preserved.missing = d.ds1().preserved.missing;
            assert_eq!(&back, d.ds1(), "{name}: edited groups");
            edited = true;
            grouped_maps += 1;
        }
        // tile ops
        let f = if d.ds1().floors.is_empty() {
            LayerRef::Shadow
        } else {
            LayerRef::Floor(0)
        };
        let mut rng = Rng::new(5);
        d.paint_rect(
            f,
            CellRect::new((0, 0), (w, h)),
            &[Brush::floor(1, 1), Brush::floor(2, 2)],
            &mut rng,
        )
        .unwrap();
        d.flood_fill(f, 0, 0, &[Brush::floor(3, 3)], None, &mut rng)
            .unwrap();
        d.replace_tiles(&[f], floor_key(3, 3), floor_key(4, 4), None)
            .unwrap();
        if !d.ds1().floors.is_empty() {
            d.reroll_floors(
                0,
                None,
                &[
                    FloorChoice {
                        main: 1,
                        sub: 1,
                        rarity: 1,
                        walk: 0,
                    },
                    FloorChoice {
                        main: 2,
                        sub: 2,
                        rarity: 1,
                        walk: 0,
                    },
                ],
                &RerollOptions::default(),
                &|_, _| None,
            )
            .unwrap();
        }
        d.resize_edges(1, 1, 1, 1).unwrap();
        d.ds1().to_bytes().unwrap();
        while d.undo() {}
        assert_eq!(d.ds1(), &original, "{name}: undo");
        assert_eq!(
            d.ds1().to_bytes().unwrap(),
            bytes,
            "{name}: bytes after undo"
        );
        let _ = edited;
        checked += 1;
    }
    eprintln!("checked {checked} maps ({grouped_maps} with group edits)");
}
