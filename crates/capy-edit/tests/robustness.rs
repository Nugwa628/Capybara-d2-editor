//! Robustness: clipboards from the OS clipboard (any text, damaged JSON,
//! inconsistent sizes), documents from weird but accepted DS1 files, and
//! extreme UI coordinates must give errors or no-ops, never a panic.
//!
//! Knobs: `PROPTEST_CASES` (proptest cases; the defaults below apply when it
//! is unset) and `CAPY_FUZZ_ITERS` (iterations of the private-corpus
//! mutation loop, default 40). The corpus is `CAPY_PRIVATE_DIR` or the
//! nearest `testdata/private`; that test prints `skipped` without it.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use capy_edit::{
    CellRect, CellValue, Clipboard, Document, LayerRef, MoveMode, PasteMode, SelectOp, Selection,
    diff,
};
use d2_formats::Grid;
use d2_formats::ds1::{Cell, Ds1, Group, NpcPath, Object, PathPoint, WallLayer};
use proptest::prelude::*;
use proptest::test_runner::Config as ProptestConfig;
use serde_json::Value;

// ----- knobs -------------------------------------------------------------------

/// `PROPTEST_CASES` when set, else `default` (so the env var always wins).
fn config(default: u32) -> ProptestConfig {
    let mut c = ProptestConfig::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        c.cases = default;
    }
    c.failure_persistence = None;
    c
}

fn iters() -> u64 {
    let n = std::env::var("CAPY_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    println!("CAPY_FUZZ_ITERS = {n}");
    n
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0 | 1;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

// ----- synthetic maps --------------------------------------------------------------

fn tile(main: u8, sub: u8) -> u32 {
    (u32::from(main) << 20) | (u32::from(sub) << 8) | 1
}

fn filled(w: u32, h: u32, seed: u32) -> Grid<Cell> {
    let mut g = Grid::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if !(x + y + seed).is_multiple_of(3) {
                g.set(x, y, Cell(tile((x % 7) as u8, (y % 5) as u8)));
            }
        }
    }
    g
}

/// A valid model of `version` with every layer kind the version allows,
/// objects, groups and paths where they exist.
fn synthetic(version: i32, w: u32, h: u32, walls: usize, floors: usize) -> Ds1 {
    let mut ds = Ds1::new(w, h, if version >= 8 { 2 } else { 0 });
    ds.version = version;
    ds.walls.clear();
    ds.floors.clear();
    let walls = if version >= 4 { walls } else { 1 };
    let floors = if version >= 16 { floors } else { 1 };
    for i in 0..walls {
        let mut o = Grid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                o.set(x, y, (x + y + i as u32) % 20);
            }
        }
        ds.walls.push(WallLayer {
            tiles: filled(w, h, i as u32),
            orientations: o,
        });
    }
    for i in 0..floors {
        ds.floors.push(filled(w, h, 10 + i as u32));
    }
    ds.shadow = filled(w, h, 7);
    if version < 4 {
        ds.tag = Some(Grid::new(w, h));
    } else if version >= 10 {
        ds.tag_type = 1;
        let mut t = Grid::new(w, h);
        t.set(0, 0, 3);
        ds.tag = Some(t);
    }
    if version >= 3 {
        ds.files
            .push(b"data\\global\\tiles\\act1\\town\\floor.dt1".to_vec());
    }
    if version >= 2 {
        let flags = if version >= 6 { 1 } else { 0 };
        ds.objects.push(Object {
            kind: 1,
            id: 5,
            x: 2,
            y: 2,
            flags,
        });
        ds.objects.push(Object {
            kind: 2,
            id: 7,
            x: 7,
            y: 3,
            flags: 0,
        });
        ds.objects.push(Object {
            kind: 2,
            id: 8,
            x: 7,
            y: 3,
            flags: 0,
        });
    }
    if version >= 12 {
        ds.groups.push(Group {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            unknown: 0,
        });
    }
    if version >= 14 {
        ds.npc_paths.push(NpcPath {
            x: 2,
            y: 2,
            points: vec![
                PathPoint {
                    x: 3,
                    y: 3,
                    action: 1,
                },
                PathPoint {
                    x: 9,
                    y: 1,
                    action: 1,
                },
            ],
        });
    }
    ds
}

/// Valid DS1 files of many versions and shapes (1x1, all layers, ...).
fn seed_files() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (version, w, h, walls, floors) in [
        (18, 6, 5, 4, 2),
        (18, 1, 1, 1, 1),
        (18, 1, 1, 4, 2),
        (16, 3, 7, 2, 2),
        (15, 4, 4, 1, 1),
        (14, 4, 4, 2, 1),
        (12, 3, 3, 1, 1),
        (10, 2, 5, 3, 1),
        (7, 3, 3, 2, 1),
        (6, 2, 2, 1, 1),
        (4, 2, 2, 4, 1),
        (3, 3, 3, 1, 1),
        (2, 2, 2, 1, 1),
        (1, 2, 2, 1, 1),
    ] {
        let ds = synthetic(version, w, h, walls, floors);
        if let Ok(bytes) = ds.to_bytes() {
            out.push(bytes);
        }
    }
    assert!(out.len() >= 10, "most synthetic maps must be writable");
    out
}

fn mutate_bytes(rng: &mut Rng, data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(6) {
            0 | 1 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] ^= 1 << rng.below(8);
            }
            2 if !out.is_empty() => {
                // Header dwords (version, size, act, counts) are the interesting part.
                let i = rng.below(out.len().min(64)) & !3;
                let v: u32 = [0, 1, u32::MAX, 0x7FFF_FFFF, 0x8000_0000, 4, 2, 255][rng.below(8)];
                for (k, b) in v.to_le_bytes().into_iter().enumerate() {
                    if let Some(slot) = out.get_mut(i + k) {
                        *slot = b;
                    }
                }
            }
            3 => {
                let n = rng.below(out.len() + 1);
                out.truncate(n);
            }
            4 => {
                let i = rng.below(out.len() + 1);
                let n = 1 + rng.below(12);
                let extra: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                out.splice(i..i, extra);
            }
            _ => {
                let i = rng.below(out.len() + 1);
                out.extend_from_slice(&data[..i.min(data.len())]);
            }
        }
    }
    out
}

// ----- the edit battery --------------------------------------------------------------

fn all_layers(ds: &Ds1) -> Vec<LayerRef> {
    let mut v = LayerRef::all_in(ds);
    v.extend([
        LayerRef::Wall(3),
        LayerRef::Wall(4),
        LayerRef::Wall(255),
        LayerRef::Floor(1),
        LayerRef::Floor(2),
        LayerRef::Floor(255),
        LayerRef::Tag,
    ]);
    v
}

const EXTREME_U32: [u32; 5] = [0, 1, 1023, u32::MAX - 1, u32::MAX];
const EXTREME_I32: [i32; 7] = [0, 1, -1, 4096, -4096, i32::MAX, i32::MIN];

/// Runs every editing entry point with ordinary and extreme arguments. Only
/// panics fail; errors and no-ops are expected.
fn battery(ds: Ds1, knob: u64) {
    let original = ds.clone();
    let mut doc = Document::new(ds);
    doc.set_clip_context(Some("data\\x.ds1".into()), Some(u32::MAX));
    let w = original.width;
    let h = original.height;
    let layers = all_layers(&original);
    let value = CellValue {
        cell: tile(3, 4) | (1 << 31),
        orientation: 0xFFFF_FFFF,
    };

    // Selections from UI rectangles and polygons.
    let mut sel = Selection::new();
    for &a in &EXTREME_U32 {
        for &b in &EXTREME_U32 {
            let mut s = Selection::new();
            s.box_select(
                doc.ds1(),
                CellRect::new((a, b), (b, a)),
                &layers,
                true,
                SelectOp::Add,
            );
            let _ = s.bounds();
        }
    }
    sel.box_select(
        doc.ds1(),
        CellRect::new((0, 0), (u32::MAX, u32::MAX)),
        &layers,
        knob.is_multiple_of(2),
        SelectOp::Replace,
    );
    for polygon in [
        vec![(f32::NAN, 0.0), (1.0, 1.0), (0.0, 2.0)],
        vec![(f32::INFINITY, 0.0), (1.0, 1.0), (0.0, 2.0)],
        vec![(-1e30, -1e30), (1e30, -1e30), (1e30, 1e30), (-1e30, 1e30)],
        vec![(0.0, 0.0), (1e9, 0.0)],
        vec![(-5.0, -5.0), (-1.0, -5.0), (-3.0, -1.0)],
        vec![(0.0, 0.0), (3.5, 0.0), (3.5, 2.5), (0.0, 2.5)],
    ] {
        let mut s = Selection::new();
        s.lasso(doc.ds1(), &polygon, &layers, SelectOp::Replace);
        s.lasso(doc.ds1(), &polygon, &layers, SelectOp::Intersect);
    }
    let mut s = Selection::new();
    s.by_tile_id(doc.ds1(), 3, 4, Some(u32::MAX), &layers, SelectOp::Add);
    s.by_tile_id(doc.ds1(), 255, 255, None, &layers, SelectOp::Subtract);
    s.identical(
        doc.ds1(),
        (LayerRef::Floor(0), 0, 0),
        &layers,
        SelectOp::Add,
    );
    s.identical(
        doc.ds1(),
        (LayerRef::Wall(9), u32::MAX, 0),
        &layers,
        SelectOp::Add,
    );
    sel.insert(LayerRef::Wall(200), u32::MAX, u32::MAX);
    sel.insert(LayerRef::Tag, w, h);

    // Cell edits with coordinates at and past the edges.
    for &layer in &layers {
        let mut cells = vec![(0, 0, value)];
        for &x in &EXTREME_U32 {
            cells.push((x, h.saturating_sub(1), value));
        }
        let _ = doc.paint(layer, &cells);
        let _ = doc.paint(layer, &[(w.saturating_sub(1), h.saturating_sub(1), value)]);
    }
    doc.begin_group("stroke");
    doc.begin_group("inner");
    let _ = doc.paint(LayerRef::Floor(0), &[(0, 0, CellValue::EMPTY)]);
    doc.end_group();
    let _ = doc.set_cell_flags(&sel, Some(true), Some(false));
    doc.end_group();
    doc.end_group();

    // Clipboard round trip at extreme positions.
    let clip = doc.copy(&sel);
    let clip = Clipboard::from_json(&clip.to_json()).unwrap_or(clip);
    for &x in &EXTREME_I32 {
        for mode in [
            PasteMode::Overwrite,
            PasteMode::SkipOccupied,
            PasteMode::OnlyIntoEmptyLayers,
        ] {
            let _ = doc.preview_paste(&clip, (x, -x.saturating_add(1)), mode);
            let _ = doc.paste(&clip, (x, x), mode);
        }
    }
    let _ = doc.cut(&sel);
    let _ = doc.erase(&sel);

    // Layer operations, including impossible indices.
    for &from in &layers {
        for to in [
            LayerRef::Wall(3),
            LayerRef::Floor(1),
            LayerRef::Wall(255),
            LayerRef::Tag,
        ] {
            let _ = doc.move_to_layer(&sel, from, to, MoveMode::SkipOccupied);
            let _ = doc.move_to_layer(&sel, from, to, MoveMode::Overwrite);
        }
    }
    for &layer in &layers {
        let _ = doc.add_layer(layer);
    }
    for &layer in layers.iter().rev() {
        let _ = doc.remove_layer(layer);
    }

    // Objects and NPC paths with extreme indices and coordinates.
    for &v in &EXTREME_I32 {
        let o = Object {
            kind: v,
            id: v,
            x: v,
            y: v.wrapping_neg(),
            flags: v,
        };
        let _ = doc.add_object(o);
        let _ = doc.set_object(0, o);
        let _ = doc.set_object(usize::MAX, o);
        let _ = doc.move_objects(&[0, 1, usize::MAX], v, v);
        let _ = doc.move_objects(&[0, 1], v, -v.saturating_add(1));
        let p = PathPoint {
            x: v,
            y: v,
            action: v,
        };
        for at in [None, Some(0), Some(1), Some(usize::MAX)] {
            let _ = doc.add_path_point(0, p, at);
            let _ = doc.add_path_point(usize::MAX, p, at);
        }
        let _ = doc.move_path_point(0, 0, v, v);
        let _ = doc.move_path_point(0, usize::MAX, v, v);
        let _ = doc.set_path_action(0, 1, v);
        let _ = doc.delete_path_points(0, &[usize::MAX, 0, 0]);
        let _ = doc.path_of(usize::MAX);
    }
    let _ = doc.delete_objects(&[usize::MAX]);
    let _ = doc.delete_objects(&[0, 0, 1]);

    // Size and version.
    for (nw, nh) in [
        (0, 1),
        (1, 0),
        (u32::MAX, u32::MAX),
        (u32::MAX, 1),
        (1 << 20, 2),
        (1, 1),
        (w.saturating_add(3).min(64), h.saturating_add(1).min(64)),
    ] {
        let _ = doc.resize(nw, nh);
    }
    for v in [
        i32::MIN,
        -1,
        0,
        original.version,
        original.version.saturating_add(1),
        13,
        18,
        19,
        i32::MAX,
    ] {
        let _ = doc.upgrade_version(v);
    }

    let _ = diff(&original, doc.ds1());
    let _ = diff(doc.ds1(), &original);
    let _ = doc.is_dirty();
    let _ = doc.ds1().to_bytes();

    // Undo everything, redo everything, undo again: no step may panic.
    let mut guard = 0;
    while doc.undo() && guard < 10_000 {
        guard += 1;
    }
    while doc.redo() && guard < 20_000 {
        guard += 1;
    }
    while doc.undo() && guard < 30_000 {
        guard += 1;
    }
    doc.mark_saved();
}

// ----- clipboard JSON mutations ----------------------------------------------------

fn nasty_number(rng: &mut Rng) -> Value {
    match rng.below(12) {
        0 => Value::from(u64::MAX),
        1 => Value::from(u32::MAX),
        2 => Value::from(i64::MIN),
        3 => Value::from(i32::MIN),
        4 => Value::from(i32::MAX),
        5 => Value::from(-1),
        6 => Value::from(0),
        7 => Value::from(255),
        8 => Value::from(256),
        9 => serde_json::json!(1.5e300),
        10 => serde_json::json!(-0.5),
        _ => Value::from(rng.next() % 100),
    }
}

fn mutate_json(rng: &mut Rng, v: &mut Value, depth: u32) {
    match v {
        Value::Object(map) if !map.is_empty() => {
            let keys: Vec<String> = map.keys().cloned().collect();
            let key = keys[rng.below(keys.len())].clone();
            match rng.below(6) {
                0 => {
                    map.remove(&key);
                }
                1 => {
                    map.insert(key, nasty_number(rng));
                }
                2 => {
                    map.insert(key, Value::Null);
                }
                3 => {
                    let layer = [
                        serde_json::json!({"Wall": 255}),
                        serde_json::json!({"Floor": 2}),
                        serde_json::json!("Tag"),
                        serde_json::json!("Shadow"),
                        serde_json::json!({"Wall": 3}),
                    ][rng.below(5)]
                    .clone();
                    map.insert(key, layer);
                }
                _ if depth < 8 => {
                    if let Some(child) = map.get_mut(&key) {
                        mutate_json(rng, child, depth + 1);
                    }
                }
                _ => {}
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let i = rng.below(items.len());
            match rng.below(5) {
                0 => {
                    let dup = items[i].clone();
                    for _ in 0..1 + rng.below(3) {
                        items.push(dup.clone());
                    }
                }
                1 => {
                    items.remove(i);
                }
                2 => items.truncate(i),
                _ if depth < 8 => mutate_json(rng, &mut items[i], depth + 1),
                _ => {}
            }
        }
        Value::Number(_) => *v = nasty_number(rng),
        _ => *v = nasty_number(rng),
    }
}

fn paste_everywhere(clip: &Clipboard, knob: u64) {
    for (version, w, h) in [(18, 6, 5), (3, 3, 3), (1, 1, 1), (14, 2, 2)] {
        let mut doc = Document::new(synthetic(version, w, h, 2, 2));
        let before = doc.ds1().clone();
        for at in [
            (0, 0),
            (-1, -1),
            (2, 1),
            (i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            (i32::MIN, 0),
            ((knob % 7) as i32 - 3, (knob % 5) as i32 - 2),
        ] {
            for mode in [
                PasteMode::Overwrite,
                PasteMode::SkipOccupied,
                PasteMode::OnlyIntoEmptyLayers,
            ] {
                let preview = doc.preview_paste(clip, at, mode);
                let r = doc.paste(clip, at, mode);
                if let Ok(done) = &r {
                    assert_eq!(done, &preview, "paste must do what the preview said");
                }
            }
        }
        while doc.undo() {}
        assert_eq!(doc.ds1(), &before, "undoing every paste restores the map");
    }
}

fn clip_from(seed: u64) -> Clipboard {
    let doc = Document::new(synthetic(18, 6, 5, 4, 2));
    let mut sel = Selection::new();
    let x0 = (seed % 4) as u32;
    let y0 = ((seed >> 8) % 3) as u32;
    sel.box_select(
        doc.ds1(),
        CellRect::new((x0, y0), (x0 + 2, y0 + 2)),
        &LayerRef::all_in(doc.ds1()),
        seed.is_multiple_of(3),
        SelectOp::Replace,
    );
    doc.copy(&sel)
}

proptest! {
    #![proptest_config(config(256))]

    /// The OS clipboard can hold any text; whatever parses must paste safely.
    #[test]
    fn clipboard_from_any_text(text in "\\PC{0,200}") {
        if let Some(clip) = Clipboard::from_json(&text) {
            paste_everywhere(&clip, text.len() as u64);
        }
    }

    /// Byte-level damage to a real clipboard's JSON.
    #[test]
    fn clipboard_from_damaged_json_bytes(seed in any::<u64>()) {
        let mut rng = Rng(seed);
        let json = clip_from(seed).to_json();
        let bad = mutate_bytes(&mut rng, json.as_bytes());
        if let Ok(text) = std::str::from_utf8(&bad)
            && let Some(clip) = Clipboard::from_json(text)
        {
            paste_everywhere(&clip, seed);
        }
    }

    /// Structural damage: huge or negative numbers, dropped fields,
    /// impossible layers, width/height that disagree with the cells.
    #[test]
    fn clipboard_from_damaged_json_values(seed in any::<u64>()) {
        let mut rng = Rng(seed);
        let mut v: Value = serde_json::from_str(&clip_from(seed).to_json()).unwrap();
        for _ in 0..1 + rng.below(4) {
            mutate_json(&mut rng, &mut v, 0);
        }
        if let Some(clip) = Clipboard::from_json(&v.to_string()) {
            paste_everywhere(&clip, seed);
        }
    }

    /// Hand-built clipboards with arbitrary field values (what a JSON
    /// clipboard from another tool could say once it parses).
    #[test]
    fn arbitrary_clipboards_paste_safely(
        width in any::<u32>(),
        height in any::<u32>(),
        cells in proptest::collection::vec(
            (0u8..4, any::<u8>(), any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>()),
            0..24,
        ),
        objects in proptest::collection::vec(
            (any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>()),
            0..8,
        ),
        act in any::<i32>(),
        lvl in proptest::option::of(any::<u32>()),
        version in any::<i32>(),
    ) {
        let small = |v: u32| if v.is_multiple_of(2) { v % 8 } else { v };
        let clip = Clipboard {
            width,
            height,
            cells: cells
                .into_iter()
                .map(|(k, i, x, y, cell, orientation)| {
                    let layer = match k {
                        0 => LayerRef::Floor(i),
                        1 => LayerRef::Wall(i),
                        2 => LayerRef::Shadow,
                        _ => LayerRef::Tag,
                    };
                    (layer, small(x), small(y), CellValue { cell, orientation })
                })
                .collect(),
            objects: objects
                .into_iter()
                .map(|(kind, id, x, y, flags)| Object {
                    kind,
                    id,
                    x: if x % 2 == 0 { x % 40 } else { x },
                    y: if y % 2 == 0 { y % 40 } else { y },
                    flags,
                })
                .collect(),
            source: capy_edit::ClipSource {
                game_path: None,
                act,
                lvl_type: lvl,
                version,
            },
        };
        let round = Clipboard::from_json(&clip.to_json());
        prop_assert_eq!(round.as_ref(), Some(&clip));
        paste_everywhere(&clip, width as u64);
    }
}

proptest! {
    #![proptest_config(config(64))]

    /// Every edit entry point with extreme arguments on valid maps of many
    /// versions and shapes (including 1x1 and every layer at its limit).
    #[test]
    fn extreme_arguments_on_synthetic_maps(which in 0usize..14, knob in any::<u64>()) {
        let files = seed_files();
        let bytes = &files[which % files.len()];
        battery(Ds1::parse(bytes).unwrap(), knob);
    }

    /// Damaged DS1 files that the parser still accepts become documents;
    /// editing them must not panic.
    #[test]
    fn edits_on_weird_but_accepted_maps(which in 0usize..14, seed in any::<u64>()) {
        let files = seed_files();
        let mut rng = Rng(seed);
        let bad = mutate_bytes(&mut rng, &files[which % files.len()]);
        if let Ok(ds) = Ds1::parse(&bad) {
            // Keep the battery's whole-map loops cheap on absurd sizes.
            if u64::from(ds.width) * u64::from(ds.height) <= 1 << 16 {
                battery(ds, seed);
            }
        }
    }
}

/// A clipboard from elsewhere can hold any number of objects; the anchor
/// check of a paste must not be quadratic in the number of objects (it was:
/// a large pasted object list froze the editor).
#[test]
fn pasting_many_objects_stays_fast() {
    let mut doc = Document::new(synthetic(18, 200, 200, 1, 1));
    let objects: Vec<Object> = (0..20_000)
        .map(|i| Object {
            kind: 1,
            id: i,
            x: i % 1000,
            y: i / 1000,
            flags: 0,
        })
        .collect();
    let clip = Clipboard {
        width: 1,
        height: 1,
        cells: Vec::new(),
        objects,
        source: capy_edit::ClipSource {
            game_path: None,
            act: 2,
            lvl_type: None,
            version: 18,
        },
    };
    let started = std::time::Instant::now();
    let preview = doc.preview_paste(&clip, (0, 0), PasteMode::Overwrite);
    // The synthetic map has a path on (2, 2) with an object: one conflict.
    assert_eq!(preview.warnings.len(), 1, "{:?}", preview.warnings);
    assert!(doc.paste(&clip, (0, 0), PasteMode::Overwrite).is_err());
    println!("preview + paste of 20,000 objects: {:?}", started.elapsed());
}

/// Models no file produces but `Document::new` accepts: zero-sized maps,
/// a tag layer on a high version without a tag type, ...
#[test]
fn degenerate_models() {
    for (w, h) in [(0, 0), (0, 3), (3, 0), (1, 1)] {
        battery(Ds1::new(w, h, 0), 1);
        let mut ds = Ds1::new(w, h, 4);
        ds.tag = Some(Grid::new(w, h));
        ds.version = 18;
        ds.tag_type = 0;
        battery(ds, 2);
        let mut ds = synthetic(18, w.max(1), h.max(1), 4, 2);
        ds.version = i32::MAX;
        battery(ds, 3);
        let mut ds = synthetic(3, w.max(1), h.max(1), 1, 1);
        ds.version = i32::MIN;
        battery(ds, 4);
    }
}

// ----- private corpus -------------------------------------------------------------

fn private_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CAPY_PRIVATE_DIR") {
        let dir = PathBuf::from(dir);
        return dir.is_dir().then_some(dir);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|a| a.join("testdata").join("private"))
        .find(|p| p.is_dir())
}

fn ds1_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            ds1_files(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ds1")) {
            out.push(p);
        }
    }
}

/// Real maps, damaged; whatever still parses is edited with the battery.
#[test]
fn edits_on_damaged_corpus_maps() {
    let Some(root) = private_root() else {
        println!("skipped: no private corpus (CAPY_PRIVATE_DIR / testdata/private)");
        return;
    };
    let mut files = Vec::new();
    ds1_files(&root.join("extracted"), &mut files);
    // Small maps keep the battery quick; spread the sample over the tree.
    files.retain(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 64 * 1024));
    if files.is_empty() {
        println!("skipped: the corpus has no small DS1 files");
        return;
    }
    let step = (files.len() / 24).max(1);
    let seeds: Vec<Vec<u8>> = files
        .iter()
        .step_by(step)
        .take(24)
        .filter_map(|p| std::fs::read(p).ok())
        .collect();
    let mut rng = Rng(0xC0FF_EE00_DEAD_BEEF);
    let n = iters();
    let mut edited = 0;
    for i in 0..n {
        let seed = &seeds[rng.below(seeds.len())];
        let bytes = if i % 4 == 0 {
            seed.clone()
        } else {
            mutate_bytes(&mut rng, seed)
        };
        if let Ok(ds) = Ds1::parse(&bytes)
            && u64::from(ds.width) * u64::from(ds.height) <= 1 << 16
        {
            battery(ds, i);
            edited += 1;
        }
    }
    println!("{edited} of {n} corpus cases parsed and were edited");
}
