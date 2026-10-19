//! Property tests: random documents, random edit sequences, undo/redo laws.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use capy_edit::{
    CellRect, CellValue, Clipboard, Document, EditError, LayerRef, MoveMode, PasteMode, SelectOp,
    Selection,
};
use common::tile;
use d2_formats::Grid;
use d2_formats::ds1::{Cell, Ds1, Group, NpcPath, Object, PathPoint, WallLayer};
use proptest::prelude::*;

// ----- a tiny deterministic RNG ----------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo) as u64) as i32
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

// ----- random documents -------------------------------------------------------

fn rand_cell(rng: &mut Rng) -> u32 {
    if rng.chance(45) {
        return 0;
    }
    let mut c = tile(rng.below(4) as u8, rng.below(4) as u8);
    if rng.chance(10) {
        c |= 1 << 31;
    }
    if rng.chance(10) {
        c |= 1 << 17;
    }
    c
}

fn rand_grid(rng: &mut Rng, w: u32, h: u32) -> Grid<Cell> {
    let cells = (0..w * h).map(|_| Cell(rand_cell(rng))).collect();
    Grid::from_vec(w, h, cells).unwrap()
}

fn rand_u32_grid(rng: &mut Rng, w: u32, h: u32, max: u64) -> Grid<u32> {
    let cells = (0..w * h)
        .map(|_| {
            if rng.chance(50) {
                0
            } else {
                rng.below(max) as u32
            }
        })
        .collect();
    Grid::from_vec(w, h, cells).unwrap()
}

fn rand_object(rng: &mut Rng, w: u32, h: u32, flags_ok: bool) -> Object {
    Object {
        kind: rng.range(1, 3),
        id: rng.range(0, 200),
        x: rng.range(0, (w * 5) as i32),
        y: rng.range(0, (h * 5) as i32),
        flags: if flags_ok && rng.chance(40) {
            rng.range(1, 9)
        } else {
            0
        },
    }
}

fn gen_ds1(rng: &mut Rng) -> Ds1 {
    let w = 1 + rng.below(7) as u32;
    let h = 1 + rng.below(6) as u32;
    let mut ds = Ds1::new(w, h, rng.range(0, 5));
    let version = [18, 16, 3, 12, 5, 18, 7, 1][rng.below(8) as usize];
    ds.version = version;
    if version < 8 {
        ds.act = 0;
    }
    ds.walls.clear();
    ds.floors.clear();
    let wall_count = if version >= 4 {
        rng.below(5) as usize
    } else {
        1
    };
    let floor_count = if version >= 16 {
        rng.below(3) as usize
    } else {
        1
    };
    for _ in 0..wall_count {
        ds.walls.push(WallLayer {
            tiles: rand_grid(rng, w, h),
            orientations: rand_u32_grid(rng, w, h, 30),
        });
    }
    for _ in 0..floor_count {
        ds.floors.push(rand_grid(rng, w, h));
    }
    ds.shadow = rand_grid(rng, w, h);

    if version < 4 {
        ds.tag = Some(rand_u32_grid(rng, w, h, 300));
    } else if version >= 10 {
        ds.tag_type = rng.below(3) as i32;
        if (1..=2).contains(&ds.tag_type) {
            ds.tag = Some(rand_u32_grid(rng, w, h, 300));
        }
    }
    if version >= 3 && rng.chance(50) {
        ds.files.push(b"data\\global\\tiles\\x.dt1".to_vec());
    }
    if (9..=13).contains(&version) {
        ds.preserved.unknown_v9_13 = [5000, 5000];
    }
    if version >= 2 {
        for _ in 0..rng.below(5) {
            ds.objects.push(rand_object(rng, w, h, version >= 6));
        }
    }
    if version >= 12 && (1..=2).contains(&ds.tag_type) {
        for _ in 0..rng.below(3) {
            ds.groups.push(Group {
                x: rng.range(0, w as i32),
                y: rng.range(0, h as i32),
                width: rng.range(1, 4),
                height: rng.range(1, 4),
                unknown: if version >= 13 { rng.range(0, 3) } else { 0 },
            });
        }
        if version >= 18 {
            ds.preserved.pre_group_dword = rng.range(0, 3);
        }
    }
    if version >= 14 {
        let anchors: Vec<Object> = ds
            .objects
            .iter()
            .copied()
            .filter(|_| rng.chance(70))
            .collect();
        for o in anchors {
            ds.npc_paths.push(NpcPath {
                x: o.x,
                y: o.y,
                points: (0..if rng.chance(10) { 0 } else { 1 + rng.below(3) })
                    .map(|_| PathPoint {
                        x: rng.range(0, 20),
                        y: rng.range(0, 20),
                        action: if version >= 15 { rng.range(1, 4) } else { 1 },
                    })
                    .collect(),
            });
        }
    }
    ds
}

// ----- random edits -------------------------------------------------------------

fn rand_layer_any(rng: &mut Rng) -> LayerRef {
    match rng.below(10) {
        0..=2 => LayerRef::Wall(rng.below(5) as u8),
        3..=5 => LayerRef::Floor(rng.below(3) as u8),
        6 | 7 => LayerRef::Shadow,
        _ => LayerRef::Tag,
    }
}

fn rand_existing(rng: &mut Rng, ds: &Ds1) -> LayerRef {
    let all = LayerRef::all_in(ds);
    all[rng.below(all.len() as u64) as usize]
}

fn rand_value(rng: &mut Rng, layer: LayerRef) -> CellValue {
    match layer {
        LayerRef::Tag => CellValue {
            cell: if rng.chance(30) {
                0
            } else {
                rng.below(300) as u32
            },
            orientation: 0,
        },
        _ => CellValue {
            cell: rand_cell(rng),
            orientation: if rng.chance(60) {
                rng.below(30) as u32
            } else {
                0
            },
        },
    }
}

fn rand_selection(rng: &mut Rng, ds: &Ds1) -> Selection {
    let mut layers: Vec<LayerRef> = LayerRef::all_in(ds)
        .into_iter()
        .filter(|_| rng.chance(60))
        .collect();
    if rng.chance(15) {
        layers.push(rand_layer_any(rng));
    }
    let a = (
        rng.below(u64::from(ds.width) + 1) as u32,
        rng.below(u64::from(ds.height) + 1) as u32,
    );
    let b = (
        rng.below(u64::from(ds.width) + 1) as u32,
        rng.below(u64::from(ds.height) + 1) as u32,
    );
    let mut sel = Selection::new();
    sel.box_select(
        ds,
        CellRect::new(a, b),
        &layers,
        rng.chance(20),
        SelectOp::Replace,
    );
    match rng.below(6) {
        0 => {
            let poly: Vec<(f32, f32)> = (0..rng.range(2, 6))
                .map(|_| (rng.range(-1, 9) as f32, rng.range(-1, 8) as f32))
                .collect();
            let op =
                [SelectOp::Add, SelectOp::Subtract, SelectOp::Intersect][rng.below(3) as usize];
            sel.lasso(ds, &poly, &layers, op);
        }
        1 => {
            let op = [SelectOp::Add, SelectOp::Subtract, SelectOp::Replace][rng.below(3) as usize];
            sel.by_tile_id(
                ds,
                rng.below(4) as u8,
                rng.below(4) as u8,
                if rng.chance(30) {
                    Some(rng.below(30) as u32)
                } else {
                    None
                },
                &layers,
                op,
            );
        }
        2 => {
            let seed = (
                rand_existing(rng, ds),
                rng.below(u64::from(ds.width)) as u32,
                rng.below(u64::from(ds.height)) as u32,
            );
            sel.identical(ds, seed, &layers, SelectOp::Add);
        }
        _ => {}
    }
    sel
}

/// Edit kinds 0..EDIT_KINDS are edits; the next five codes are history commands.
const EDIT_KINDS: u64 = 25;

/// An object index: mostly valid, sometimes just past the end. With `prefer_path`
/// an object that has an NPC path is favoured.
fn pick_object(rng: &mut Rng, ds: &Ds1, prefer_path: bool) -> usize {
    if ds.objects.is_empty() || rng.chance(6) {
        return ds.objects.len() + rng.below(2) as usize;
    }
    if prefer_path && rng.chance(85) {
        let with_path: Vec<usize> = (0..ds.objects.len())
            .filter(|&i| {
                ds.npc_paths
                    .iter()
                    .any(|p| p.x == ds.objects[i].x && p.y == ds.objects[i].y)
            })
            .collect();
        if !with_path.is_empty() {
            return with_path[rng.below(with_path.len() as u64) as usize];
        }
    }
    rng.below(ds.objects.len() as u64) as usize
}

fn rand_point(rng: &mut Rng) -> PathPoint {
    PathPoint {
        x: rng.range(0, 30),
        y: rng.range(0, 30),
        action: if rng.chance(60) { 1 } else { rng.range(0, 6) },
    }
}

/// Runs one randomly parameterised edit (or history command, which the
/// caller handles). The returned error, if any, is the edit's own.
fn run_edit(doc: &mut Document, kind: u64, seed: u64) -> Result<(), EditError> {
    let mut rng = Rng(seed);
    let rng = &mut rng;
    let ds = doc.ds1().clone();
    match kind {
        0 => {
            let layer = if rng.chance(90) {
                rand_existing(rng, &ds)
            } else {
                rand_layer_any(rng)
            };
            let cells: Vec<(u32, u32, CellValue)> = (0..1 + rng.below(6))
                .map(|_| {
                    let extra = u64::from(rng.chance(5));
                    (
                        rng.below(u64::from(ds.width) + extra) as u32,
                        rng.below(u64::from(ds.height)) as u32,
                        rand_value(rng, layer),
                    )
                })
                .collect();
            doc.paint(layer, &cells)
        }
        1 => doc.erase(&rand_selection(rng, &ds)),
        2 => {
            let pick = |rng: &mut Rng| match rng.below(3) {
                0 => None,
                1 => Some(true),
                _ => Some(false),
            };
            let (h, u) = (pick(rng), pick(rng));
            doc.set_cell_flags(&rand_selection(rng, &ds), h, u)
        }
        3 | 4 => {
            let from = rand_existing(rng, &ds);
            let to = rand_layer_any(rng);
            let mode = if rng.chance(50) {
                MoveMode::Overwrite
            } else {
                MoveMode::SkipOccupied
            };
            let sel = rand_selection(rng, &ds);
            match doc.move_to_layer(&sel, from, to, mode) {
                // every selected target was occupied: reported, nothing recorded
                Ok(report) if report.moved == 0 => {
                    assert!(report.layer_added.is_none());
                    Ok(())
                }
                Ok(report) => {
                    assert_eq!(report.layer_added.is_some(), !to.exists_in(&ds));
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        5 => doc.add_layer(rand_layer_any(rng)),
        6 => doc.remove_layer(if rng.chance(85) {
            rand_existing(rng, &ds)
        } else {
            rand_layer_any(rng)
        }),
        7 => doc.resize(rng.below(9) as u32, rng.below(8) as u32),
        8 => {
            let v = [
                ds.version - 1,
                ds.version,
                4,
                6,
                7,
                10,
                12,
                13,
                14,
                16,
                18,
                19,
            ][rng.below(12) as usize];
            doc.upgrade_version(v)
        }
        9 | 10 => {
            let src_ds = if rng.chance(30) {
                ds.clone()
            } else {
                gen_ds1(rng)
            };
            let mut src = Document::new(src_ds);
            src.set_clip_context(None, Some(rng.below(3) as u32));
            let sel = rand_selection(rng, src.ds1());
            let mut clip = src.copy(&sel);
            if rng.chance(50) {
                clip = Clipboard::from_json(&clip.to_json()).unwrap();
            }
            let at = (rng.range(-3, 9), rng.range(-3, 8));
            let mode = [
                PasteMode::Overwrite,
                PasteMode::SkipOccupied,
                PasteMode::OnlyIntoEmptyLayers,
            ][rng.below(3) as usize];
            let preview = doc.preview_paste(&clip, at, mode);
            assert_eq!(doc.ds1(), &ds, "a preview must not change the document");
            match doc.paste(&clip, at, mode) {
                Ok(p) => {
                    assert_eq!(p, preview);
                    assert!(p.needs_version.is_none());
                    Ok(())
                }
                Err(EditError::NeedsVersion { required, .. }) => {
                    assert_eq!(preview.needs_version, Some(required));
                    Err(EditError::NeedsVersion {
                        current: ds.version,
                        required,
                    })
                }
                Err(e) => Err(e),
            }
        }
        11 => {
            let sel = rand_selection(rng, &ds);
            let expect = doc.copy(&sel);
            let got = doc.cut(&sel)?;
            assert_eq!(got, expect);
            Ok(())
        }
        12 => {
            let flags_ok = rng.chance(50);
            doc.add_object(rand_object(rng, 9, 8, flags_ok)).map(|_| ())
        }
        13 => {
            let idx: Vec<usize> = (0..rng.below(4))
                .map(|_| rng.below(ds.objects.len() as u64 + 1) as usize)
                .collect();
            doc.move_objects(&idx, rng.range(-6, 7), rng.range(-6, 7))
        }
        14 => {
            let idx: Vec<usize> = (0..rng.below(4))
                .map(|_| rng.below(ds.objects.len() as u64 + 1) as usize)
                .collect();
            doc.delete_objects(&idx)
        }
        15 | 16 => {
            // a clip with hostile contents must be harmless
            let mut clip = Document::new(ds.clone()).copy(&rand_selection(rng, &ds));
            clip.cells.push((
                LayerRef::Wall(rng.below(8) as u8),
                rng.below(4) as u32,
                rng.below(4) as u32,
                rand_value(rng, LayerRef::Wall(0)),
            ));
            clip.cells.push((
                LayerRef::Floor(rng.below(4) as u8),
                u32::MAX - 1,
                0,
                rand_value(rng, LayerRef::Wall(0)),
            ));
            doc.paste(
                &clip,
                (rng.range(-2, 5), rng.range(-2, 5)),
                PasteMode::Overwrite,
            )
            .map(|_| ())
        }
        17 => {
            // restore-from-snapshot
            let other = gen_ds1(rng);
            doc.apply_patch(capy_edit::Patch::replace("Restore", ds, other))
        }
        19 => {
            let i = pick_object(rng, &ds, true);
            let old = ds
                .objects
                .get(i)
                .copied()
                .unwrap_or(rand_object(rng, 9, 8, true));
            let new = match rng.below(4) {
                // move it (the path follows)
                0 | 1 => Object {
                    x: old.x + rng.range(-3, 4),
                    y: old.y + rng.range(-3, 4),
                    ..old
                },
                // onto another object's sub-tile
                2 if !ds.objects.is_empty() => {
                    let o = ds.objects[rng.below(ds.objects.len() as u64) as usize];
                    Object {
                        x: o.x,
                        y: o.y,
                        ..old
                    }
                }
                _ => {
                    let flags_ok = rng.chance(50);
                    rand_object(rng, 9, 8, flags_ok)
                }
            };
            doc.set_object(i, new)
        }
        20 | 24 => {
            let i = pick_object(rng, &ds, true);
            let at = match rng.below(4) {
                0 => None,
                1 => Some(0),
                2 => Some(rng.below(4) as usize),
                _ => Some(rng.below(9) as usize),
            };
            let point = rand_point(rng);
            let result = doc.add_path_point(i, point, at);
            if result.is_ok() {
                // the point landed where it was asked to, on the path of that object
                let path = doc.path_of(i).unwrap();
                let p = at.unwrap_or(path.points.len() - 1);
                assert_eq!(path.points[p], point);
            }
            result
        }
        21 => {
            let i = pick_object(rng, &ds, true);
            let k = rng.below(4) as usize;
            doc.move_path_point(i, k, rng.range(0, 40), rng.range(0, 40))
        }
        22 => {
            let i = pick_object(rng, &ds, true);
            let k = rng.below(4) as usize;
            doc.set_path_action(i, k, if rng.chance(40) { 1 } else { rng.range(0, 7) })
        }
        23 => {
            let i = pick_object(rng, &ds, true);
            let idx: Vec<usize> = (0..rng.below(4)).map(|_| rng.below(4) as usize).collect();
            doc.delete_path_points(i, &idx)
        }
        _ => doc.upgrade_version(ds.version + 1 + rng.range(0, 3)),
    }
}

// ----- the model check ------------------------------------------------------------

fn roundtrip(ds: &Ds1) -> Result<(), TestCaseError> {
    let bytes = ds.to_bytes();
    prop_assert!(bytes.is_ok(), "not representable: {:?}", bytes.err());
    let back = Ds1::parse(&bytes.unwrap());
    prop_assert!(back.is_ok(), "does not re-parse: {:?}", back.err());
    prop_assert_eq!(&back.unwrap(), ds);
    Ok(())
}

fn run_case(init_seed: u64, ops: &[(u8, u64)]) -> Result<(), TestCaseError> {
    let initial = gen_ds1(&mut Rng(init_seed));
    roundtrip(&initial)?;
    let mut doc = Document::new(initial.clone());
    // `states[d]` is the document at undo depth d; `redo` is the redo stack of states.
    let mut states = vec![initial.clone()];
    let mut redo: Vec<Ds1> = Vec::new();
    let mut saved = initial.clone();
    let mut depth = 0u32;
    let mut group_ops = 0usize;
    prop_assert!(!doc.is_dirty());

    for &(code, seed) in ops {
        let kind = u64::from(code) % (EDIT_KINDS + 5);
        let before = doc.ds1().clone();
        let (revision, can_u, can_r) = (doc.revision(), doc.can_undo(), doc.can_redo());
        let (ul, rl) = (
            doc.undo_label().map(str::to_string),
            doc.redo_label().map(str::to_string),
        );
        match kind {
            25 => {
                doc.begin_group("Group");
                if depth == 0 {
                    group_ops = 0;
                }
                depth += 1;
            }
            26 => {
                doc.end_group();
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 && group_ops > 0 {
                        states.push(doc.ds1().clone());
                    }
                }
            }
            27 | 28 => {
                // undo and redo close an open group first
                if depth > 0 {
                    depth = 0;
                    if group_ops > 0 {
                        states.push(doc.ds1().clone());
                    }
                }
                if kind == 27 {
                    let expect = states.len() > 1;
                    prop_assert_eq!(doc.undo(), expect);
                    if expect {
                        redo.push(states.pop().unwrap());
                    }
                } else {
                    let expect = !redo.is_empty();
                    prop_assert_eq!(doc.redo(), expect);
                    if expect {
                        states.push(redo.pop().unwrap());
                    }
                }
                prop_assert_eq!(doc.ds1(), states.last().unwrap());
            }
            29 => {
                doc.mark_saved();
                saved = doc.ds1().clone();
                prop_assert!(!doc.is_dirty());
            }
            _ => match run_edit(&mut doc, kind % EDIT_KINDS, seed) {
                // a no-op succeeds and leaves no trace
                Ok(()) if doc.revision() == revision => {
                    prop_assert_eq!(doc.ds1(), &before);
                    prop_assert_eq!(doc.can_undo(), can_u);
                    prop_assert_eq!(doc.can_redo(), can_r);
                    prop_assert_eq!(doc.undo_label().map(str::to_string), ul);
                    prop_assert_eq!(doc.redo_label().map(str::to_string), rl);
                }
                Ok(()) => {
                    roundtrip(doc.ds1())?;
                    if depth == 0 {
                        let after = doc.ds1().clone();
                        states.push(after.clone());
                        redo.clear();
                        prop_assert!(doc.can_undo());
                        prop_assert!(!doc.can_redo());
                        // (c) undo then redo is the identity
                        prop_assert!(doc.undo());
                        prop_assert_eq!(doc.ds1(), &before);
                        prop_assert!(doc.redo());
                        prop_assert_eq!(doc.ds1(), &after);
                    } else {
                        if group_ops == 0 {
                            redo.clear();
                        }
                        group_ops += 1;
                    }
                }
                Err(_) => {
                    // a failed edit changes nothing
                    prop_assert_eq!(doc.ds1(), &before);
                    prop_assert_eq!(doc.revision(), revision);
                    prop_assert_eq!(doc.can_undo(), can_u);
                    prop_assert_eq!(doc.can_redo(), can_r);
                    prop_assert_eq!(doc.undo_label().map(str::to_string), ul);
                    prop_assert_eq!(doc.redo_label().map(str::to_string), rl);
                }
            },
        }
        // (d)
        prop_assert_eq!(doc.is_dirty(), *doc.ds1() != saved);
    }
    while depth > 0 {
        doc.end_group();
        depth -= 1;
        if depth == 0 && group_ops > 0 {
            states.push(doc.ds1().clone());
        }
    }
    prop_assert_eq!(doc.ds1(), states.last().unwrap());
    prop_assert_eq!(doc.is_dirty(), *doc.ds1() != saved);

    // go to the far end of the redo stack so the whole history is one line
    while let Some(next) = redo.pop() {
        prop_assert!(doc.redo());
        states.push(next);
        prop_assert_eq!(doc.ds1(), states.last().unwrap());
    }
    prop_assert!(!doc.can_redo());

    // (a) undo-all restores the initial document, step by step
    let mut n = 0;
    while doc.undo() {
        n += 1;
        prop_assert!(n < states.len());
        prop_assert_eq!(doc.ds1(), &states[states.len() - 1 - n]);
        prop_assert_eq!(doc.is_dirty(), *doc.ds1() != saved);
    }
    prop_assert_eq!(n, states.len() - 1);
    prop_assert_eq!(doc.ds1(), &initial);

    // (b) redo-all reproduces the final document, step by step
    let mut n = 0;
    while doc.redo() {
        n += 1;
        prop_assert_eq!(doc.ds1(), &states[n]);
        roundtrip(doc.ds1())?;
    }
    prop_assert_eq!(n, states.len() - 1);
    prop_assert_eq!(doc.ds1(), states.last().unwrap());
    Ok(())
}

/// `CAPY_PROP_CASES` multiplies the default case counts (for long soak runs).
fn cases(default: u32) -> u32 {
    let factor = std::env::var("CAPY_PROP_CASES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);
    default.saturating_mul(factor)
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::with_cases(cases(512)) })]

    #[test]
    fn random_edit_sequences_obey_the_undo_laws(
        seed in any::<u64>(),
        ops in proptest::collection::vec((0u8..30, any::<u64>()), 1..40),
    ) {
        run_case(seed, &ops)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::with_cases(cases(256)) })]

    /// Long sequences, to reach deep undo stacks and many redo branches.
    #[test]
    fn long_sequences(
        seed in any::<u64>(),
        ops in proptest::collection::vec((0u8..30, any::<u64>()), 60..120),
    ) {
        run_case(seed, &ops)?;
    }

    #[test]
    fn generated_documents_are_valid(seed in any::<u64>()) {
        roundtrip(&gen_ds1(&mut Rng(seed)))?;
    }
}

// ----- scale ------------------------------------------------------------------------

#[test]
fn many_undo_steps_on_a_large_map() {
    let mut ds = Ds1::new(100, 100, 0);
    for _ in 0..3 {
        ds.add_wall_layer().unwrap();
    }
    ds.version = 18;
    ds.add_floor_layer().unwrap();
    let original = ds.clone();
    let mut doc = Document::new(ds);
    let mut rng = Rng(7);
    for i in 0..4000u32 {
        let layer = if i % 3 == 0 {
            LayerRef::Wall((i % 4) as u8)
        } else {
            LayerRef::Floor((i % 2) as u8)
        };
        doc.begin_group("stroke");
        for _ in 0..8 {
            let x = rng.below(100) as u32;
            let y = rng.below(100) as u32;
            let v = CellValue {
                cell: tile((i % 60) as u8 + 1, 1),
                orientation: if matches!(layer, LayerRef::Wall(_)) {
                    i % 30
                } else {
                    0
                },
            };
            let _ = doc.paint(layer, &[(x, y, v)]);
        }
        doc.end_group();
    }
    let finished = doc.ds1().clone();
    let mut steps = 0;
    while doc.undo() {
        steps += 1;
    }
    assert!(steps >= 3900, "{steps}");
    assert_eq!(doc.ds1(), &original);
    while doc.redo() {}
    assert_eq!(doc.ds1(), &finished);
}

/// Guards the generator: every kind of edit must succeed often enough that
/// the properties above are not vacuous.
#[test]
fn every_edit_kind_succeeds_sometimes() {
    let mut ok = [0usize; EDIT_KINDS as usize];
    let mut needs_version = 0usize;
    let mut rng = Rng(99);
    for _ in 0..400 {
        let mut doc = Document::new(gen_ds1(&mut rng));
        for _ in 0..30 {
            let kind = rng.below(EDIT_KINDS);
            let revision = doc.revision();
            match run_edit(&mut doc, kind, rng.next()) {
                Ok(()) if doc.revision() > revision => ok[kind as usize] += 1,
                Ok(()) => {}
                Err(EditError::NeedsVersion { .. }) => needs_version += 1,
                Err(_) => {}
            }
        }
    }
    eprintln!("successes per kind: {ok:?}, NeedsVersion: {needs_version}");
    for (kind, n) in ok.iter().enumerate() {
        assert!(*n >= 30, "edit kind {kind} succeeded only {n} times");
    }
    assert!(needs_version >= 30);
}
